//! HTTP handlers — 모두 DB 만 읽는다 (외부 API 직접 호출은 collector / scanner 가 담당).

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{sse::{Event, Sse}, IntoResponse, Json},
};
use chrono::{DateTime, Datelike, Utc};
use futures::stream::{self, Stream};
use rn1_db as db;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::{convert::Infallible, sync::Arc, time::Duration};
use crate::{AppState, ErrorBody};

pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1").fetch_one(&state.pool).await.is_ok();
    Json(serde_json::json!({
        "ok": db_ok,
        "ts": Utc::now(),
    }))
}

#[derive(Deserialize)]
pub struct PortfolioQ {
    /// "tree" 면 event 그룹핑, "flat" 이면 단일 포지션 배열
    #[serde(default = "default_tree")]
    pub view: String,
    /// 정렬 키: "recent" (last_updated desc) | "value" (current_value desc) |
    /// "pnl" (cash_pnl desc) | "end" (earliest_end asc, NULL last)
    #[serde(default = "default_sort")]
    pub sort: String,
    /// 필터: "active" (size>0 only) | "redeemable" | "mergeable" | "all"
    #[serde(default = "default_filter")]
    pub filter: String,
    /// 검색어 (title / event_slug / outcome name 부분 일치, 대소문자 무시)
    #[serde(default)]
    pub q: Option<String>,
    /// 페이지네이션 — events 기준
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}
fn default_tree() -> String { "tree".into() }
fn default_sort() -> String { "recent".into() }
fn default_filter() -> String { "recent".into() }
fn default_page() -> u32 { 1 }
fn default_page_size() -> u32 { 100 }

#[derive(Serialize)]
pub struct PositionEntry {
    pub condition_id: String,
    pub outcome_index: i16,
    pub outcome_name: String,
    pub size: f64,
    pub avg_price: Option<f64>,
    pub current_price: Option<f64>,
    pub current_value: f64,
    pub initial_value: f64,
    pub realized_pnl: f64,
    pub cash_pnl: f64,
    pub total_bought: f64,
    pub mergeable: bool,
    pub redeemable: bool,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub event_slug: Option<String>,
    pub icon: Option<String>,
    pub neg_risk: bool,
    pub end_date: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    // 마지막 관측된 fill — 토스트의 실제 fill 가격 표시용 (current_price 가 시장 mid 라서 inaccurate)
    pub last_fill_price: Option<f64>,
    pub last_fill_ts:    Option<DateTime<Utc>>,
    // 첫 fill ts — 양면 마켓에서 어느 outcome 이 먼저 시작됐는지 표시 (FIRST leg)
    pub first_fill_ts:   Option<DateTime<Utc>>,
    /// Seconds since `current_price` was last refreshed (trader upsert_asset_quote).
    /// Frontend should warn (red badge) if > ~60s. 2026-05-25 incident: cache
    /// silently stayed 24 minutes old, profits shown were on stale prices.
    pub quote_age_s: Option<i64>,
}

#[derive(Serialize)]
pub struct EventNode {
    pub event_slug: String,
    pub sample_title: Option<String>,
    pub sample_icon: Option<String>,
    pub market_count: i64,
    pub leg_count: i64,
    pub total_value: f64,
    pub total_cash_pnl: f64,
    pub total_realized_pnl: f64,
    pub any_redeemable: bool,
    pub any_mergeable: bool,
    pub earliest_end: Option<DateTime<Utc>>,
    pub last_updated: DateTime<Utc>,
    /// Most-recent fill ts across all legs in this event. Distinct from
    /// `last_updated` — the latter advances on every bid/ask quote refresh,
    /// which makes "recent activity" sort fluctuate even when no fills have
    /// happened. Use this for "최근 체결순" (fill-order) sort.
    pub latest_fill_ts: Option<DateTime<Utc>>,
    pub markets: Vec<MarketNode>,
}

#[derive(Serialize)]
pub struct MarketNode {
    pub condition_id: String,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub neg_risk: bool,
    pub end_date: Option<DateTime<Utc>>,
    pub current_value: f64,
    pub cash_pnl: f64,
    pub realized_pnl: f64,
    pub outcomes: Vec<PositionEntry>,
    /// 같은 event 안에서 market family — "moneyline" / "ou-totals" / "spread"
    /// / "first-inning" / "alt-spread" / "other". Polymarket 의 neg_risk 가 false 여도
    /// 동일 event 안의 다중 condition 을 sluggish 패턴으로 묶어 표시.
    pub category: String,
    /// O/U / spread 에서 threshold 값 (예: "8.5", "-1.5") — UI 정렬에 사용
    pub family_param: Option<String>,
}

/// Market 분류 — slug + title pattern 으로 family 추출.
/// 같은 event 안의 markets 를 의미 단위로 묶기 위함.
/// Polymarket 의 neg_risk=false 여도 sluggish 매칭으로 family 식별 (예: MLB 의 다중 O/U).
///
/// 반환: (category, family_param). family_param 은 O/U threshold 또는 spread amount 등.
fn categorize_market(title: Option<&str>, slug: Option<&str>, event_slug: &str)
    -> (String, Option<String>)
{
    let s = slug.unwrap_or("");
    let t = title.unwrap_or("").to_lowercase();
    let strip = s.strip_prefix(event_slug).unwrap_or(s);
    // event_slug 와 동일하거나 -more-markets, -prematch 등 mainline 변형 = moneyline
    if s == event_slug || strip.starts_with("-more-") || strip.starts_with("-prematch") || strip.starts_with("-3way") {
        return ("moneyline".into(), None);
    }
    // O/U totals — slug 에 -total-, -ou-, -over-under- 등
    if strip.starts_with("-total-") || strip.contains("-ou-") || strip.starts_with("-over-under")
       || t.contains("o/u ") || t.contains("over/under")
    {
        // threshold 추출 — "total-8pt5" → "8.5", "total-10pt5" → "10.5"
        let param = strip.rsplit('-').next()
            .map(|x| x.replace("pt", ".").to_string())
            .filter(|p| !p.is_empty());
        return ("ou-totals".into(), param);
    }
    // Spread
    if strip.contains("-spread-") || t.contains("spread:") {
        // "spread-away-1pt5" / "spread-home-1pt5" → "1.5" (away/home 정보는 별도)
        let param = strip.rsplit('-').next()
            .map(|x| x.replace("pt", ".").to_string())
            .filter(|p| !p.is_empty());
        return ("spread".into(), param);
    }
    // First inning / quarter / period
    if strip.contains("-1st-") || strip.contains("-first-") || t.contains("first ")
       || t.contains("1st ") || t.contains(" inning")
    {
        return ("first-inning".into(), None);
    }
    // Alt spread (다른 threshold)
    if strip.contains("-alt-") {
        return ("alt-spread".into(), None);
    }
    ("other".into(), None)
}

pub async fn positions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PortfolioQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // WHERE 절은 filter param 으로 동적 결정.
    // - recent: 최근 24h RN1 trade 가 있던 conditionId 만 (= 폴리마켓의 Positions 탭 동작)
    // - active: size > 0 (광범위, 옛 청산 대기 포함)
    // - redeemable/mergeable: 청산 가능 상태만
    // - all: 모든 행 (size=0 청산 끝난 것까지)
    let where_clause = match q.filter.as_str() {
        "redeemable"  => "redeemable = true",
        "mergeable"   => "mergeable  = true",
        "active"      => "size > 0",
        "all"         => "(size > 0 OR redeemable OR mergeable)",
        // 기본: "recent" — 폴리마켓 Activity 와 매칭되는 마켓만
        _             => "(size > 0 OR redeemable OR mergeable) AND condition_id IN \
                          (SELECT DISTINCT condition_id FROM fills \
                           WHERE is_ours = false AND ts > NOW() - INTERVAL '24 hours')",
    };
    let q_like = q.q.as_ref().map(|s| format!("%{}%", s.to_lowercase()));
    let has_q = q_like.is_some();
    let sql = format!(r#"
        SELECT p.condition_id, p.outcome_index, p.outcome_name, p.size, p.avg_price,
            p.current_price, p.current_value, p.initial_value, p.realized_pnl, p.cash_pnl,
            p.total_bought, p.mergeable, p.redeemable, p.title, p.slug, p.event_slug, p.icon,
            p.neg_risk, p.end_date, p.updated_at,
            lf.last_fill_price, lf.last_fill_ts,
            ff.first_fill_ts
        FROM positions p
        LEFT JOIN LATERAL (
            -- 마지막 fill 의 실제 가격 + ts. (fills_cid_ts_idx 활용, cheap)
            SELECT price AS last_fill_price, ts AS last_fill_ts
            FROM fills
            WHERE condition_id = p.condition_id AND outcome_index = p.outcome_index
              AND is_ours = false
            ORDER BY ts DESC LIMIT 1
        ) lf ON true
        LEFT JOIN LATERAL (
            -- 첫 fill ts — first-leg 식별용 (양 outcome 의 ts 비교)
            SELECT MIN(ts) AS first_fill_ts
            FROM fills
            WHERE condition_id = p.condition_id AND outcome_index = p.outcome_index
              AND is_ours = false
        ) ff ON true
        WHERE {where_clause}
          {q_filter}
    "#,
        where_clause = where_clause,
        q_filter = if has_q {
            "AND (LOWER(p.title) LIKE $1 OR LOWER(p.event_slug) LIKE $1 OR LOWER(p.outcome_name) LIKE $1)"
        } else { "" });
    let mut query = sqlx::query(&sql);
    if let Some(s) = &q_like { query = query.bind(s); }
    let rows = query.fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let positions: Vec<PositionEntry> = rows.into_iter().map(|r| PositionEntry {
        condition_id: r.get("condition_id"),
        outcome_index: r.get("outcome_index"),
        outcome_name: r.get("outcome_name"),
        size: r.get("size"),
        avg_price: r.get("avg_price"),
        current_price: r.get("current_price"),
        current_value: r.get("current_value"),
        initial_value: r.get("initial_value"),
        realized_pnl: r.get("realized_pnl"),
        cash_pnl: r.get("cash_pnl"),
        total_bought: r.get("total_bought"),
        mergeable: r.get("mergeable"),
        redeemable: r.get("redeemable"),
        title: r.get("title"),
        slug: r.get("slug"),
        event_slug: r.get("event_slug"),
        icon: r.get("icon"),
        neg_risk: r.get("neg_risk"),
        end_date: r.get("end_date"),
        updated_at: r.get("updated_at"),
        last_fill_price: r.get("last_fill_price"),
        last_fill_ts:    r.get("last_fill_ts"),
        first_fill_ts:   r.get("first_fill_ts"),
        quote_age_s:     r.try_get("quote_age_s").ok(),
    }).collect();

    if q.view == "flat" {
        return Ok(Json(serde_json::json!({ "positions": positions })));
    }

    // group by event_slug -> tree
    use std::collections::BTreeMap;
    let mut by_event: BTreeMap<String, Vec<PositionEntry>> = BTreeMap::new();
    let mut orphans: Vec<PositionEntry> = Vec::new();
    for p in positions {
        match p.event_slug.clone() {
            Some(es) if !es.is_empty() => by_event.entry(es).or_default().push(p),
            _ => orphans.push(p),
        }
    }
    let mut events: Vec<EventNode> = Vec::with_capacity(by_event.len());
    for (event_slug, mut legs) in by_event {
        legs.sort_by_key(|p| (p.condition_id.clone(), p.outcome_index));
        let mut by_cid: BTreeMap<String, Vec<PositionEntry>> = BTreeMap::new();
        for p in legs { by_cid.entry(p.condition_id.clone()).or_default().push(p); }
        let mut markets: Vec<MarketNode> = Vec::new();
        for (cid, outcomes) in by_cid {
            let title = outcomes[0].title.clone();
            let slug = outcomes[0].slug.clone();
            let neg_risk = outcomes[0].neg_risk;
            let end_date = outcomes[0].end_date;
            let cur_val: f64 = outcomes.iter().map(|o| o.current_value).sum();
            let cash: f64 = outcomes.iter().map(|o| o.cash_pnl).sum();
            let real: f64 = outcomes.iter().map(|o| o.realized_pnl).sum();
            let (category, family_param) = categorize_market(
                title.as_deref(), slug.as_deref(), &event_slug);
            markets.push(MarketNode {
                condition_id: cid,
                title, slug, neg_risk, end_date,
                current_value: cur_val,
                cash_pnl: cash,
                realized_pnl: real,
                outcomes,
                category, family_param,
            });
        }
        // Category priority 로 정렬 — 동일 카테고리 안에서 family_param (numeric) 으로 다시 정렬.
        // moneyline → ou-totals (낮은 threshold 부터) → spread → first-inning → other
        fn cat_rank(c: &str) -> i32 {
            match c {
                "moneyline"     => 0,
                "ou-totals"     => 1,
                "spread"        => 2,
                "first-inning"  => 3,
                "alt-spread"    => 4,
                _               => 9,
            }
        }
        markets.sort_by(|a, b| {
            let ra = cat_rank(&a.category);
            let rb = cat_rank(&b.category);
            if ra != rb { return ra.cmp(&rb); }
            // 동일 category — family_param 의 numeric value 로 (O/U threshold 등)
            let pa: f64 = a.family_param.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let pb: f64 = b.family_param.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if (pa - pb).abs() > 1e-9 {
                return pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal);
            }
            // tie-break: current_value desc
            b.current_value.partial_cmp(&a.current_value).unwrap_or(std::cmp::Ordering::Equal)
        });
        let market_count = markets.len() as i64;
        let leg_count: i64 = markets.iter().map(|m| m.outcomes.len() as i64).sum();
        let total_value: f64 = markets.iter().map(|m| m.current_value).sum();
        let total_cash: f64 = markets.iter().map(|m| m.cash_pnl).sum();
        let total_real: f64 = markets.iter().map(|m| m.realized_pnl).sum();
        let any_redeem = markets.iter().any(|m| m.outcomes.iter().any(|o| o.redeemable));
        let any_merge = markets.iter().any(|m| m.outcomes.iter().any(|o| o.mergeable));
        let earliest_end = markets.iter().filter_map(|m| m.end_date).min();
        let last_updated = markets.iter()
            .flat_map(|m| m.outcomes.iter().map(|o| o.updated_at))
            .max().unwrap_or_else(Utc::now);
        let latest_fill_ts = markets.iter()
            .flat_map(|m| m.outcomes.iter().filter_map(|o| o.last_fill_ts))
            .max();
        let sample_title = markets[0].title.clone();
        let sample_icon = markets[0].outcomes[0].icon.clone();
        events.push(EventNode {
            event_slug, sample_title, sample_icon, market_count, leg_count,
            total_value, total_cash_pnl: total_cash, total_realized_pnl: total_real,
            any_redeemable: any_redeem, any_mergeable: any_merge,
            earliest_end, last_updated, latest_fill_ts, markets,
        });
    }
    // 정렬
    use std::cmp::Ordering;
    let f_partial = |a: f64, b: f64| b.partial_cmp(&a).unwrap_or(Ordering::Equal);
    match q.sort.as_str() {
        "value"      => events.sort_by(|a, b| f_partial(a.total_value, b.total_value)),
        "value_asc"  => events.sort_by(|a, b| f_partial(b.total_value, a.total_value)),
        "pnl"        => events.sort_by(|a, b| f_partial(a.total_cash_pnl, b.total_cash_pnl)),
        "pnl_asc"    => events.sort_by(|a, b| f_partial(b.total_cash_pnl, a.total_cash_pnl)),
        "realized"   => events.sort_by(|a, b| f_partial(a.total_realized_pnl, b.total_realized_pnl)),
        "legs"       => events.sort_by(|a, b| b.leg_count.cmp(&a.leg_count)),
        "markets"    => events.sort_by(|a, b| b.market_count.cmp(&a.market_count)),
        "name"       => events.sort_by(|a, b| match (&a.sample_title, &b.sample_title) {
            (Some(x), Some(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => a.event_slug.cmp(&b.event_slug),
        }),
        "end"  | "end_asc" => events.sort_by(|a, b| match (a.earliest_end, b.earliest_end) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        "end_desc"  => events.sort_by(|a, b| match (a.earliest_end, b.earliest_end) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        // 체결순 — uses latest_fill_ts only.
        "fill" => events.sort_by(|a, b| match (a.latest_fill_ts, b.latest_fill_ts) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        "fill_asc" => events.sort_by(|a, b| match (a.latest_fill_ts, b.latest_fill_ts) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        "oldest"     => events.sort_by(|a, b| a.last_updated.cmp(&b.last_updated)),
        // "recent" default — 최근 업데이트 내림차순
        _ => events.sort_by(|a, b| b.last_updated.cmp(&a.last_updated)),
    }

    let total_value: f64 = events.iter().map(|e| e.total_value).sum();
    let total_cash: f64  = events.iter().map(|e| e.total_cash_pnl).sum();
    let total_real: f64  = events.iter().map(|e| e.total_realized_pnl).sum();
    let event_total = events.len();

    // 페이지네이션
    let page_size = q.page_size.max(10).min(500) as usize;
    let page = q.page.max(1) as usize;
    let start = (page - 1) * page_size;
    let paged_events: Vec<EventNode> = events.into_iter().skip(start).take(page_size).collect();

    Ok(Json(serde_json::json!({
        "events": paged_events,
        "orphans": orphans,
        "total_value": total_value,
        "total_cash_pnl": total_cash,
        "total_realized_pnl": total_real,
        "event_count": event_total,
        "page": page,
        "page_size": page_size,
        "total_pages": (event_total + page_size - 1) / page_size.max(1),
        "filter": q.filter,
        "sort": q.sort,
        "q": q.q,
    })))
}

pub async fn open_orders(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 자체 봇이 깔아둔 open orders (signed/posted/resting/partially_filled).
    // Polymarket UI 의 Open Orders 와 동일한 모양을 만들기 위해 markets / asset_to_condition
    // 메타를 LEFT JOIN — outcome_name / title / event_slug / icon 까지 함께 반환.
    let rows = sqlx::query(r#"
        SELECT o.order_id, o.clob_order_hash, o.condition_id, o.outcome_index,
               o.asset_id, o.token_id, o.side, o.price, o.size, o.size_usdc,
               o.fill_size_shares, o.fill_usdc, o.rebate_usdc, o.mid_at_place,
               o.kind, o.tag, o.status, o.signed_at, o.posted_at, o.filled_at,
               o.cancelled_at, o.cancelled_reason,
               atc.outcome_name AS outcome_name,
               m.question AS market_question,
               m.slug AS market_slug,
               m.event_slug AS event_slug,
               m.icon AS icon,
               m.neg_risk AS neg_risk,
               m.game_start AS game_start,
               m.end_date AS end_date,
               -- Per-outcome quote from asset_to_condition (trader-cached
               -- every tick). markets.best_bid is single-value per market =
               -- collapses both legs to the same price. The order's actual
               -- "current value" requires the leg-specific quote.
               atc.best_bid AS best_bid,
               atc.best_ask AS best_ask,
               EXTRACT(EPOCH FROM (NOW() - atc.quoted_at))::bigint AS quote_age_s
          FROM orders o
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = o.condition_id
           AND atc.outcome_index = o.outcome_index
          LEFT JOIN markets m
            ON m.condition_id = o.condition_id
         WHERE o.status IN ('signed','posted','resting','partially_filled')
         ORDER BY o.signed_at DESC
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let now = Utc::now();
    let orders: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        let signed_at: DateTime<Utc> = r.get("signed_at");
        let posted_at: Option<DateTime<Utc>> = r.get("posted_at");
        let size: f64 = r.get("size");
        let fill: f64 = r.get("fill_size_shares");
        let price: f64 = r.get("price");
        let remaining = (size - fill).max(0.0);
        let placed_age_sec = (now - signed_at).num_seconds();
        serde_json::json!({
            "order_id":         r.get::<i64, _>("order_id"),
            "clob_order_hash":  r.get::<Option<String>, _>("clob_order_hash"),
            "condition_id":     r.get::<String, _>("condition_id"),
            "outcome_index":    r.get::<i16, _>("outcome_index"),
            "outcome_name":     r.get::<Option<String>, _>("outcome_name"),
            "asset_id":         r.get::<String, _>("asset_id"),
            "token_id":         r.get::<Option<String>, _>("token_id"),
            "side":             r.get::<String, _>("side"),
            "price":            price,
            "size":             size,
            "size_usdc":        r.get::<f64, _>("size_usdc"),
            "fill_size_shares": fill,
            "fill_usdc":        r.get::<f64, _>("fill_usdc"),
            "remaining_shares": remaining,
            "remaining_usdc":   remaining * price,
            "kind":             r.get::<String, _>("kind"),
            "tag":              r.get::<String, _>("tag"),
            "status":           r.get::<String, _>("status"),
            "signed_at":        signed_at,
            "posted_at":        posted_at,
            "placed_age_sec":   placed_age_sec,
            "title":            r.get::<Option<String>, _>("market_question"),
            "slug":             r.get::<Option<String>, _>("market_slug"),
            "event_slug":       r.get::<Option<String>, _>("event_slug"),
            "icon":             r.get::<Option<String>, _>("icon"),
            "neg_risk":         r.get::<bool, _>("neg_risk"),
            "game_start":       r.get::<Option<DateTime<Utc>>, _>("game_start"),
            "end_date":         r.get::<Option<DateTime<Utc>>, _>("end_date"),
            "best_bid":         r.get::<Option<f64>, _>("best_bid"),
            "best_ask":         r.get::<Option<f64>, _>("best_ask"),
            "mid_at_place":     r.get::<Option<f64>, _>("mid_at_place"),
            "quote_age_s":      r.try_get::<Option<i64>, _>("quote_age_s").ok().flatten(),
        })
    }).collect();

    let total_open = orders.len();
    let total_remaining_usdc: f64 = orders.iter()
        .filter_map(|o| o.get("remaining_usdc").and_then(|v| v.as_f64()))
        .sum();

    Ok(Json(serde_json::json!({
        "orders":               orders,
        "total_open":           total_open,
        "total_remaining_usdc": total_remaining_usdc,
        "now":                  now,
    })))
}

/// `/api/live/orders/recent` — closed (filled/cancelled/rejected) orders + their meta.
///
/// Returns up to 100 most-recent FILLED rows + up to 100 most-recent CANCELLED/
/// REJECTED rows. Using a single ORDER BY LIMIT N caused filled history to be
/// hidden whenever the bot churned through many cancels (e.g. cap_breach
/// thrashing on a settled market). Split caps guarantee both buckets are
/// reachable for the frontend filter pills.
pub async fn closed_orders(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let rows = sqlx::query(r#"
        WITH filled AS (
            SELECT o.* FROM orders o
             WHERE status IN ('filled','partially_filled')
             ORDER BY COALESCE(o.filled_at, o.signed_at) DESC
             LIMIT 100
        ),
        cancelled AS (
            SELECT o.* FROM orders o
             WHERE status IN ('cancelled','rejected','expired')
             ORDER BY COALESCE(o.cancelled_at, o.signed_at) DESC
             LIMIT 100
        ),
        combined AS (
            SELECT * FROM filled
            UNION ALL
            SELECT * FROM cancelled
        )
        SELECT c.order_id, c.clob_order_hash, c.condition_id, c.outcome_index,
               c.side, c.price, c.size, c.size_usdc,
               c.fill_size_shares, c.fill_usdc, c.rebate_usdc,
               c.kind, c.tag, c.status, c.signed_at, c.filled_at, c.cancelled_at,
               c.cancelled_reason,
               atc.outcome_name AS outcome_name,
               m.question AS market_question,
               m.slug AS market_slug,
               m.event_slug AS event_slug,
               m.icon AS icon
          FROM combined c
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = c.condition_id
           AND atc.outcome_index = c.outcome_index
          LEFT JOIN markets m
            ON m.condition_id = c.condition_id
         ORDER BY COALESCE(c.filled_at, c.cancelled_at, c.signed_at) DESC
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let orders: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        let size: f64 = r.get("size");
        let fill: f64 = r.get("fill_size_shares");
        serde_json::json!({
            "order_id":         r.get::<i64, _>("order_id"),
            "clob_order_hash":  r.get::<Option<String>, _>("clob_order_hash"),
            "condition_id":     r.get::<String, _>("condition_id"),
            "outcome_index":    r.get::<i16, _>("outcome_index"),
            "outcome_name":     r.get::<Option<String>, _>("outcome_name"),
            "side":             r.get::<String, _>("side"),
            "price":            r.get::<f64, _>("price"),
            "size":             size,
            "size_usdc":        r.get::<f64, _>("size_usdc"),
            "fill_size_shares": fill,
            "fill_usdc":        r.get::<f64, _>("fill_usdc"),
            "rebate_usdc":      r.get::<f64, _>("rebate_usdc"),
            "kind":             r.get::<String, _>("kind"),
            "tag":              r.get::<String, _>("tag"),
            "status":           r.get::<String, _>("status"),
            "signed_at":        r.get::<DateTime<Utc>, _>("signed_at"),
            "filled_at":        r.get::<Option<DateTime<Utc>>, _>("filled_at"),
            "cancelled_at":     r.get::<Option<DateTime<Utc>>, _>("cancelled_at"),
            "cancelled_reason": r.get::<Option<String>, _>("cancelled_reason"),
            "title":            r.get::<Option<String>, _>("market_question"),
            "slug":             r.get::<Option<String>, _>("market_slug"),
            "event_slug":       r.get::<Option<String>, _>("event_slug"),
            "icon":             r.get::<Option<String>, _>("icon"),
        })
    }).collect();

    Ok(Json(serde_json::json!({ "orders": orders, "now": Utc::now() })))
}

/// `/api/live/positions` — bot-owned positions, tree-grouped by event_slug.
///
/// Same `PortfolioTree` shape as `/api/portfolio/positions` so the web tree
/// component is identical, but data is built from `fills WHERE is_ours=true`.
/// In paper mode this is the only source of truth — `positions` table reflects
/// only on-chain (data-api) holdings, not the bot's simulated fills.
pub async fn live_positions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PortfolioQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // Build per-leg aggregates from is_ours fills, joined with markets / atc for
    // metadata (title, event_slug, icon, current best_bid/ask → mid).
    let q_like = q.q.as_ref().map(|s| format!("%{}%", s.to_lowercase()));
    let has_q = q_like.is_some();

    let sql = format!(r#"
        WITH bot_fills AS (
            SELECT condition_id, outcome_index,
                   -- BUY shares minus SELL shares (S2 exits)
                   SUM(CASE WHEN side='BUY'  THEN size      ELSE 0          END) AS total_buy_size,
                   SUM(CASE WHEN side='BUY'  THEN usdc_size ELSE 0          END) AS total_buy_usdc,
                   SUM(CASE WHEN side='SELL' THEN size      ELSE 0          END) AS total_sell_size,
                   SUM(CASE WHEN side='SELL' THEN usdc_size ELSE 0          END) AS total_sell_usdc,
                   MIN(ts) AS first_fill_ts,
                   MAX(ts) AS last_fill_ts
              FROM fills
             WHERE is_ours = true
             GROUP BY condition_id, outcome_index
        ),
        merge_agg AS (
            -- MERGE removes equal shares from BOTH legs (matched pair recycled to $1)
            SELECT condition_id,
                   SUM(shares) AS merged_shares,
                   SUM(realized_pnl) AS merged_pnl
              FROM merges
             GROUP BY condition_id
        ),
        redeem_agg AS (
            -- REDEEM is per-leg (winner gets $1, loser gets $0)
            SELECT condition_id, outcome_index,
                   SUM(shares) AS redeemed_shares,
                   SUM(realized_pnl) AS redeemed_pnl
              FROM redemptions
             GROUP BY condition_id, outcome_index
        ),
        last_fill AS (
            SELECT DISTINCT ON (condition_id, outcome_index)
                   condition_id, outcome_index, price AS last_fill_price, ts AS last_fill_ts
              FROM fills
             WHERE is_ours = true
             ORDER BY condition_id, outcome_index, ts DESC
        ),
        net_positions AS (
            -- NET held = BUY - SELL - MERGE - REDEEM
            -- (MERGE removes shares from both legs; REDEEM is per-leg)
            SELECT bf.condition_id, bf.outcome_index,
                   bf.total_buy_size, bf.total_buy_usdc,
                   bf.total_sell_size, bf.total_sell_usdc,
                   COALESCE(mg.merged_shares, 0)  AS merged_shares,
                   COALESCE(mg.merged_pnl,    0)  AS merged_pnl,
                   COALESCE(rd.redeemed_shares,0) AS redeemed_shares,
                   COALESCE(rd.redeemed_pnl,   0) AS redeemed_pnl,
                   GREATEST(
                       bf.total_buy_size
                       - bf.total_sell_size
                       - COALESCE(mg.merged_shares,   0)
                       - COALESCE(rd.redeemed_shares, 0),
                       0
                   ) AS net_size,
                   CASE WHEN bf.total_buy_size > 0
                        THEN bf.total_buy_usdc / bf.total_buy_size
                        ELSE NULL END AS avg_price,
                   bf.first_fill_ts, bf.last_fill_ts
              FROM bot_fills bf
              LEFT JOIN merge_agg  mg ON mg.condition_id = bf.condition_id
              LEFT JOIN redeem_agg rd ON rd.condition_id = bf.condition_id
                                      AND rd.outcome_index = bf.outcome_index
        )
        SELECT  bf.condition_id, bf.outcome_index::int2 AS outcome_index,
                COALESCE(atc.outcome_name, '?'::text) AS outcome_name,
                bf.net_size  AS size,
                bf.avg_price,
                -- Per-outcome mid from asset_to_condition (trader-cached every tick).
                CASE WHEN atc.best_bid IS NOT NULL AND atc.best_ask IS NOT NULL
                     THEN (atc.best_bid + atc.best_ask) / 2.0
                     ELSE COALESCE(atc.last_trade, atc.best_bid, atc.best_ask)
                END AS current_price,
                -- MTM = current_price × NET held shares (NOT gross bought)
                bf.net_size * COALESCE(
                    CASE WHEN atc.best_bid IS NOT NULL AND atc.best_ask IS NOT NULL
                         THEN (atc.best_bid + atc.best_ask) / 2.0
                         ELSE COALESCE(atc.last_trade, atc.best_bid, atc.best_ask, 0)
                    END, 0
                ) AS current_value,
                -- Initial value of CURRENTLY HELD shares = net_size × avg_price
                -- (cost basis preserved across MERGE/REDEEM/SELL per planner spec)
                COALESCE(bf.net_size * bf.avg_price, 0) AS initial_value,
                -- Realized PnL = MERGE recovery + REDEEM payout + S2 SELL revenue/cost
                -- merged_pnl & redeemed_pnl already pre-computed by trader.
                -- SELL PnL = sell_usdc - (sell_size × avg_price).
                (bf.merged_pnl + bf.redeemed_pnl
                 + (bf.total_sell_usdc - COALESCE(bf.total_sell_size * bf.avg_price, 0))
                ) AS realized_pnl,
                -- Unrealized PnL = current_value − initial_value (on net held only)
                (bf.net_size * COALESCE(
                    CASE WHEN atc.best_bid IS NOT NULL AND atc.best_ask IS NOT NULL
                         THEN (atc.best_bid + atc.best_ask) / 2.0
                         ELSE COALESCE(atc.last_trade, atc.best_bid, atc.best_ask, 0)
                    END, 0
                ) - COALESCE(bf.net_size * bf.avg_price, 0)) AS cash_pnl,
                bf.total_buy_size AS total_bought,
                false AS mergeable,
                false AS redeemable,
                m.question AS title,
                m.slug     AS slug,
                -- event_slug fallback: collector 가 event_slug 채우지 못한
                -- 매치도 cid 로 그룹화되어 트리에 표시되도록.
                COALESCE(NULLIF(m.event_slug, ''), m.slug, bf.condition_id) AS event_slug,
                m.icon     AS icon,
                COALESCE(m.neg_risk, false) AS neg_risk,
                m.end_date AS end_date,
                GREATEST(bf.last_fill_ts, m.refreshed_at) AS updated_at,
                lf.last_fill_price,
                lf.last_fill_ts,
                bf.first_fill_ts,
                EXTRACT(EPOCH FROM (NOW() - atc.quoted_at))::bigint AS quote_age_s
          FROM net_positions bf
          LEFT JOIN markets m
            ON m.condition_id = bf.condition_id
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = bf.condition_id
           AND atc.outcome_index = bf.outcome_index
          LEFT JOIN last_fill lf
            ON lf.condition_id = bf.condition_id
           AND lf.outcome_index = bf.outcome_index
         WHERE bf.net_size > 0.1 {q_filter}
    "#, q_filter = if has_q {
        "AND (LOWER(m.question) LIKE $1 OR LOWER(m.event_slug) LIKE $1 OR LOWER(atc.outcome_name) LIKE $1)"
    } else { "" });

    let mut query = sqlx::query(&sql);
    if let Some(s) = &q_like { query = query.bind(s); }
    let rows = query.fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let positions: Vec<PositionEntry> = rows.into_iter().map(|r| PositionEntry {
        condition_id: r.get("condition_id"),
        outcome_index: r.get("outcome_index"),
        outcome_name: r.get("outcome_name"),
        size: r.get("size"),
        avg_price: r.get("avg_price"),
        current_price: r.get("current_price"),
        current_value: r.get("current_value"),
        initial_value: r.get("initial_value"),
        realized_pnl: r.get("realized_pnl"),
        cash_pnl: r.get("cash_pnl"),
        total_bought: r.get("total_bought"),
        mergeable: r.get("mergeable"),
        redeemable: r.get("redeemable"),
        title: r.get("title"),
        slug: r.get("slug"),
        event_slug: r.get("event_slug"),
        icon: r.get("icon"),
        neg_risk: r.get("neg_risk"),
        end_date: r.get("end_date"),
        updated_at: r.get::<Option<DateTime<Utc>>, _>("updated_at").unwrap_or_else(Utc::now),
        last_fill_price: r.get("last_fill_price"),
        last_fill_ts:    r.get("last_fill_ts"),
        first_fill_ts:   r.get("first_fill_ts"),
        quote_age_s:     r.try_get("quote_age_s").ok(),
    }).collect();

    if q.view == "flat" {
        return Ok(Json(serde_json::json!({ "positions": positions })));
    }

    // ── tree grouping (same as `positions` handler) ──
    use std::collections::BTreeMap;
    let mut by_event: BTreeMap<String, Vec<PositionEntry>> = BTreeMap::new();
    let mut orphans: Vec<PositionEntry> = Vec::new();
    for p in positions {
        match p.event_slug.clone() {
            Some(es) if !es.is_empty() => by_event.entry(es).or_default().push(p),
            _ => orphans.push(p),
        }
    }
    let mut events: Vec<EventNode> = Vec::with_capacity(by_event.len());
    for (event_slug, mut legs) in by_event {
        legs.sort_by_key(|p| (p.condition_id.clone(), p.outcome_index));
        let mut by_cid: BTreeMap<String, Vec<PositionEntry>> = BTreeMap::new();
        for p in legs { by_cid.entry(p.condition_id.clone()).or_default().push(p); }
        let mut markets: Vec<MarketNode> = Vec::new();
        for (cid, outcomes) in by_cid {
            let title    = outcomes[0].title.clone();
            let slug     = outcomes[0].slug.clone();
            let neg_risk = outcomes[0].neg_risk;
            let end_date = outcomes[0].end_date;
            let cur_val: f64 = outcomes.iter().map(|o| o.current_value).sum();
            let cash: f64    = outcomes.iter().map(|o| o.cash_pnl).sum();
            let real: f64    = outcomes.iter().map(|o| o.realized_pnl).sum();
            let (category, family_param) = categorize_market(
                title.as_deref(), slug.as_deref(), &event_slug);
            markets.push(MarketNode {
                condition_id: cid, title, slug, neg_risk, end_date,
                current_value: cur_val, cash_pnl: cash, realized_pnl: real,
                outcomes, category, family_param,
            });
        }
        // moneyline → ou-totals → spread → first-inning → alt-spread → other
        fn cat_rank(c: &str) -> i32 {
            match c {
                "moneyline"=>0, "ou-totals"=>1, "spread"=>2,
                "first-inning"=>3, "alt-spread"=>4, _=>9,
            }
        }
        markets.sort_by(|a, b| {
            let ra = cat_rank(&a.category); let rb = cat_rank(&b.category);
            if ra != rb { return ra.cmp(&rb); }
            let pa: f64 = a.family_param.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let pb: f64 = b.family_param.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if (pa - pb).abs() > 1e-9 {
                return pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal);
            }
            b.current_value.partial_cmp(&a.current_value).unwrap_or(std::cmp::Ordering::Equal)
        });
        let market_count = markets.len() as i64;
        let leg_count: i64 = markets.iter().map(|m| m.outcomes.len() as i64).sum();
        let total_value: f64 = markets.iter().map(|m| m.current_value).sum();
        let total_cash: f64  = markets.iter().map(|m| m.cash_pnl).sum();
        let total_real: f64  = markets.iter().map(|m| m.realized_pnl).sum();
        let any_redeem = markets.iter().any(|m| m.outcomes.iter().any(|o| o.redeemable));
        let any_merge  = markets.iter().any(|m| m.outcomes.iter().any(|o| o.mergeable));
        let earliest_end = markets.iter().filter_map(|m| m.end_date).min();
        let last_updated = markets.iter()
            .flat_map(|m| m.outcomes.iter().map(|o| o.updated_at))
            .max().unwrap_or_else(Utc::now);
        let latest_fill_ts = markets.iter()
            .flat_map(|m| m.outcomes.iter().filter_map(|o| o.last_fill_ts))
            .max();
        let sample_title = markets[0].title.clone();
        let sample_icon  = markets[0].outcomes[0].icon.clone();
        events.push(EventNode {
            event_slug, sample_title, sample_icon, market_count, leg_count,
            total_value, total_cash_pnl: total_cash, total_realized_pnl: total_real,
            any_redeemable: any_redeem, any_mergeable: any_merge,
            earliest_end, last_updated, latest_fill_ts, markets,
        });
    }

    use std::cmp::Ordering;
    let f_partial = |a: f64, b: f64| b.partial_cmp(&a).unwrap_or(Ordering::Equal);
    match q.sort.as_str() {
        "value"      => events.sort_by(|a, b| f_partial(a.total_value, b.total_value)),
        "value_asc"  => events.sort_by(|a, b| f_partial(b.total_value, a.total_value)),
        "pnl"        => events.sort_by(|a, b| f_partial(a.total_cash_pnl, b.total_cash_pnl)),
        "pnl_asc"    => events.sort_by(|a, b| f_partial(b.total_cash_pnl, a.total_cash_pnl)),
        "realized"   => events.sort_by(|a, b| f_partial(a.total_realized_pnl, b.total_realized_pnl)),
        "legs"       => events.sort_by(|a, b| b.leg_count.cmp(&a.leg_count)),
        "markets"    => events.sort_by(|a, b| b.market_count.cmp(&a.market_count)),
        "name"       => events.sort_by(|a, b| match (&a.sample_title, &b.sample_title) {
            (Some(x), Some(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
            (Some(_), None) => Ordering::Less, (None, Some(_)) => Ordering::Greater,
            _ => a.event_slug.cmp(&b.event_slug),
        }),
        "end" | "end_asc" => events.sort_by(|a, b| match (a.earliest_end, b.earliest_end) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less, (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        "end_desc"  => events.sort_by(|a, b| match (a.earliest_end, b.earliest_end) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less, (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        // 체결순 — uses latest_fill_ts only (NOT last_updated, which advances
        // on every quote refresh). Events without any fills sort last.
        "fill" => events.sort_by(|a, b| match (a.latest_fill_ts, b.latest_fill_ts) {
            (Some(x), Some(y)) => y.cmp(&x),                  // newest first
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        "fill_asc" => events.sort_by(|a, b| match (a.latest_fill_ts, b.latest_fill_ts) {
            (Some(x), Some(y)) => x.cmp(&y),                  // oldest first
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }),
        "oldest"     => events.sort_by(|a, b| a.last_updated.cmp(&b.last_updated)),
        _            => events.sort_by(|a, b| b.last_updated.cmp(&a.last_updated)),
    }

    let total_value: f64 = events.iter().map(|e| e.total_value).sum();
    let total_cash:  f64 = events.iter().map(|e| e.total_cash_pnl).sum();
    let total_real:  f64 = events.iter().map(|e| e.total_realized_pnl).sum();
    let event_total = events.len();

    let page_size = q.page_size.max(10).min(500) as usize;
    let page = q.page.max(1) as usize;
    let start = (page - 1) * page_size;
    let paged_events: Vec<EventNode> = events.into_iter().skip(start).take(page_size).collect();

    Ok(Json(serde_json::json!({
        "events":             paged_events,
        "orphans":            orphans,
        "total_value":        total_value,
        "total_cash_pnl":     total_cash,
        "total_realized_pnl": total_real,
        "event_count":        event_total,
        "page":               page,
        "page_size":          page_size,
        "total_pages":        (event_total + page_size - 1) / page_size.max(1),
        "filter":             q.filter,
        "sort":               q.sort,
        "q":                  q.q,
    })))
}

/// `/api/live/portfolio-summary` — bot의 polymarket-style 헤더 KPI.
///
/// `scope=recent` → 최근 24h 활동한 마켓만 (default, 폴리마켓 portfolio 탭과 매칭)
/// `scope=all`    → 트레이더가 거래한 모든 마켓 누적
///
/// Returns the same shape as `/api/portfolio/summary` so the React component
/// can be near-identical: positions_value · biggest_win · predictions ·
/// gross_24h · joined_at · alias / wallet.
pub async fn live_portfolio_summary(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LiveScopeQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let scope = q.scope.as_deref().unwrap_or("recent");

    // Per-outcome mid from asset_to_condition (trader-cached) — markets table
    // has only one bid/ask per row which conflated legs.
    let mid_expr = "CASE WHEN atc.best_bid IS NOT NULL AND atc.best_ask IS NOT NULL \
                     THEN (atc.best_bid + atc.best_ask) / 2.0 \
                     ELSE COALESCE(atc.last_trade, atc.best_bid, atc.best_ask, 0) END";

    // positions_value — bot fills aggregated × current mid price
    let where_recent = if scope == "recent" {
        "AND f.condition_id IN (SELECT DISTINCT condition_id FROM fills \
                                WHERE is_ours = TRUE \
                                  AND ts > NOW() - INTERVAL '24 hours')"
    } else { "" };

    let sql = format!(r#"
        WITH per_leg AS (
            SELECT f.condition_id, f.outcome_index,
                   SUM(CASE WHEN f.side='BUY'  THEN f.size ELSE -f.size END) AS net_size,
                   SUM(CASE WHEN f.side='BUY'  THEN f.usdc_size ELSE 0 END) AS gross_buy
              FROM fills f
             WHERE f.is_ours = TRUE
               {where_recent}
             GROUP BY f.condition_id, f.outcome_index
            HAVING SUM(CASE WHEN f.side='BUY' THEN f.size ELSE -f.size END) > 0
        )
        SELECT
            COALESCE(SUM(per_leg.net_size * {mid_expr}), 0)::float8 AS positions_value,
            COALESCE(SUM(per_leg.gross_buy), 0)::float8             AS total_gross_buy,
            COUNT(DISTINCT per_leg.condition_id)::int8              AS position_legs
          FROM per_leg
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = per_leg.condition_id
           AND atc.outcome_index = per_leg.outcome_index
    "#);
    let pos_row = sqlx::query(&sql).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;
    let positions_value: f64 = pos_row.get("positions_value");
    let _gross_total: f64    = pos_row.get("total_gross_buy");
    let position_legs: i64   = pos_row.get("position_legs");

    // predictions (= total fills count) + 24h
    let pred_row = sqlx::query(r#"
        SELECT
            COUNT(*)::int8                                                       AS predictions,
            COUNT(*) FILTER (WHERE ts > NOW() - INTERVAL '24 hours')::int8        AS predictions_24h,
            COALESCE(SUM(usdc_size) FILTER (WHERE ts > NOW() - INTERVAL '24 hours'), 0)::float8 AS gross_24h,
            MIN(ts) AS joined_at
          FROM fills
         WHERE is_ours = TRUE
    "#).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;
    let predictions: i64       = pred_row.get("predictions");
    let predictions_24h: i64   = pred_row.get("predictions_24h");
    let gross_24h: f64         = pred_row.get("gross_24h");
    let joined_at: Option<DateTime<Utc>> = pred_row.get("joined_at");

    // biggest_win — best single-position cash_pnl (current mid − avg_buy) × shares.
    // Real settlement / merge PnL would be more meaningful but isn't tracked yet.
    let bw_row = sqlx::query(&format!(r#"
        WITH per_leg AS (
            SELECT f.condition_id, f.outcome_index,
                   SUM(CASE WHEN f.side='BUY'  THEN f.size ELSE -f.size END) AS net_size,
                   SUM(CASE WHEN f.side='BUY'  THEN f.usdc_size ELSE 0 END) AS gross_buy,
                   m.question AS title, m.slug AS slug, m.event_slug AS event_slug
              FROM fills f
              LEFT JOIN markets m ON m.condition_id = f.condition_id
             WHERE f.is_ours = TRUE
               {where_recent}
             GROUP BY f.condition_id, f.outcome_index, m.question, m.slug, m.event_slug
        )
        SELECT
            per_leg.net_size * COALESCE({mid_expr}, 0) - per_leg.gross_buy AS pnl,
            per_leg.title, per_leg.slug, per_leg.event_slug
          FROM per_leg
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = per_leg.condition_id
           AND atc.outcome_index = per_leg.outcome_index
         ORDER BY per_leg.net_size * COALESCE({mid_expr}, 0) - per_leg.gross_buy DESC NULLS LAST
         LIMIT 1
    "#)).fetch_optional(&state.pool).await.map_err(ErrorBody::new)?;
    let (biggest_win, biggest_win_title, biggest_win_slug, biggest_win_event) = match bw_row {
        Some(r) => (
            r.get::<Option<f64>, _>("pnl"),
            r.get::<Option<String>, _>("title"),
            r.get::<Option<String>, _>("slug"),
            r.get::<Option<String>, _>("event_slug"),
        ),
        None => (None, None, None, None),
    };

    // trader_status 의 wallet
    let st_row = sqlx::query(
        "SELECT mode, wallet FROM trader_status WHERE id = 1"
    ).fetch_optional(&state.pool).await.map_err(ErrorBody::new)?;
    let (mode, wallet_addr): (String, Option<String>) = match st_row {
        Some(r) => {
            let mode: String = r.get("mode");
            let w: Option<serde_json::Value> = r.get("wallet");
            let addr = w.as_ref().and_then(|v| v.get("address")).and_then(|v| v.as_str()).map(String::from);
            (mode, addr)
        }
        None => ("paper".into(), None),
    };

    Ok(Json(serde_json::json!({
        "alias":               if mode == "live" { "rn1-trader · LIVE" } else { "rn1-trader · PAPER" },
        "wallet":              wallet_addr.unwrap_or_else(|| "(paper · no wallet)".into()),
        "scope":               scope,
        "positions_value":     positions_value,
        "position_legs":       position_legs,
        "biggest_win":         biggest_win,
        "biggest_win_title":   biggest_win_title,
        "biggest_win_slug":    biggest_win_slug,
        "biggest_win_event":   biggest_win_event,
        "predictions":         predictions,
        "predictions_24h":     predictions_24h,
        "gross_24h":           gross_24h,
        "joined_at":           joined_at.map(|d| d.to_rfc3339()).unwrap_or_else(|| Utc::now().to_rfc3339()),
        "now":                 Utc::now().to_rfc3339(),
    })))
}

#[derive(Deserialize)]
pub struct LiveScopeQ {
    pub scope: Option<String>,
}

#[derive(Deserialize)]
pub struct LivePnlSeriesQ {
    pub tf: Option<String>,     // 1D / 1W / 1M / 1Y / YTD / ALL
    pub scope: Option<String>,  // recent / all
}

/// `/api/live/pnl-series` — bot의 cumulative PnL time series (mirror /api/portfolio/pnl-series).
///
/// Approach: bot fills 만 보고 시간 순서대로 cumulative gross_buy / cumulative
/// positions value 를 계산. realized PnL 은 0 (MERGE/REDEEM 추적 전), net =
/// (positions_value_at_t) − (gross_buy_cumulative_at_t).
///
/// 시간 단위 (`tf`) 에 따라 bucket 폭이 다름:
///   1D → 1h × 24
///   1W → 3h × 56
///   1M → 6h × 120
///   1Y/YTD/ALL → 1d
pub async fn live_pnl_series(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LivePnlSeriesQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tf = q.tf.as_deref().unwrap_or("1D");
    let scope = q.scope.as_deref().unwrap_or("recent");
    let now = Utc::now();
    let win_end = now;
    let (win_start, bucket_count, bucket_minutes) = match tf {
        "1W"  => (now - chrono::Duration::days(7),    56i64,   3 * 60i64),
        "1M"  => (now - chrono::Duration::days(30),   120,     6 * 60),
        "1Y"  => (now - chrono::Duration::days(365),  365,     24 * 60),
        "YTD" => {
            let year_start = chrono::DateTime::<Utc>::from_naive_utc_and_offset(
                chrono::NaiveDate::from_ymd_opt(now.date_naive().year(), 1, 1).unwrap()
                    .and_hms_opt(0, 0, 0).unwrap(), Utc);
            let days = (now - year_start).num_days().max(1);
            (year_start, days.min(400), 24 * 60)
        }
        "ALL" => {
            // first fill ts → now, daily buckets
            let first: Option<DateTime<Utc>> = sqlx::query_scalar(
                "SELECT MIN(ts) FROM fills WHERE is_ours = TRUE"
            ).fetch_one(&state.pool).await.unwrap_or(None);
            let start = first.unwrap_or_else(|| now - chrono::Duration::days(30));
            let days = (now - start).num_days().max(1);
            (start, days.min(720), 24 * 60)
        }
        _ /* 1D */ => (now - chrono::Duration::hours(24), 24, 60),
    };

    let scope_filter = if scope == "recent" {
        "AND f.condition_id IN (SELECT DISTINCT condition_id FROM fills \
                                WHERE is_ours = TRUE \
                                  AND ts > NOW() - INTERVAL '24 hours')"
    } else { "" };

    // Pull all bot fills with current market mid (used as MTM proxy for past
    // bucket times — assumes price ≈ stable; lazy approximation).
    let rows = sqlx::query(&format!(r#"
        SELECT f.ts, f.condition_id, f.outcome_index, f.side, f.size, f.usdc_size,
               COALESCE(
                   CASE WHEN atc.best_bid IS NOT NULL AND atc.best_ask IS NOT NULL
                        THEN (atc.best_bid + atc.best_ask) / 2.0
                        ELSE COALESCE(atc.last_trade, atc.best_bid, atc.best_ask)
                   END, 0)::float8 AS current_mid
          FROM fills f
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = f.condition_id
           AND atc.outcome_index = f.outcome_index
         WHERE f.is_ours = TRUE
           AND f.ts >= $1
           {scope_filter}
         ORDER BY f.ts ASC
    "#))
    .bind(win_start)
    .fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    #[derive(Default, Clone)]
    struct LegState {
        net_size: f64,     // bought - sold - merged - redeemed
        gross_buy: f64,    // total USDC spent on BUY fills (running)
        sold_usdc: f64,    // total USDC received from SELL fills
        sold_size: f64,    // total shares sold (S2 exits)
        avg_buy_px: f64,   // gross_buy / total_buy_size (for SELL cost basis)
        total_buy_size: f64,
        current_mid: f64,
    }
    use std::collections::HashMap;
    let mut state_map: HashMap<(String, i16), LegState> = HashMap::new();

    // Pre-fill state_map with fills *before* the window so positions_value at
    // window_start is correct (not 0). Each leg tracks gross BUY usdc + size
    // (for avg_buy_px), plus SELL usdc + size.
    let pre_rows = sqlx::query(&format!(r#"
        SELECT f.condition_id, f.outcome_index,
               SUM(CASE WHEN f.side='BUY'  THEN f.size      ELSE 0     END) AS total_buy_size,
               SUM(CASE WHEN f.side='BUY'  THEN f.usdc_size ELSE 0     END) AS gross_buy,
               SUM(CASE WHEN f.side='SELL' THEN f.size      ELSE 0     END) AS sold_size,
               SUM(CASE WHEN f.side='SELL' THEN f.usdc_size ELSE 0     END) AS sold_usdc,
               COALESCE(
                   CASE WHEN atc.best_bid IS NOT NULL AND atc.best_ask IS NOT NULL
                        THEN (atc.best_bid + atc.best_ask) / 2.0
                        ELSE COALESCE(atc.last_trade, atc.best_bid, atc.best_ask)
                   END, 0)::float8 AS current_mid
          FROM fills f
          LEFT JOIN asset_to_condition atc
            ON atc.condition_id = f.condition_id
           AND atc.outcome_index = f.outcome_index
         WHERE f.is_ours = TRUE
           AND f.ts < $1
           {scope_filter}
         GROUP BY f.condition_id, f.outcome_index, atc.best_bid, atc.best_ask, atc.last_trade
    "#))
    .bind(win_start)
    .fetch_all(&state.pool).await.map_err(ErrorBody::new)?;
    for r in &pre_rows {
        let cid: String = r.get("condition_id");
        let leg: i16 = r.get("outcome_index");
        let buy_size: f64 = r.get("total_buy_size");
        let gross_buy: f64 = r.get("gross_buy");
        let sold_size: f64 = r.get("sold_size");
        let sold_usdc: f64 = r.get("sold_usdc");
        let avg_buy_px = if buy_size > 0.0 { gross_buy / buy_size } else { 0.0 };
        state_map.insert((cid, leg), LegState {
            net_size:       buy_size - sold_size,   // merges/redeems applied below
            gross_buy,
            sold_usdc,
            sold_size,
            avg_buy_px,
            total_buy_size: buy_size,
            current_mid:    r.get("current_mid"),
        });
    }

    // Pre-window MERGEs (subtract from both legs of their cid)
    let pre_merges = sqlx::query(
        "SELECT condition_id, SUM(shares) AS s, SUM(realized_pnl) AS p
           FROM merges WHERE ts < $1 GROUP BY condition_id"
    ).bind(win_start).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;
    let mut pre_merge_pnl = 0.0_f64;
    for r in &pre_merges {
        let cid: String = r.get("condition_id");
        let shares: f64 = r.get("s");
        let pnl: f64 = r.get("p");
        pre_merge_pnl += pnl;
        for leg in 0..2i16 {
            if let Some(st) = state_map.get_mut(&(cid.clone(), leg)) {
                st.net_size = (st.net_size - shares).max(0.0);
            }
        }
    }
    // Pre-window REDEMPTIONs (per-leg)
    let pre_redeems = sqlx::query(
        "SELECT condition_id, outcome_index, SUM(shares) AS s, SUM(realized_pnl) AS p
           FROM redemptions WHERE ts < $1 GROUP BY condition_id, outcome_index"
    ).bind(win_start).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;
    let mut pre_redeem_pnl = 0.0_f64;
    for r in &pre_redeems {
        let cid: String = r.get("condition_id");
        let leg: i16 = r.get("outcome_index");
        let shares: f64 = r.get("s");
        let pnl: f64 = r.get("p");
        pre_redeem_pnl += pnl;
        if let Some(st) = state_map.get_mut(&(cid, leg)) {
            st.net_size = (st.net_size - shares).max(0.0);
        }
    }

    // In-window MERGEs & REDEMPTIONs, sorted by ts for replay alongside fills.
    let merge_rows = sqlx::query(
        "SELECT ts, condition_id, shares, realized_pnl FROM merges WHERE ts >= $1 ORDER BY ts ASC"
    ).bind(win_start).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;
    let redeem_rows = sqlx::query(
        "SELECT ts, condition_id, outcome_index, shares, realized_pnl
           FROM redemptions WHERE ts >= $1 ORDER BY ts ASC"
    ).bind(win_start).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let bucket_dur = chrono::Duration::minutes(bucket_minutes);
    let mut points: Vec<serde_json::Value> = Vec::with_capacity(bucket_count as usize + 1);
    let mut fill_idx = 0usize;
    let fill_count = rows.len();
    let mut merge_idx = 0usize;
    let mut redeem_idx = 0usize;
    let mut cum_realized = pre_merge_pnl + pre_redeem_pnl;
    // Pre-window SELL realized: sold_usdc - sold_size × avg_buy_px (per leg)
    for st in state_map.values() {
        cum_realized += st.sold_usdc - st.sold_size * st.avg_buy_px;
    }

    for i in 0..=bucket_count {
        let bucket_ts = win_start + bucket_dur * (i as i32);
        if bucket_ts > win_end { break; }
        // Apply BUY/SELL fills, MERGEs, REDEEMs in time order up to bucket_ts.
        while fill_idx < fill_count {
            let fr = &rows[fill_idx];
            let ts: DateTime<Utc> = fr.get("ts");
            if ts > bucket_ts { break; }
            let cid: String = fr.get("condition_id");
            let leg: i16 = fr.get("outcome_index");
            let side: String = fr.get("side");
            let size: f64 = fr.get("size");
            let usdc: f64 = fr.get("usdc_size");
            let mid: f64 = fr.get("current_mid");
            let st = state_map.entry((cid, leg)).or_default();
            st.current_mid = mid;
            if side == "BUY" {
                st.net_size += size;
                st.gross_buy += usdc;
                st.total_buy_size += size;
                st.avg_buy_px = if st.total_buy_size > 0.0 {
                    st.gross_buy / st.total_buy_size
                } else { 0.0 };
            } else {
                // S2 SELL: realize (usdc - size × avg_buy_px)
                st.net_size -= size;
                st.sold_size += size;
                st.sold_usdc += usdc;
                cum_realized += usdc - size * st.avg_buy_px;
            }
            fill_idx += 1;
        }
        while merge_idx < merge_rows.len() {
            let mr = &merge_rows[merge_idx];
            let ts: DateTime<Utc> = mr.get("ts");
            if ts > bucket_ts { break; }
            let cid: String = mr.get("condition_id");
            let shares: f64 = mr.get("shares");
            let pnl: f64 = mr.get("realized_pnl");
            cum_realized += pnl;
            for leg in 0..2i16 {
                if let Some(st) = state_map.get_mut(&(cid.clone(), leg)) {
                    st.net_size = (st.net_size - shares).max(0.0);
                }
            }
            merge_idx += 1;
        }
        while redeem_idx < redeem_rows.len() {
            let rr = &redeem_rows[redeem_idx];
            let ts: DateTime<Utc> = rr.get("ts");
            if ts > bucket_ts { break; }
            let cid: String = rr.get("condition_id");
            let leg: i16 = rr.get("outcome_index");
            let shares: f64 = rr.get("shares");
            let pnl: f64 = rr.get("realized_pnl");
            cum_realized += pnl;
            if let Some(st) = state_map.get_mut(&(cid, leg)) {
                st.net_size = (st.net_size - shares).max(0.0);
            }
            redeem_idx += 1;
        }
        // Unrealized = sum(net_size × current_mid - net_size × avg_buy_px)
        //            = sum(net_size × (current_mid - avg_buy_px))
        let unrealized: f64 = state_map.values()
            .map(|s| s.net_size.max(0.0) * (s.current_mid - s.avg_buy_px))
            .sum();
        let pos_value: f64 = state_map.values()
            .map(|s| s.net_size.max(0.0) * s.current_mid)
            .sum();
        let net = cum_realized + unrealized;
        points.push(serde_json::json!({
            "ts":         bucket_ts.to_rfc3339(),
            "realized":   cum_realized,
            "unrealized": unrealized,
            "net":        net,
            "pos_value":  pos_value,
            "value":      net,
        }));
    }

    let start_net = points.first().and_then(|p| p.get("net")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end_net   = points.last().and_then(|p| p.get("net")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    Ok(Json(serde_json::json!({
        "tf":           tf,
        "scope":        scope,
        "interval":     format!("{}min", bucket_minutes),
        "bucket":       format!("{}min", bucket_minutes),
        "metric":       "net",
        "source":       "trader_fills",
        "point_count":  points.len(),
        "points":       points,
        "start_net":    start_net,
        "end_net":      end_net,
        "delta_net":    end_net - start_net,
        "start_cum":    start_net,
        "end_cum":      end_net,
        "delta_cum":    end_net - start_net,
    })))
}

/// `/api/live/positions/{cid}/breakdown` — bot-only price-level breakdown.
/// Same shape as `position_breakdown` but filtered to is_ours=true.
pub async fn live_position_breakdown(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(cid): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let levels = sqlx::query(r#"
        SELECT outcome_index,
               COALESCE(outcome_name, '?') AS outcome_name,
               side,
               ROUND((price * 100)::numeric)::int AS price_cents,
               COUNT(*)::bigint        AS fill_count,
               SUM(size)::float8       AS total_size,
               SUM(usdc_size)::float8  AS total_usdc,
               MIN(ts) AS first_ts,
               MAX(ts) AS last_ts
          FROM fills
         WHERE condition_id = $1 AND is_ours = true
         GROUP BY outcome_index, outcome_name, side, price_cents
         ORDER BY outcome_index, side, price_cents DESC
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let price_levels: Vec<serde_json::Value> = levels.into_iter().map(|r| {
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: String = r.get("outcome_name");
        let side: String = r.get("side");
        let price_cents: i32 = r.get("price_cents");
        let fill_count: i64 = r.get("fill_count");
        let total_size: f64 = r.get("total_size");
        let total_usdc: f64 = r.get("total_usdc");
        let first_ts: DateTime<Utc> = r.get("first_ts");
        let last_ts: DateTime<Utc> = r.get("last_ts");
        serde_json::json!({
            "outcome_index": outcome_index,
            "outcome_name":  outcome_name,
            "side":          side,
            "price_cents":   price_cents,
            "price":         (price_cents as f64) / 100.0,
            "fill_count":    fill_count,
            "total_size":    total_size,
            "total_usdc":    total_usdc,
            "first_ts":      first_ts,
            "last_ts":       last_ts,
        })
    }).collect();

    let fills = sqlx::query(r#"
        SELECT ts, outcome_index, outcome_name, side, size, price, usdc_size, tx_hash
          FROM fills
         WHERE condition_id = $1 AND is_ours = true
         ORDER BY ts DESC
         LIMIT 500
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let fills_json: Vec<serde_json::Value> = fills.into_iter().map(|r| {
        let ts: DateTime<Utc> = r.get("ts");
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: Option<String> = r.get("outcome_name");
        let side: String = r.get("side");
        let size: f64 = r.get("size");
        let price: f64 = r.get("price");
        let usdc_size: f64 = r.get("usdc_size");
        let tx_hash: Option<String> = r.get("tx_hash");
        serde_json::json!({
            "ts": ts, "outcome_index": outcome_index, "outcome_name": outcome_name,
            "side": side, "size": size, "price": price, "usdc_size": usdc_size,
            "tx_hash": tx_hash,
        })
    }).collect();

    Ok(Json(serde_json::json!({
        "condition_id":  cid,
        "outcomes":      [],  // not needed for the bot view (positions table is data-api side)
        "price_levels":  price_levels,
        "fills":         fills_json,
    })))
}

/// `/api/live/markets-upcoming` — ATP/WTA 경기 일정.
///
/// Polymarket sports schedule mirror — game_start 기준 정렬, 윈도우 -3h ~ +96h.
/// universe prefix 매칭 (slug head 가 atp / wta) + doubles/itf 제외 + active+open.
///
/// Returns the schedule view: market meta + best_bid/ask + 봇이 거래한 적 있는지.
pub async fn live_markets_upcoming(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UpcomingQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let universe: Vec<String> = q.universe.as_deref().unwrap_or("atp,wta").split(',')
        .map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    let prefix_like: Vec<String> = universe.iter().map(|p| format!("{}-%", p)).collect();
    let hours_back: i64 = q.hours_back.unwrap_or(3);
    let hours_ahead: i64 = q.hours_ahead.unwrap_or(96);
    let min_vol: f64 = q.min_vol.unwrap_or(0.0);

    let rows = sqlx::query(r#"
        SELECT m.condition_id, m.slug, m.event_slug, m.question, m.neg_risk,
               m.game_start, m.end_date, m.volume_num,
               m.best_bid, m.best_ask, m.last_trade_price, m.icon,
               m.accepting_orders, m.uma_status, m.closed,
               EXISTS (SELECT 1 FROM fills f
                       WHERE f.condition_id = m.condition_id
                         AND f.is_ours = true) AS has_bot_fills
          FROM markets m
         WHERE m.slug LIKE ANY($1)
           AND COALESCE(m.closed, false) = false
           AND m.game_start IS NOT NULL
           AND m.game_start BETWEEN NOW() - ($2::bigint * INTERVAL '1 hour')
                                 AND NOW() + ($3::bigint * INTERVAL '1 hour')
           AND m.slug NOT ILIKE '%doubles%'
           AND m.slug NOT ILIKE '%-itf-%'
           AND COALESCE(m.volume_num, 0) >= $4
         ORDER BY m.game_start ASC
         LIMIT 200
    "#)
    .bind(&prefix_like[..])
    .bind(hours_back)
    .bind(hours_ahead)
    .bind(min_vol)
    .fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let now = Utc::now();
    let markets: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        let game_start: Option<DateTime<Utc>> = r.get("game_start");
        let secs_to_start: Option<i64> = game_start.map(|gs| (gs - now).num_seconds());
        let game_state = match secs_to_start {
            Some(s) if s >  -90 * 60 && s < 30 * 60 => "live",   // 시작 30분 전 ~ 1.5h 후 = in-play 근사
            Some(s) if s >= 30 * 60                 => "upcoming",
            Some(s) if s <= -90 * 60                => "post",
            _ => "unknown",
        };
        let best_bid: Option<f64> = r.get("best_bid");
        let best_ask: Option<f64> = r.get("best_ask");
        let spread = match (best_bid, best_ask) {
            (Some(b), Some(a)) => Some(a - b),
            _ => None,
        };
        let mid = match (best_bid, best_ask) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        };
        serde_json::json!({
            "condition_id":      r.get::<String, _>("condition_id"),
            "slug":              r.get::<String, _>("slug"),
            "event_slug":        r.get::<Option<String>, _>("event_slug"),
            "question":          r.get::<Option<String>, _>("question"),
            "neg_risk":          r.get::<bool, _>("neg_risk"),
            "game_start":        game_start,
            "end_date":          r.get::<Option<DateTime<Utc>>, _>("end_date"),
            "secs_to_start":     secs_to_start,
            "game_state":        game_state,
            "volume_num":        r.get::<Option<f64>, _>("volume_num"),
            "best_bid":          best_bid,
            "best_ask":          best_ask,
            "mid":               mid,
            "spread":            spread,
            "last_trade_price":  r.get::<Option<f64>, _>("last_trade_price"),
            "icon":              r.get::<Option<String>, _>("icon"),
            "accepting_orders":  r.get::<Option<DateTime<Utc>>, _>("accepting_orders"),
            "uma_status":        r.get::<Option<String>, _>("uma_status"),
            "has_bot_fills":     r.get::<bool, _>("has_bot_fills"),
        })
    }).collect();

    Ok(Json(serde_json::json!({
        "now":              now,
        "universe":         universe,
        "hours_back":       hours_back,
        "hours_ahead":      hours_ahead,
        "min_vol":          min_vol,
        "market_count":     markets.len(),
        "markets":          markets,
    })))
}

#[derive(Deserialize)]
pub struct UpcomingQ {
    pub universe:    Option<String>,   // "atp,wta" 기본
    pub hours_back:  Option<i64>,      // -3h 기본
    pub hours_ahead: Option<i64>,      // +96h 기본
    pub min_vol:     Option<f64>,      // 0 기본
}

/// Trader state — single-row trader_status + today's daily totals.
pub async fn trader_state(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let row = sqlx::query(r#"
        SELECT mode, running, focus_slug, current_tick, last_cycle_at,
               last_cycle_ms, candidates, ws_market_connected, ws_user_connected,
               kill_switch, last_error, wallet, config_snapshot, strategy,
               updated_at
          FROM trader_status WHERE id = 1
    "#).fetch_optional(&state.pool).await.map_err(ErrorBody::new)?;

    let daily = sqlx::query(r#"
        SELECT date::text AS date, gross_filled, realized_pnl, rebate,
               fills_count, kill_switch
          FROM trader_daily WHERE date = CURRENT_DATE
    "#).fetch_optional(&state.pool).await.map_err(ErrorBody::new)?;

    let bot = match row {
        Some(r) => serde_json::json!({
            "mode":                r.get::<String, _>("mode"),
            "running":             r.get::<bool, _>("running"),
            "focus_slug":          r.get::<Option<String>, _>("focus_slug"),
            "current_tick":        r.get::<i64, _>("current_tick"),
            "last_cycle_at":       r.get::<Option<DateTime<Utc>>, _>("last_cycle_at"),
            "last_cycle_ms":       r.get::<i64, _>("last_cycle_ms"),
            "candidates":          r.get::<i32, _>("candidates"),
            "ws_market_connected": r.get::<bool, _>("ws_market_connected"),
            "ws_user_connected":   r.get::<bool, _>("ws_user_connected"),
            "kill_switch":         r.get::<bool, _>("kill_switch"),
            "last_error":          r.get::<Option<String>, _>("last_error"),
            "wallet":              r.get::<Option<serde_json::Value>, _>("wallet"),
            "config":              r.get::<Option<serde_json::Value>, _>("config_snapshot"),
            "strategy":            r.get::<Option<serde_json::Value>, _>("strategy"),
            "updated_at":          r.get::<DateTime<Utc>, _>("updated_at"),
        }),
        None => serde_json::json!({
            "mode": "paper", "running": false,
            "note": "trader has not reported a tick yet — run `cargo run -p rn1-trader`",
        }),
    };

    let daily_json = match daily {
        Some(d) => serde_json::json!({
            "date":         d.get::<String, _>("date"),
            "gross_filled": d.get::<f64, _>("gross_filled"),
            "realized_pnl": d.get::<f64, _>("realized_pnl"),
            "rebate":       d.get::<f64, _>("rebate"),
            "fills_count":  d.get::<i32, _>("fills_count"),
            "kill_switch":  d.get::<bool, _>("kill_switch"),
        }),
        None => serde_json::json!({
            "date": Utc::now().date_naive().to_string(),
            "gross_filled": 0.0, "realized_pnl": 0.0, "rebate": 0.0,
            "fills_count": 0, "kill_switch": false,
        }),
    };

    Ok(Json(serde_json::json!({
        "bot":   bot,
        "daily": daily_json,
        "now":   Utc::now(),
    })))
}

/// 단일 conditionId 에 대한 가격대별 fill 분해.
/// Frontend 의 OutcomeLine 확장에서 사용 — RN1 이 한 outcome 에 대해
/// 서로 다른 가격으로 몇 번 매수했는지 그룹화하여 보여준다.
/// price 는 1¢ 단위로 라운드해서 그룹핑 (예: 0.5100000001 / 0.5099999 → 51¢ 동일).
pub async fn position_breakdown(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(cid): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 1) 가격대별 그룹핑
    let levels = sqlx::query(r#"
        SELECT outcome_index,
               COALESCE(outcome_name, '?') AS outcome_name,
               side,
               ROUND((price * 100)::numeric)::int AS price_cents,
               COUNT(*)::bigint AS fill_count,
               SUM(size)::float8 AS total_size,
               SUM(usdc_size)::float8 AS total_usdc,
               MIN(ts) AS first_ts,
               MAX(ts) AS last_ts
        FROM fills
        WHERE condition_id = $1 AND is_ours = false
        GROUP BY outcome_index, outcome_name, side, price_cents
        ORDER BY outcome_index, side, price_cents DESC
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let price_levels: Vec<serde_json::Value> = levels.into_iter().map(|r| {
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: String = r.get("outcome_name");
        let side: String = r.get("side");
        let price_cents: i32 = r.get("price_cents");
        let fill_count: i64 = r.get("fill_count");
        let total_size: f64 = r.get("total_size");
        let total_usdc: f64 = r.get("total_usdc");
        let first_ts: DateTime<Utc> = r.get("first_ts");
        let last_ts: DateTime<Utc> = r.get("last_ts");
        serde_json::json!({
            "outcome_index": outcome_index,
            "outcome_name": outcome_name,
            "side": side,
            "price_cents": price_cents,
            "price": (price_cents as f64) / 100.0,
            "fill_count": fill_count,
            "total_size": total_size,
            "total_usdc": total_usdc,
            "first_ts": first_ts,
            "last_ts": last_ts,
        })
    }).collect();

    // 2) 개별 fills (최대 500개, 최신순)
    let fills = sqlx::query(r#"
        SELECT ts, outcome_index, outcome_name, side, size, price, usdc_size, tx_hash
        FROM fills
        WHERE condition_id = $1 AND is_ours = false
        ORDER BY ts DESC
        LIMIT 500
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let fills_json: Vec<serde_json::Value> = fills.into_iter().map(|r| {
        let ts: DateTime<Utc> = r.get("ts");
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: Option<String> = r.get("outcome_name");
        let side: String = r.get("side");
        let size: f64 = r.get("size");
        let price: f64 = r.get("price");
        let usdc_size: f64 = r.get("usdc_size");
        let tx_hash: Option<String> = r.get("tx_hash");
        serde_json::json!({
            "ts": ts,
            "outcome_index": outcome_index,
            "outcome_name": outcome_name,
            "side": side,
            "size": size,
            "price": price,
            "usdc_size": usdc_size,
            "tx_hash": tx_hash,
        })
    }).collect();

    // 3) 포지션 요약 (per-outcome)
    let pos_rows = sqlx::query(r#"
        SELECT outcome_index, outcome_name, size, avg_price, current_price,
               current_value, total_bought, cash_pnl, realized_pnl,
               redeemable, mergeable, title, event_slug
        FROM positions
        WHERE condition_id = $1
        ORDER BY outcome_index
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let outcomes: Vec<serde_json::Value> = pos_rows.into_iter().map(|r| {
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: String = r.get("outcome_name");
        let size: f64 = r.get("size");
        let avg_price: Option<f64> = r.get("avg_price");
        let current_price: Option<f64> = r.get("current_price");
        let current_value: f64 = r.get("current_value");
        let total_bought: f64 = r.get("total_bought");
        let cash_pnl: f64 = r.get("cash_pnl");
        let realized_pnl: f64 = r.get("realized_pnl");
        let redeemable: bool = r.get("redeemable");
        let mergeable: bool = r.get("mergeable");
        serde_json::json!({
            "outcome_index": outcome_index,
            "outcome_name": outcome_name,
            "size": size,
            "avg_price": avg_price,
            "current_price": current_price,
            "current_value": current_value,
            "total_bought": total_bought,
            "cash_pnl": cash_pnl,
            "realized_pnl": realized_pnl,
            "redeemable": redeemable,
            "mergeable": mergeable,
        })
    }).collect();

    Ok(Json(serde_json::json!({
        "condition_id": cid,
        "outcomes": outcomes,
        "price_levels": price_levels,
        "fills": fills_json,
    })))
}

#[derive(Deserialize)]
pub struct HistoryQ {
    /// 정렬: "newest" (ts desc) | "oldest" (ts asc) | "size" (size desc) |
    ///       "usdc" (usdc_size desc) | "price" (price desc) | "low_price" (price asc)
    #[serde(default = "default_history_sort")]
    pub sort: String,
    /// 검색 q (title / outcome / event_slug LIKE)
    #[serde(default)]
    pub q: Option<String>,
    /// side 필터: "BUY" | "SELL" | "ALL"
    #[serde(default = "default_side")]
    pub side: String,
    /// 페이지네이션
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_history_page_size")]
    pub page_size: u32,
}
fn default_history_sort() -> String { "newest".into() }
fn default_history_page_size() -> u32 { 100 }
fn default_side() -> String { "ALL".into() }

#[derive(Serialize)]
pub struct FillEntry {
    pub ts: DateTime<Utc>,
    pub condition_id: String,
    pub outcome_index: i16,
    pub outcome_name: Option<String>,
    pub side: String,
    pub size: f64,
    pub price: f64,
    pub usdc_size: f64,
    pub title: Option<String>,
    pub event_slug: Option<String>,
    pub icon: Option<String>,
    pub tx_hash: Option<String>,
    pub is_ours: bool,
}

pub async fn history(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HistoryQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let order_clause = match q.sort.as_str() {
        "oldest"     => "ts ASC",
        "size"       => "size DESC",
        "usdc"       => "usdc_size DESC",
        "price"      => "price DESC",
        "low_price"  => "price ASC",
        _            => "ts DESC", // newest
    };
    let mut where_clauses: Vec<String> = vec![];
    match q.side.as_str() {
        "BUY"  => where_clauses.push("side = 'BUY'".into()),
        "SELL" => where_clauses.push("side = 'SELL'".into()),
        _ => {}
    }
    let mut binds: Vec<String> = Vec::new();
    if let Some(qstr) = q.q.as_ref().filter(|s| !s.is_empty()) {
        where_clauses.push("(LOWER(title) LIKE $1 OR LOWER(event_slug) LIKE $1 OR LOWER(outcome_name) LIKE $1)".into());
        binds.push(format!("%{}%", qstr.to_lowercase()));
    }
    let where_sql = if where_clauses.is_empty() { "TRUE".to_string() }
                    else { where_clauses.join(" AND ") };

    let page_size = q.page_size.clamp(20, 1000) as i64;
    let page = q.page.max(1) as i64;
    let offset = (page - 1) * page_size;

    // total count
    let count_sql = format!("SELECT COUNT(*) FROM fills WHERE {}", where_sql);
    let mut count_q = sqlx::query_as::<_, (i64,)>(&count_sql);
    for b in &binds { count_q = count_q.bind(b); }
    let (total_count,): (i64,) = count_q.fetch_one(&state.pool).await.map_err(ErrorBody::new)?;

    // page
    let sql = format!(r#"
        SELECT ts, condition_id, outcome_index, outcome_name, side, size, price, usdc_size,
            title, event_slug, icon, tx_hash, is_ours
        FROM fills
        WHERE {where_sql}
        ORDER BY {order_clause}
        LIMIT {page_size} OFFSET {offset}
    "#);
    let mut q_data = sqlx::query(&sql);
    for b in &binds { q_data = q_data.bind(b); }
    let rows = q_data.fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let fills: Vec<FillEntry> = rows.into_iter().map(|r| FillEntry {
        ts: r.get("ts"),
        condition_id: r.get("condition_id"),
        outcome_index: r.get("outcome_index"),
        outcome_name: r.get("outcome_name"),
        side: r.get("side"),
        size: r.get("size"),
        price: r.get("price"),
        usdc_size: r.get("usdc_size"),
        title: r.get("title"),
        event_slug: r.get("event_slug"),
        icon: r.get("icon"),
        tx_hash: r.get("tx_hash"),
        is_ours: r.get("is_ours"),
    }).collect();
    Ok(Json(serde_json::json!({
        "fills": fills,
        "page": page,
        "page_size": page_size,
        "total_count": total_count,
        "total_pages": (total_count as u64 + page_size as u64 - 1) / (page_size as u64).max(1),
        "sort": q.sort,
        "side": q.side,
        "q": q.q,
    })))
}

pub async fn analysis(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SummaryQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let is_recent = q.scope == "recent";
    let pos_where = if is_recent {
        "(size > 0 OR redeemable OR mergeable) AND condition_id IN \
         (SELECT DISTINCT condition_id FROM fills \
          WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours')"
    } else {
        "(size > 0 OR redeemable OR mergeable)"
    };
    let fills_where = if is_recent {
        "is_ours = false AND ts > NOW() - INTERVAL '24 hours'"
    } else {
        "is_ours = false"
    };

    // 라이브 누적 (positions + fills) + forensic 상수
    let positions_summary: (Option<f64>, Option<f64>, Option<f64>, Option<i64>) =
        sqlx::query_as(&format!(
            "SELECT SUM(current_value), SUM(cash_pnl), SUM(realized_pnl), COUNT(*) \
             FROM positions WHERE {pos_where}"
        )).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;

    let fills_summary: (Option<i64>, Option<f64>, Option<i64>) = sqlx::query_as(&format!(
        "SELECT COUNT(*), SUM(usdc_size), COUNT(*) FILTER (WHERE side='BUY') \
         FROM fills WHERE {fills_where}"
    )).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;

    let categories: Vec<(String, i64, f64, f64)> = sqlx::query_as(&format!(
        "SELECT COALESCE(SUBSTRING(slug FROM '^[a-z0-9]+'), 'other') AS cat, \
                COUNT(*)::bigint, COALESCE(SUM(usdc_size),0)::float8, 0.0::float8 \
         FROM fills WHERE {fills_where} \
         GROUP BY cat ORDER BY 3 DESC LIMIT 20"
    )).fetch_all(&state.pool).await.unwrap_or_default();

    Ok(Json(serde_json::json!({
        "wallet": rn1_core::RN1_WALLET,
        "alias": rn1_core::RN1_ALIAS,
        "scope": q.scope,
        "scope_label": if is_recent { "최근 24h 활동 마켓 한정" } else { "전체" },
        "live": {
            "position_legs": positions_summary.3.unwrap_or(0),
            "current_value": positions_summary.0.unwrap_or(0.0),
            "cash_pnl": positions_summary.1.unwrap_or(0.0),
            "realized_pnl": positions_summary.2.unwrap_or(0.0),
            "fills_observed": fills_summary.0.unwrap_or(0),
            "gross_notional_observed": fills_summary.1.unwrap_or(0.0),
            "buy_count": fills_summary.2.unwrap_or(0),
        },
        "categories": categories.into_iter().map(|(cat, n, g, _)| serde_json::json!({
            "category": cat, "fills": n, "notional": g
        })).collect::<Vec<_>>(),
        "forensic_v0_1": {
            "window": "2026-04-28 ~ 2026-05-18 (21d)",
            "trades": 432_221,
            "buy_pct": 99.99977,
            "markets": 10_089,
            "two_sided_pct": 76.2,
            "neg_risk_pct": 26.1,
            "gross_notional": 39_885_153.0,
            "trade_pnl": 1_291_425.0,
            "merge_locked": 983_946.0,
            "residual_pnl": 334_072.0,
            "maker_rebate": 297_066.0,
            "net": 1_588_490.0,
            "daily_avg": 75_642.0,
        },
        "strategy_signature": {
            "entry": "양면 binary maker BUY (양 outcome 동시 limit BUY)",
            "exit": "MERGE (양쪽 동량) + REDEEM (resolution 후) — SELL 거의 없음",
            "alpha_sources": [
                "MERGE-locked margin: pair_count × (1 − Σavg_pair)",
                "Maker rebate: 0.75% × gross (sports fee schedule)",
                "Residual leg outcome (variance)"
            ],
            "non_used": "multi-option YES bundle arb (forensic net −$24K)",
            "universe": "sports moneyline / spread / O-U / draw (negRisk 26%)",
            "timing_p50": {
                "entry_lag_minutes": 18_875,
                "last_trade_minus_gameStart_min": -96
            }
        }
    })))
}

pub async fn redemptions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let rows = sqlx::query(r#"
        SELECT action_id, condition_id, action_type, size, tx_hash, status,
               realized_usdc, triggered_at, confirmed_at
        FROM position_actions
        ORDER BY triggered_at DESC LIMIT 200
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;
    let actions: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({
        "action_id": r.get::<i64, _>("action_id"),
        "condition_id": r.get::<String, _>("condition_id"),
        "action_type": r.get::<String, _>("action_type"),
        "size": r.get::<Option<f64>, _>("size"),
        "tx_hash": r.get::<Option<String>, _>("tx_hash"),
        "status": r.get::<String, _>("status"),
        "realized_usdc": r.get::<Option<f64>, _>("realized_usdc"),
        "triggered_at": r.get::<DateTime<Utc>, _>("triggered_at"),
        "confirmed_at": r.get::<Option<DateTime<Utc>>, _>("confirmed_at"),
    })).collect();
    Ok(Json(serde_json::json!({ "actions": actions })))
}

/// 헤더 KPI — 모두 24h activity 한정 (recent_cids 에 들어있는 conditionId 만).
/// scope=all 쿼리 파라미터로 전체값도 조회 가능.
#[derive(Deserialize)]
pub struct SummaryQ {
    /// "recent" (default — 24h activity) | "all" (전체)
    #[serde(default = "default_scope")]
    pub scope: String,
}
fn default_scope() -> String { "recent".into() }

pub async fn portfolio_summary(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SummaryQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let is_recent = q.scope == "recent";
    let pos_where = if is_recent {
        "(size > 0 OR redeemable OR mergeable) AND condition_id IN \
         (SELECT DISTINCT condition_id FROM fills \
          WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours')"
    } else {
        "(size > 0 OR redeemable OR mergeable)"
    };

    // Run all 4 queries CONCURRENTLY — they're independent and each one
    // previously stalled the page header. avg 494 ms → ~150 ms (slowest alone).
    // SQL strings bound to locals so `tokio::join!` can hold the futures.
    let pos_sql = format!(
        "SELECT COALESCE(SUM(current_value),0), COUNT(*) FROM positions WHERE {pos_where}"
    );
    let biggest_sql = format!(
        "SELECT realized_pnl, title, slug, event_slug FROM positions \
         WHERE realized_pnl IS NOT NULL AND {pos_where} \
         ORDER BY realized_pnl DESC NULLS LAST LIMIT 1"
    );
    let preds_sql: &str = if is_recent {
        "SELECT COUNT(*) FROM fills WHERE is_ours = false AND ts > NOW() - INTERVAL '24 hours'"
    } else {
        "SELECT COUNT(*) FROM fills WHERE is_ours = false"
    };
    let act24_sql: &str =
        "SELECT COUNT(*), COALESCE(SUM(usdc_size),0) FROM fills \
         WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours'";

    let pos_fut     = sqlx::query_as::<_, (Option<f64>, Option<i64>)>(&pos_sql).fetch_one(&state.pool);
    let biggest_fut = sqlx::query_as::<_, (Option<f64>, Option<String>, Option<String>, Option<String>)>(&biggest_sql).fetch_optional(&state.pool);
    let preds_fut   = sqlx::query_as::<_, (Option<i64>,)>(preds_sql).fetch_one(&state.pool);
    let act24_fut   = sqlx::query_as::<_, (Option<i64>, Option<f64>)>(act24_sql).fetch_one(&state.pool);

    let (pos, biggest, preds, act24) = tokio::join!(pos_fut, biggest_fut, preds_fut, act24_fut);
    let pos     = pos    .map_err(ErrorBody::new)?;
    let biggest = biggest.map_err(ErrorBody::new)?;
    let preds   = preds  .map_err(ErrorBody::new)?;
    let act24   = act24  .map_err(ErrorBody::new)?;

    Ok(Json(serde_json::json!({
        "wallet": rn1_core::RN1_WALLET,
        "alias": rn1_core::RN1_ALIAS,
        "scope": q.scope,
        "positions_value": pos.0.unwrap_or(0.0),
        "position_legs": pos.1.unwrap_or(0),
        "biggest_win": biggest.as_ref().and_then(|r| r.0),
        "biggest_win_title": biggest.as_ref().and_then(|r| r.1.clone()),
        "biggest_win_slug":  biggest.as_ref().and_then(|r| r.2.clone()),
        "biggest_win_event": biggest.as_ref().and_then(|r| r.3.clone()),
        "predictions": preds.0.unwrap_or(0),
        "predictions_24h": act24.0.unwrap_or(0),
        "gross_24h": act24.1.unwrap_or(0.0),
        "joined_at": "2024-12-12",
        "now": Utc::now(),
    })))
}

/// 현재 활동 중인 마켓들의 상세 — 진입 결정용 메트릭:
///  - bid/ask/spread (현재 quote 위치)
///  - volume (마켓 깊이)
///  - RN1 의 평균 진입가 vs current mid (slippage)
///  - 24h 안 RN1 fill 수, 평균 size (활동 강도)
///  - 게임 시작까지/지나간 시간
///  - tick size / max spread (LP 보상)
#[derive(Deserialize)]
pub struct MarketsDetailQ {
    /// "activity" (24h fills desc) | "spread" (asc) | "value" (desc) |
    /// "game" (eta asc) | "volume" (desc) | "slippage_abs" (abs desc) |
    /// "price_range" (desc) | "fills_asc" (활동 낮은 순)
    #[serde(default = "default_md_sort")]
    pub sort: String,
    /// 검색 q (title / event_slug LIKE)
    #[serde(default)]
    pub q: Option<String>,
    /// game_state 필터: "all" | "upcoming" | "live" | "post"
    #[serde(default = "default_md_state")]
    pub state: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_md_page_size")]
    pub page_size: u32,
}
fn default_md_sort() -> String { "activity".into() }
fn default_md_state() -> String { "all".into() }
fn default_md_page_size() -> u32 { 25 }

pub async fn markets_detail(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MarketsDetailQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 24h 안 RN1 trade 가 있던 condition_id 들 + 마켓 메타 + RN1 포지션 + 활동 통계
    // LEFT JOIN markets 이라 markets 테이블에 없는 cid 도 fills/positions 메타로 표시 가능.
    use rust_decimal::prelude::ToPrimitive;
    let rows = sqlx::query(r#"
        WITH recent_cids AS (
            SELECT condition_id, MAX(ts) AS last_fill_ts
            FROM fills
            WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours'
            GROUP BY condition_id
        ),
        latest_fill AS (
            SELECT DISTINCT ON (condition_id) condition_id, title, icon, event_slug, slug
            FROM fills
            WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours'
            ORDER BY condition_id, ts DESC
        )
        SELECT
            rc.condition_id,
            COALESCE(m.question, lf.title)        AS title,
            COALESCE(m.event_slug, lf.event_slug) AS event_slug,
            COALESCE(m.slug, lf.slug)             AS slug,
            COALESCE(m.icon, lf.icon)             AS icon,
            COALESCE(m.neg_risk, false)           AS neg_risk,
            m.best_bid, m.best_ask, m.last_trade_price, m.volume_num,
            m.order_min_size, m.order_price_min_tick_size, m.rewards_max_spread,
            m.game_start, m.end_date, m.uma_status,
            COALESCE(p_agg.total_current_value, 0.0),
            COALESCE(p_agg.total_size, 0.0),
            p_agg.avg_entry,
            f_agg.fills_24h,
            f_agg.gross_24h,
            f_agg.min_price_24h,
            f_agg.max_price_24h,
            rc.last_fill_ts
        FROM recent_cids rc
        LEFT JOIN latest_fill lf ON lf.condition_id = rc.condition_id
        LEFT JOIN markets m ON m.condition_id = rc.condition_id
        LEFT JOIN LATERAL (
            SELECT
                SUM(current_value) AS total_current_value,
                SUM(size) AS total_size,
                CASE WHEN SUM(size) > 0
                     THEN SUM(avg_price * size) / SUM(size) END AS avg_entry
            FROM positions WHERE condition_id = rc.condition_id
        ) p_agg ON true
        LEFT JOIN LATERAL (
            SELECT
                COUNT(*) AS fills_24h,
                SUM(usdc_size)::float8 AS gross_24h,
                MIN(price)::float8 AS min_price_24h,
                MAX(price)::float8 AS max_price_24h
            FROM fills
            WHERE condition_id = rc.condition_id
              AND is_ours = false
              AND ts > NOW() - INTERVAL '24 hours'
        ) f_agg ON true
        ORDER BY rc.last_fill_ts DESC
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let now = Utc::now();
    let markets: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        let cid: String = r.get(0);
        let title: Option<String> = r.get(1);
        let event_slug: Option<String> = r.get(2);
        let slug: Option<String> = r.get(3);
        let icon: Option<String> = r.get(4);
        let neg_risk: bool = r.get(5);
        let best_bid: Option<f64> = r.get(6);
        let best_ask: Option<f64> = r.get(7);
        let last_trade_price: Option<f64> = r.get(8);
        let volume_num: Option<f64> = r.get(9);
        let order_min_size: Option<f64> = r.get(10);
        let tick_size: Option<f64> = r.get(11);
        let max_spread: Option<f64> = r.get(12);
        let game_start: Option<DateTime<Utc>> = r.get(13);
        let end_date: Option<DateTime<Utc>> = r.get(14);
        let uma_status: Option<String> = r.get(15);
        let cur_value: Option<f64> = r.get(16);
        let size: Option<f64> = r.get(17);
        let avg_entry: Option<f64> = r.get(18);
        let fills_24h: Option<i64> = r.get(19);
        let gross_24h: Option<f64> = r.get(20);
        let min_fill_24h: Option<f64> = r.get(21);
        let max_fill_24h: Option<f64> = r.get(22);
        let last_fill_ts: Option<DateTime<Utc>> = r.get(23);

        let spread = match (best_bid, best_ask) {
            (Some(b), Some(a)) if a > b => Some(a - b),
            _ => None,
        };
        let mid = match (best_bid, best_ask) {
            (Some(b), Some(a)) => Some((a + b) / 2.0),
            _ => last_trade_price,
        };

        let cur_value = cur_value.unwrap_or(0.0);
        let size = size.unwrap_or(0.0);

        // slippage: RN1 평균 진입가 vs 현재 mid
        // BUY-only 가정: slippage_pct = (avg_entry - mid) / mid * 100
        let slippage_pct = match (avg_entry, mid) {
            (Some(ae), Some(m)) if m > 0.0 && ae > 0.0 => Some((ae - m) / m * 100.0),
            _ => None,
        };

        // 24h fill 평균 size (gross / fills)
        let fills_24h_i = fills_24h.unwrap_or(0);
        let avg_fill_size = match (fills_24h_i, gross_24h) {
            (n, Some(g)) if n > 0 => Some(g / n as f64),
            _ => None,
        };

        // 게임 시점 표시
        let game_eta_sec = game_start.map(|g| (g - now).num_seconds());
        let game_state = match game_eta_sec {
            Some(s) if s > 0 => "upcoming",
            Some(s) if s > -2*3600 => "live", // 게임 시작 후 2시간 안
            Some(_) => "post",
            None => "unknown",
        };

        let min_24h = min_fill_24h;
        let max_24h = max_fill_24h;

        // 가격 변동폭 (24h fills 의 max-min)
        let price_range_24h = match (min_24h, max_24h) {
            (Some(mn), Some(mx)) => Some(mx - mn),
            _ => None,
        };

        serde_json::json!({
            "condition_id": cid,
            "title": title,
            "event_slug": event_slug,
            "slug": slug,
            "icon": icon,
            "neg_risk": neg_risk,
            "uma_status": uma_status,
            "game_start": game_start,
            "game_eta_sec": game_eta_sec,
            "game_state": game_state,
            "end_date": end_date,
            // orderbook
            "best_bid": best_bid,
            "best_ask": best_ask,
            "mid": mid,
            "spread": spread,
            "spread_bps": spread.zip(mid).map(|(s, m)| if m > 0.0 { Some(s / m * 10000.0) } else { None }).flatten(),
            "last_trade_price": last_trade_price,
            "volume_num": volume_num,
            "order_min_size": order_min_size,
            "tick_size": tick_size,
            "rewards_max_spread": max_spread,
            // RN1 활동
            "rn1_current_value": cur_value,
            "rn1_total_size": size,
            "rn1_avg_entry": avg_entry,
            "slippage_pct": slippage_pct,
            // 24h fill 통계
            "fills_24h": fills_24h_i,
            "gross_24h": gross_24h.unwrap_or(0.0),
            "avg_fill_size": avg_fill_size,
            "price_min_24h": min_24h,
            "price_max_24h": max_24h,
            "price_range_24h": price_range_24h,
            "last_fill_ts": last_fill_ts,
        })
    }).collect();

    // 후처리 (server-side sort/filter/page) — SQL 결과는 그대로,
    // metric 필드들 (slippage, game_eta 등) 이 SQL 외에서 계산되어 후처리가 더 단순.
    let mut markets = markets;

    // 검색
    if let Some(qstr) = q.q.as_ref().filter(|s| !s.is_empty()) {
        let qs = qstr.to_lowercase();
        markets.retain(|m| {
            let t = m["title"].as_str().unwrap_or("").to_lowercase();
            let e = m["event_slug"].as_str().unwrap_or("").to_lowercase();
            t.contains(&qs) || e.contains(&qs)
        });
    }
    // state 필터
    if q.state != "all" {
        markets.retain(|m| m["game_state"].as_str() == Some(q.state.as_str()));
    }

    // 정렬
    use std::cmp::Ordering;
    fn fval(v: &serde_json::Value, k: &str) -> f64 { v[k].as_f64().unwrap_or(0.0) }
    fn fval_inf(v: &serde_json::Value, k: &str) -> f64 { v[k].as_f64().unwrap_or(f64::INFINITY) }
    match q.sort.as_str() {
        "spread"       => markets.sort_by(|a, b| fval_inf(a, "spread").partial_cmp(&fval_inf(b, "spread")).unwrap_or(Ordering::Equal)),
        "value"        => markets.sort_by(|a, b| fval(b, "rn1_current_value").partial_cmp(&fval(a, "rn1_current_value")).unwrap_or(Ordering::Equal)),
        "volume"       => markets.sort_by(|a, b| fval(b, "volume_num").partial_cmp(&fval(a, "volume_num")).unwrap_or(Ordering::Equal)),
        "game"         => markets.sort_by(|a, b| fval_inf(a, "game_eta_sec").partial_cmp(&fval_inf(b, "game_eta_sec")).unwrap_or(Ordering::Equal)),
        "slippage_abs" => markets.sort_by(|a, b| {
            let av = a["slippage_pct"].as_f64().map(f64::abs).unwrap_or(0.0);
            let bv = b["slippage_pct"].as_f64().map(f64::abs).unwrap_or(0.0);
            bv.partial_cmp(&av).unwrap_or(Ordering::Equal)
        }),
        "price_range"  => markets.sort_by(|a, b| fval(b, "price_range_24h").partial_cmp(&fval(a, "price_range_24h")).unwrap_or(Ordering::Equal)),
        "fills_asc"    => markets.sort_by(|a, b| fval(a, "fills_24h").partial_cmp(&fval(b, "fills_24h")).unwrap_or(Ordering::Equal)),
        // "activity" default
        _              => markets.sort_by(|a, b| fval(b, "fills_24h").partial_cmp(&fval(a, "fills_24h")).unwrap_or(Ordering::Equal)),
    }

    let total = markets.len();
    let page_size = q.page_size.clamp(5, 200) as usize;
    let page = q.page.max(1) as usize;
    let start = (page - 1) * page_size;
    let slice: Vec<serde_json::Value> = markets.into_iter().skip(start).take(page_size).collect();

    Ok(Json(serde_json::json!({
        "now": now,
        "market_count": total,
        "page": page,
        "page_size": page_size,
        "total_pages": (total + page_size - 1) / page_size.max(1),
        "sort": q.sort,
        "state_filter": q.state,
        "q": q.q,
        "markets": slice,
    })))
}

/// Profit/Loss 그래프용 timeseries. fills 의 누적 매수 비용 vs 정산 가치 비교는
/// 복잡하므로, 본 endpoint 는 시간 버킷별 활동 강도(누적 gross notional + fill 수)
/// 를 반환. 진짜 cumulative PnL 은 collector 가 pnl_snapshots 에 기록할 때
/// 갈아끼울 자리.
#[derive(Deserialize)]
pub struct PnlSeriesQ {
    /// "1D" | "1W" | "1M" | "1Y" | "YTD" | "ALL"
    #[serde(default = "default_tf")]
    pub tf: String,
    /// "recent" (24h activity 마켓만) | "all"
    #[serde(default = "default_scope")]
    pub scope: String,
}
fn default_tf() -> String { "1D".into() }

pub async fn pnl_series(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PnlSeriesQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 시간창 + 버킷 폭 — 폴리마켓 portfolio chart 와 동일 granularity:
    //  1D = 1h × 24 points
    //  1W = 3h × 56 points
    //  1M = 6h × 120 points
    //  1Y / YTD / ALL = 1d
    let (interval, bucket): (&str, &str) = match q.tf.as_str() {
        "1W"  => ("7 days",    "3 hours"),
        "1M"  => ("30 days",   "6 hours"),
        "1Y"  => ("365 days",  "1 day"),
        "YTD" => ("365 days",  "1 day"),
        "ALL" => ("3650 days", "1 day"),
        _     => ("24 hours",  "1 hour"),  // 1D
    };

    // 데이터 원천: pnl_snapshots (collector 가 5min 주기로 INSERT, scope 별로).
    // scope='recent' (24h activity 마켓 한정) 또는 'all' (전체).
    let snap_scope = if q.scope == "recent" { "recent" } else { "all" };
    let rows: Vec<(DateTime<Utc>, Option<f64>, Option<f64>, Option<f64>, Option<f64>)> =
        sqlx::query_as(&format!(r#"
            SELECT bucket AS ts, realized_pnl, unrealized,
                   COALESCE(realized_pnl, 0) + COALESCE(unrealized, 0) AS net_pnl,
                   gross_buy
            FROM (
                SELECT DISTINCT ON (bucket)
                    date_bin(INTERVAL '{bucket}', ts, TIMESTAMPTZ '2000-01-01') AS bucket,
                    ts, realized_pnl, unrealized, gross_buy
                FROM pnl_snapshots
                WHERE ts > NOW() - INTERVAL '{interval}'
                  AND scope = '{snap_scope}'
                ORDER BY bucket, ts DESC
            ) b
            ORDER BY bucket
        "#, bucket=bucket, interval=interval, snap_scope=snap_scope))
        .fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let points: Vec<serde_json::Value> = rows.iter().map(|(t, r, u, n, g)| {
        serde_json::json!({
            "ts": t,
            "realized": r.unwrap_or(0.0),
            "unrealized": u.unwrap_or(0.0),
            "net": n.unwrap_or(0.0),
            "gross_buy": g.unwrap_or(0.0),
            "value": n.unwrap_or(0.0),  // 그래프 main series
        })
    }).collect();

    let first_net = rows.first().and_then(|(_,_,_,n,_)| *n).unwrap_or(0.0);
    let last_net  = rows.last().and_then(|(_,_,_,n,_)| *n).unwrap_or(0.0);

    Ok(Json(serde_json::json!({
        "tf": q.tf,
        "scope": q.scope,
        "interval": interval,
        "bucket": bucket,
        "metric": "net_pnl",
        "points": points,
        "start_net": first_net,
        "end_net": last_net,
        "delta_net": last_net - first_net,
        // 호환 — 기존 frontend 의 start_cum/end_cum/delta_cum 도 net 으로 매핑
        "start_cum": first_net,
        "end_cum": last_net,
        "delta_cum": last_net - first_net,
        "source": "pnl_snapshots",
        "point_count": rows.len(),
    })))
}

// ============================================================================
// 분석/진단 endpoints — M0.5 baseline 검증, 진입 메이커 행동, 데이터 커버리지.
// ============================================================================

/// 메이커 quote lifecycle 분석 — RN1 이 어떤 시장에 언제 (게임 시작 대비)
/// 진입했고 가격이 어떻게 움직이는 동안 추가 매수했는지 정량화.
///
/// 사용자 가설:
///  · RN1 은 양면 binary maker BUY 로 진입
///  · 진입 후 가격이 움직이면 maker quote 를 갱신 (취소+재호가) 하며 추가 체결
///  · → fills 간 price stddev 가 크면 "가격 추적 갱신" 강함
///  · → first_fill_vs_game_min 으로 진입 시점 (게임 전/후) 파악
#[derive(Deserialize)]
pub struct MakerBehaviorQ {
    /// 분석 대상 윈도우 (시간). 기본 24h.
    #[serde(default = "default_mb_hours")]
    pub hours: i64,
    /// 정렬: "gross" (USDC 합계 내림), "fills" (체결 수 내림),
    ///       "entry" (게임 전 진입 빨랐던 순), "spread" (가격 분산 큰 순)
    #[serde(default = "default_mb_sort")]
    pub sort: String,
    /// 페이지네이션
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_mb_page_size")]
    pub page_size: u32,
}
fn default_mb_hours() -> i64 { 24 }
fn default_mb_sort() -> String { "gross".into() }
fn default_mb_page_size() -> u32 { 30 }

pub async fn maker_behavior(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MakerBehaviorQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let hours = q.hours.clamp(1, 720);
    // 마켓 단위 메이커 시그너처 — fills 의 시각/가격/크기 분포 + markets 의 game_start
    let rows = sqlx::query(&format!(r#"
        WITH market_fills AS (
            SELECT
                f.condition_id,
                MAX(f.title) AS title,
                MAX(f.event_slug) AS event_slug,
                MAX(f.icon) AS icon,
                MAX(m.game_start) AS game_start,
                MAX(m.end_date) AS end_date,
                MAX(m.uma_status) AS uma_status,
                MIN(f.ts) AS first_fill,
                MAX(f.ts) AS last_fill,
                COUNT(*)::bigint AS fills,
                SUM(f.usdc_size)::float8 AS gross_usdc,
                SUM(f.size)::float8 AS shares,
                AVG(f.price)::float8 AS avg_price,
                MIN(f.price)::float8 AS min_price,
                MAX(f.price)::float8 AS max_price,
                COALESCE(STDDEV_POP(f.price), 0)::float8 AS price_stddev,
                COUNT(DISTINCT f.outcome_index)::int AS sides_traded
            FROM fills f
            LEFT JOIN markets m ON m.condition_id = f.condition_id
            WHERE f.is_ours = false
              AND f.ts > NOW() - INTERVAL '{hours} hours'
            GROUP BY f.condition_id
        )
        SELECT * FROM market_fills
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let now = Utc::now();
    let mut out: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        let cid: String = r.get("condition_id");
        let title: Option<String> = r.get("title");
        let event_slug: Option<String> = r.get("event_slug");
        let icon: Option<String> = r.get("icon");
        let game_start: Option<DateTime<Utc>> = r.get("game_start");
        let end_date: Option<DateTime<Utc>> = r.get("end_date");
        let uma_status: Option<String> = r.get("uma_status");
        let first_fill: DateTime<Utc> = r.get("first_fill");
        let last_fill: DateTime<Utc> = r.get("last_fill");
        let fills: i64 = r.get("fills");
        let gross_usdc: f64 = r.get("gross_usdc");
        let shares: f64 = r.get("shares");
        let avg_price: f64 = r.get("avg_price");
        let min_price: f64 = r.get("min_price");
        let max_price: f64 = r.get("max_price");
        let price_stddev: f64 = r.get("price_stddev");
        let sides_traded: i32 = r.get("sides_traded");

        let active_min = (last_fill - first_fill).num_seconds() as f64 / 60.0;
        let first_vs_game_min = game_start.map(|gs| (first_fill - gs).num_seconds() as f64 / 60.0);
        let last_vs_game_min = game_start.map(|gs| (last_fill - gs).num_seconds() as f64 / 60.0);
        let avg_fill_size = if fills > 0 { shares / fills as f64 } else { 0.0 };
        let avg_fill_usdc = if fills > 0 { gross_usdc / fills as f64 } else { 0.0 };
        // inter-fill 평균 (전제: 균등 분포). 실시간 갱신 강도의 1차 proxy.
        let inter_fill_sec = if fills > 1 { (active_min * 60.0) / (fills - 1) as f64 } else { 0.0 };

        // 진입 phase 분류
        let entry_phase = match first_vs_game_min {
            Some(m) if m < -60.0  => "pre-game-1h+",  // 게임 1h+ 전 진입
            Some(m) if m < 0.0    => "pre-game",      // 게임 직전
            Some(m) if m < 60.0   => "live-1h",       // 게임 시작 후 1h 안
            Some(m) if m < 180.0  => "live-3h",
            Some(_)               => "post-game",
            None                  => "no-meta",
        };

        // 게임 까지 남은 시간 (음수 = 이미 시작/끝)
        let game_eta_sec = game_start.map(|g| (g - now).num_seconds());

        serde_json::json!({
            "condition_id": cid,
            "title": title,
            "event_slug": event_slug,
            "icon": icon,
            "uma_status": uma_status,
            "game_start": game_start,
            "game_eta_sec": game_eta_sec,
            "end_date": end_date,
            "first_fill": first_fill,
            "last_fill": last_fill,
            "fills": fills,
            "gross_usdc": gross_usdc,
            "shares": shares,
            "avg_price": avg_price,
            "min_price": min_price,
            "max_price": max_price,
            "price_range": max_price - min_price,
            "price_stddev": price_stddev,
            "avg_fill_size": avg_fill_size,
            "avg_fill_usdc": avg_fill_usdc,
            "active_minutes": active_min,
            "inter_fill_sec": inter_fill_sec,
            "sides_traded": sides_traded,
            "two_sided": sides_traded == 2,
            "first_vs_game_min": first_vs_game_min,
            "last_vs_game_min":  last_vs_game_min,
            "entry_phase": entry_phase,
        })
    }).collect();

    use std::cmp::Ordering;
    fn fval(v: &serde_json::Value, k: &str) -> f64 { v[k].as_f64().unwrap_or(0.0) }
    match q.sort.as_str() {
        "fills"  => out.sort_by(|a, b| fval(b, "fills").partial_cmp(&fval(a, "fills")).unwrap_or(Ordering::Equal)),
        "entry"  => out.sort_by(|a, b| fval(a, "first_vs_game_min").partial_cmp(&fval(b, "first_vs_game_min")).unwrap_or(Ordering::Equal)),
        "spread" => out.sort_by(|a, b| fval(b, "price_stddev").partial_cmp(&fval(a, "price_stddev")).unwrap_or(Ordering::Equal)),
        "range"  => out.sort_by(|a, b| fval(b, "price_range").partial_cmp(&fval(a, "price_range")).unwrap_or(Ordering::Equal)),
        "update" => out.sort_by(|a, b| fval(a, "inter_fill_sec").partial_cmp(&fval(b, "inter_fill_sec")).unwrap_or(Ordering::Equal)),
        _        => out.sort_by(|a, b| fval(b, "gross_usdc").partial_cmp(&fval(a, "gross_usdc")).unwrap_or(Ordering::Equal)),
    }

    // 집계 통계
    let n = out.len() as f64;
    let agg_two_sided = out.iter().filter(|v| v["two_sided"].as_bool() == Some(true)).count() as f64;
    // String keys (소유) — 아래에서 out 을 소비하기 전에 borrow 끝나야 함
    let phase_counts: std::collections::BTreeMap<String, usize> = {
        let mut m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for v in &out {
            let p = v["entry_phase"].as_str().unwrap_or("?").to_string();
            *m.entry(p).or_insert(0) += 1;
        }
        m
    };
    let avg_inter_fill = if n > 0.0 { out.iter().map(|v| fval(v, "inter_fill_sec")).sum::<f64>() / n } else { 0.0 };
    let avg_price_stddev = if n > 0.0 { out.iter().map(|v| fval(v, "price_stddev")).sum::<f64>() / n } else { 0.0 };
    let avg_price_range = if n > 0.0 { out.iter().map(|v| fval(v, "price_range")).sum::<f64>() / n } else { 0.0 };
    // 진입 분포: median first_vs_game_min (positive only)
    let mut first_vs_game_present: Vec<f64> = out.iter().filter_map(|v| v["first_vs_game_min"].as_f64()).collect();
    first_vs_game_present.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let median_entry_min = if first_vs_game_present.is_empty() { None }
        else { Some(first_vs_game_present[first_vs_game_present.len() / 2]) };

    // 페이지네이션
    let total = out.len();
    let page_size = q.page_size.clamp(10, 200) as usize;
    let page = q.page.max(1) as usize;
    let start = (page - 1) * page_size;
    let slice: Vec<serde_json::Value> = out.into_iter().skip(start).take(page_size).collect();

    Ok(Json(serde_json::json!({
        "window_hours": hours,
        "now": now,
        "total_markets": total,
        "page": page,
        "page_size": page_size,
        "total_pages": (total + page_size - 1) / page_size.max(1),
        "sort": q.sort,
        "markets": slice,
        "aggregate": {
            "two_sided_pct": if n > 0.0 { agg_two_sided / n * 100.0 } else { 0.0 },
            "avg_inter_fill_sec": avg_inter_fill,
            "avg_price_stddev": avg_price_stddev,
            "avg_price_range": avg_price_range,
            "median_entry_min_vs_game": median_entry_min,
            "phase_counts": phase_counts,
        },
    })))
}

/// Mid-price tracking — RN1 의 fill 이 maker rest (bid 에 안착) 인지, taker (ask 크로스)
/// 인지, inside-spread 인지 분류. book_events 의 fill 직전 best_bid/best_ask 와 비교.
/// 사용자 가설 "RN1 은 진입 후 가격 변동에 맞춰 maker quote 갱신" 의 *정밀* 검증.
#[derive(Deserialize)]
pub struct MidPriceQ {
    #[serde(default = "default_mp_hours")]
    pub hours: i64,
    #[serde(default = "default_mp_sample")]
    pub sample: i64,
}
fn default_mp_hours() -> i64 { 24 }
fn default_mp_sample() -> i64 { 5000 }

pub async fn mid_price_tracking(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MidPriceQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let hours = q.hours.clamp(1, 168);
    let sample = q.sample.clamp(100, 50_000);

    // 핵심 쿼리 — fills 각 건마다 book_events 의 직전 best_bid/ask 찾기 (LATERAL).
    // asset_to_condition 으로 (cid, outcome_idx) → asset_id 매핑.
    let rows = sqlx::query(&format!(r#"
        WITH recent_fills AS (
            SELECT f.condition_id, f.outcome_index, f.ts, f.price, f.size, f.usdc_size,
                   f.title, f.event_slug, f.slug,
                   atc.asset_id
            FROM fills f
            JOIN asset_to_condition atc ON atc.condition_id = f.condition_id
                                       AND atc.outcome_index = f.outcome_index
            WHERE f.is_ours = false
              AND f.ts > NOW() - INTERVAL '{hours} hours'
            ORDER BY f.ts DESC
            LIMIT {sample}
        )
        SELECT
            rf.condition_id, rf.outcome_index, rf.ts, rf.price, rf.size, rf.usdc_size,
            rf.title, rf.event_slug, rf.slug,
            bs.best_bid, bs.best_ask, bs.ts AS book_ts
        FROM recent_fills rf
        LEFT JOIN LATERAL (
            -- event_type='book' 만 양쪽 bid/ask 동시 보유 (price_change 는 한쪽씩).
            -- partial index be_book_asset_ts_idx 가 이 쿼리 가속.
            SELECT best_bid, best_ask, ts
            FROM book_events
            WHERE asset_id = rf.asset_id
              AND event_type = 'book'
              AND best_bid IS NOT NULL AND best_ask IS NOT NULL
              AND ts <= rf.ts + INTERVAL '5 seconds'
            ORDER BY ts DESC
            LIMIT 1
        ) bs ON true
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // Classification & aggregation
    let mut total = 0usize;
    let mut maker_bid = 0usize;
    let mut taker_ask = 0usize;
    let mut inside = 0usize;
    let mut outside = 0usize;
    let mut no_book = 0usize;
    let mut sum_dist_from_mid = 0.0f64;
    let mut maker_gross = 0.0f64;
    let mut taker_gross = 0.0f64;
    let mut inside_gross = 0.0f64;
    // 분류별 분포 (¢ 단위 distance from mid: -5..+5)
    let mut dist_buckets: std::collections::BTreeMap<i32, i64> = std::collections::BTreeMap::new();
    // 마켓별 maker % 계산용
    use std::collections::HashMap;
    let mut per_market: HashMap<String, (i64, i64, f64, Option<String>)> = HashMap::new(); // cid -> (total, maker, gross, title)

    let tick = 0.005; // half-cent tolerance

    for r in &rows {
        let cid: String = r.get("condition_id");
        let title: Option<String> = r.get("title");
        let price: f64 = r.get("price");
        let usdc: f64 = r.get("usdc_size");
        let bid: Option<f64> = r.get("best_bid");
        let ask: Option<f64> = r.get("best_ask");

        total += 1;
        match (bid, ask) {
            (Some(b), Some(a)) if a > b => {
                let mid = (a + b) / 2.0;
                let dist = price - mid;
                sum_dist_from_mid += dist;
                let bucket = ((dist * 100.0).round() as i32).clamp(-10, 10);
                *dist_buckets.entry(bucket).or_insert(0) += 1;

                let entry = per_market.entry(cid.clone()).or_insert((0, 0, 0.0, title.clone()));
                entry.0 += 1;
                entry.2 += usdc;

                if price <= b + tick {
                    maker_bid += 1;
                    maker_gross += usdc;
                    entry.1 += 1;
                } else if price >= a - tick {
                    taker_ask += 1;
                    taker_gross += usdc;
                } else if price > b + tick && price < a - tick {
                    inside += 1;
                    inside_gross += usdc;
                } else {
                    outside += 1;
                }
            }
            _ => {
                no_book += 1;
            }
        }
    }

    let with_book = (total - no_book) as f64;
    let avg_dist = if with_book > 0.0 { sum_dist_from_mid / with_book } else { 0.0 };
    let maker_pct = if with_book > 0.0 { maker_bid as f64 / with_book * 100.0 } else { 0.0 };
    let taker_pct = if with_book > 0.0 { taker_ask as f64 / with_book * 100.0 } else { 0.0 };
    let inside_pct = if with_book > 0.0 { inside as f64 / with_book * 100.0 } else { 0.0 };

    // Forensic baseline 의 0.75% sports maker rebate 가정으로 추정 rebate
    let est_rebate = maker_gross * 0.0075;

    // Per-market top by maker count
    let mut per_market_vec: Vec<serde_json::Value> = per_market.into_iter().map(|(cid, (n, mk, gross, title))| {
        serde_json::json!({
            "condition_id": cid,
            "title": title,
            "fills": n,
            "maker_fills": mk,
            "maker_pct": if n > 0 { mk as f64 / n as f64 * 100.0 } else { 0.0 },
            "gross_usdc": gross,
        })
    }).collect();
    per_market_vec.sort_by(|a, b|
        b["gross_usdc"].as_f64().unwrap_or(0.0)
        .partial_cmp(&a["gross_usdc"].as_f64().unwrap_or(0.0))
        .unwrap_or(std::cmp::Ordering::Equal));
    per_market_vec.truncate(30);

    // Distribution buckets as array
    let dist_array: Vec<serde_json::Value> = (-10i32..=10).map(|c| {
        serde_json::json!({ "cents_from_mid": c, "count": dist_buckets.get(&c).copied().unwrap_or(0) })
    }).collect();

    Ok(Json(serde_json::json!({
        "window_hours": hours,
        "sample": sample,
        "now": Utc::now(),
        "total_fills_scanned": total,
        "fills_with_book": total - no_book,
        "fills_no_book": no_book,
        "classification": {
            "maker_bid":    maker_bid,
            "taker_ask":    taker_ask,
            "inside_spread": inside,
            "outside":      outside,
            "maker_pct":    maker_pct,
            "taker_pct":    taker_pct,
            "inside_pct":   inside_pct,
        },
        "gross_usdc": {
            "maker": maker_gross,
            "taker": taker_gross,
            "inside": inside_gross,
            "total_with_book": maker_gross + taker_gross + inside_gross,
        },
        "avg_distance_from_mid": avg_dist,
        "estimated_rebate_usdc": est_rebate,
        "rebate_assumption_bps": 75,  // 0.75% sports
        "distance_distribution": dist_array,
        "top_markets_by_gross": per_market_vec,
    })))
}

/// MERGE / REDEEM / SPLIT 액션 최근 이력.
pub async fn actions_recent(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let rows = sqlx::query(r#"
        SELECT pa.action_id, pa.condition_id, pa.action_type,
               pa.size, pa.tx_hash, pa.realized_usdc, pa.triggered_at,
               pa.status,
               p.title, p.event_slug, p.outcome_name
        FROM position_actions pa
        LEFT JOIN LATERAL (
            SELECT title, event_slug, outcome_name
            FROM positions WHERE condition_id = pa.condition_id LIMIT 1
        ) p ON true
        ORDER BY pa.triggered_at DESC
        LIMIT 300
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let actions: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        serde_json::json!({
            "action_id": r.get::<i64, _>("action_id"),
            "condition_id": r.get::<String, _>("condition_id"),
            "action_type": r.get::<String, _>("action_type"),
            "size": r.get::<Option<f64>, _>("size"),
            "tx_hash": r.get::<Option<String>, _>("tx_hash"),
            "realized_usdc": r.get::<Option<f64>, _>("realized_usdc"),
            "triggered_at": r.get::<DateTime<Utc>, _>("triggered_at"),
            "status": r.get::<String, _>("status"),
            "title": r.get::<Option<String>, _>("title"),
            "event_slug": r.get::<Option<String>, _>("event_slug"),
            "outcome_name": r.get::<Option<String>, _>("outcome_name"),
        })
    }).collect();

    // 집계 by type
    let summary: (Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>, Option<f64>) =
        sqlx::query_as(r#"
            SELECT
              COUNT(*) FILTER (WHERE action_type='MERGE'),
              COUNT(*) FILTER (WHERE action_type='REDEEM'),
              COUNT(*) FILTER (WHERE action_type='SPLIT'),
              COALESCE(SUM(realized_usdc) FILTER (WHERE action_type='MERGE'), 0),
              COALESCE(SUM(realized_usdc) FILTER (WHERE action_type='REDEEM'), 0),
              COALESCE(SUM(realized_usdc) FILTER (WHERE action_type='SPLIT'), 0)
            FROM position_actions
        "#).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;

    Ok(Json(serde_json::json!({
        "actions": actions,
        "summary": {
            "merge_count":   summary.0.unwrap_or(0),
            "redeem_count":  summary.1.unwrap_or(0),
            "split_count":   summary.2.unwrap_or(0),
            "merge_usdc":    summary.3.unwrap_or(0.0),
            "redeem_usdc":   summary.4.unwrap_or(0.0),
            "split_usdc":    summary.5.unwrap_or(0.0),
        },
    })))
}

/// 정산 P&L 검증 — Forensic baseline (+$75K/일) 의 실시간 재현 여부.
/// 일별 분해: MERGE 페이아웃 + REDEEM 페이아웃 + 매수 비용 (gross_buy) + maker rebate 추정.
/// 누적 net = (payouts) - (cost) + (rebate) + (보유분 mark-to-market).
#[derive(Deserialize)]
pub struct RealizedPnlQ {
    #[serde(default = "default_rp_days")] pub days: i64,
}
fn default_rp_days() -> i64 { 30 }

pub async fn realized_pnl_breakdown(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RealizedPnlQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let days = q.days.clamp(1, 60);

    // 일별 액션 (MERGE/REDEEM/SPLIT) payouts
    let action_rows = sqlx::query(&format!(r#"
        SELECT date_trunc('day', triggered_at)::date AS day,
               action_type,
               COALESCE(SUM(realized_usdc), 0)::float8 AS payout,
               COUNT(*)::bigint AS n
        FROM position_actions
        WHERE triggered_at > NOW() - INTERVAL '{days} days'
        GROUP BY day, action_type
        ORDER BY day, action_type
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // 일별 fills (gross_buy = 매수 비용 outflow)
    let fill_rows = sqlx::query(&format!(r#"
        SELECT date_trunc('day', ts)::date AS day,
               COALESCE(SUM(usdc_size), 0)::float8 AS gross_buy,
               COUNT(*)::bigint AS n_fills
        FROM fills
        WHERE is_ours = false AND ts > NOW() - INTERVAL '{days} days'
        GROUP BY day
        ORDER BY day
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    use std::collections::BTreeMap;
    // day → (merge_payout, merge_n, redeem_payout, redeem_n, split_payout, split_n)
    let mut by_day: BTreeMap<chrono::NaiveDate, [f64; 6]> = BTreeMap::new();
    for r in &action_rows {
        let day: chrono::NaiveDate = r.get("day");
        let kind: String = r.get("action_type");
        let payout: f64 = r.get("payout");
        let n: i64 = r.get("n");
        let entry = by_day.entry(day).or_insert([0.0; 6]);
        match kind.as_str() {
            "MERGE"  => { entry[0] += payout; entry[1] += n as f64; }
            "REDEEM" => { entry[2] += payout; entry[3] += n as f64; }
            "SPLIT"  => { entry[4] += payout; entry[5] += n as f64; }
            _ => {}
        }
    }
    // gross_buy per day
    let mut buy_by_day: BTreeMap<chrono::NaiveDate, (f64, i64)> = BTreeMap::new();
    for r in &fill_rows {
        let day: chrono::NaiveDate = r.get("day");
        let gross: f64 = r.get("gross_buy");
        let n: i64 = r.get("n_fills");
        buy_by_day.insert(day, (gross, n));
    }

    // 두 데이터 source 의 윈도우 분리 — actions backfill (10-30d) vs fills (17h~)
    // 같은 observed_days 로 나누면 daily 평균이 왜곡됨.
    let mut all_days: std::collections::BTreeSet<chrono::NaiveDate> = std::collections::BTreeSet::new();
    for d in by_day.keys() { all_days.insert(*d); }
    for d in buy_by_day.keys() { all_days.insert(*d); }

    // 각 데이터 source 의 실제 span 추정
    let actions_days = if !by_day.is_empty() {
        // 첫 액션 ~ 마지막 액션 (일 단위 차이 + 1)
        let min_d = *by_day.keys().min().unwrap();
        let max_d = *by_day.keys().max().unwrap();
        ((max_d - min_d).num_days() + 1) as f64
    } else { 0.0 };
    // fills 는 collector 시작 이후 시간 단위 — Postgres 에서 직접 days 계산.
    // EXTRACT(EPOCH FROM interval) 은 NUMERIC 반환 → FLOAT8 캐스트 필요.
    let fills_days_row: (Option<f64>,) = sqlx::query_as(
        &format!(r#"SELECT (EXTRACT(EPOCH FROM (MAX(ts) - MIN(ts))) / 86400.0)::float8
                   FROM fills WHERE is_ours=false AND ts > NOW() - INTERVAL '{days} days'"#)
    ).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;
    let fills_days = fills_days_row.0.unwrap_or(0.0).max(0.001);

    // Maker rebate estimate — 일별 maker fills × 0.75%.
    // 단순화: 모든 fills 이 maker 라고 가정 (mid-price tracking 으로 87% 확인됨).
    // 좀더 정확하려면 fills × book LATERAL 매칭 비율 적용 필요 — 여기선 0.87 보정.
    let rebate_pct: f64 = 0.0075;  // 0.75% sports
    let maker_share: f64 = 0.87;   // 관측값

    // 보유분 현재 mark-to-market
    let cur_mtm: (Option<f64>,) = sqlx::query_as(
        "SELECT COALESCE(SUM(current_value),0)::float8 FROM positions WHERE size > 0"
    ).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;
    let current_holdings_mtm = cur_mtm.0.unwrap_or(0.0);

    // 누적
    let mut total_merge_payout = 0.0;
    let mut total_redeem_payout = 0.0;
    let mut total_split_payout = 0.0;
    let mut total_gross_buy = 0.0;
    let mut total_merge_n = 0i64;
    let mut total_redeem_n = 0i64;
    let mut total_split_n = 0i64;
    let mut total_n_fills = 0i64;

    let daily: Vec<serde_json::Value> = all_days.iter().map(|day| {
        let actions = by_day.get(day).copied().unwrap_or([0.0; 6]);
        let (gross_buy, n_fills) = buy_by_day.get(day).copied().unwrap_or((0.0, 0));
        let rebate_est = gross_buy * maker_share * rebate_pct;
        let payout_total = actions[0] + actions[2] + actions[4];
        let day_net = payout_total + rebate_est - gross_buy;

        total_merge_payout += actions[0];
        total_redeem_payout += actions[2];
        total_split_payout += actions[4];
        total_gross_buy += gross_buy;
        total_merge_n += actions[1] as i64;
        total_redeem_n += actions[3] as i64;
        total_split_n += actions[5] as i64;
        total_n_fills += n_fills;

        serde_json::json!({
            "day": day.to_string(),
            "merge_payout":  actions[0],
            "merge_n":       actions[1] as i64,
            "redeem_payout": actions[2],
            "redeem_n":      actions[3] as i64,
            "split_payout":  actions[4],
            "split_n":       actions[5] as i64,
            "gross_buy":     gross_buy,
            "n_fills":       n_fills,
            "rebate_est":    rebate_est,
            "day_net_est":   day_net,
        })
    }).collect();

    let total_payouts = total_merge_payout + total_redeem_payout + total_split_payout;
    let total_rebate_est = total_gross_buy * maker_share * rebate_pct;
    // 관측 가능한 net = (payouts - cost) + rebate + 현재 보유분 MTM
    // 다만 cost 와 payouts 의 매칭이 정확하지 않음 (cost 는 27% 만 우리 관측, payouts 는 30일)
    let net_observed = total_payouts + total_rebate_est + current_holdings_mtm - total_gross_buy;

    // Forensic baseline (21일, 2026-04-28~05-18) — 비교용 상수
    let forensic = serde_json::json!({
        "window_days":    21,
        "merge_locked":   983_946.0,    // MERGE 의 PROFIT 부분 (payout - cost)
        "residual_pnl":   334_072.0,
        "maker_rebate":   297_066.0,
        "trade_pnl":      1_291_425.0,  // 1차 trade-level 이익
        "net":            1_588_490.0,
        "daily_avg":      75_642.0,
        "gross_notional": 39_885_153.0,
    });

    // 관측 일수 — 표시용 (전체 union 일수)
    let observed_days = all_days.len() as f64;

    // 윈도우 분리: 각 component 의 실제 window 로 daily 평균 계산.
    // 잘못된 통합 평균을 피함 (이전: payouts 10d 와 gross_buy 17h 를 같은 10d 로 나눠 왜곡).
    let merge_daily   = if actions_days > 0.0 { total_merge_payout  / actions_days } else { 0.0 };
    let redeem_daily  = if actions_days > 0.0 { total_redeem_payout / actions_days } else { 0.0 };
    let payouts_daily = if actions_days > 0.0 { total_payouts       / actions_days } else { 0.0 };
    let rebate_daily  = if fills_days > 0.0   { total_rebate_est    / fills_days   } else { 0.0 };
    let buy_daily     = if fills_days > 0.0   { total_gross_buy     / fills_days   } else { 0.0 };

    // MERGE-locked profit 추정 — RN1 의 평균 pair sum_avg ≈ 0.90~0.95 → margin 5-10%.
    // 우리 데이터로 정확한 margin 추정 시 보정 필요. 단순화 5%.
    let est_merge_margin_pct: f64 = 5.0;
    let merge_locked_observed = total_merge_payout * (est_merge_margin_pct / 100.0);
    let merge_locked_daily = merge_locked_observed / actions_days.max(0.001);

    // Projected net — apples-to-apples 만 합산
    // = rebate_daily (정확) + merge_locked_daily (추정) - loss buffer (forensic 의 -7%)
    // REDEEM gross 는 cost basis 매칭 불가로 제외.
    let projected_daily_gross = rebate_daily + merge_locked_daily;
    let loss_buffer_pct: f64 = 0.07;   // forensic 의 21d 손실률
    let projected_daily_net = projected_daily_gross * (1.0 - loss_buffer_pct);

    // 일치도 — 정직한 비교만
    let gross_buy_vs_baseline_pct = if 39_885_153.0 > 0.0 {
        buy_daily / (39_885_153.0 / 21.0) * 100.0
    } else { 0.0 };
    let rebate_vs_baseline_pct = if 297_066.0 > 0.0 {
        rebate_daily / (297_066.0 / 21.0) * 100.0
    } else { 0.0 };
    let merge_locked_vs_baseline_pct = if 983_946.0 > 0.0 {
        merge_locked_daily / (983_946.0 / 21.0) * 100.0
    } else { 0.0 };
    let projected_net_vs_baseline_pct = if 75_642.0 > 0.0 {
        projected_daily_net / 75_642.0 * 100.0
    } else { 0.0 };

    Ok(Json(serde_json::json!({
        "window_days": days,
        "observed_days": observed_days as i64,
        "actions_days":  actions_days,        // payouts / actions window (10d backfill 등)
        "fills_days":    fills_days,          // fills / gross_buy window (collector 시작 이후)
        "now": Utc::now(),
        "daily": daily,
        "totals": {
            "merge_payout":          total_merge_payout,
            "merge_count":           total_merge_n,
            "redeem_payout":         total_redeem_payout,
            "redeem_count":          total_redeem_n,
            "split_payout":          total_split_payout,
            "split_count":           total_split_n,
            "all_payouts":           total_payouts,
            "gross_buy":             total_gross_buy,
            "n_fills":               total_n_fills,
            "rebate_estimate":       total_rebate_est,
            "current_holdings_mtm":  current_holdings_mtm,
            // net_observed 는 시간 윈도우 mismatch 로 inflated — 제거.
            // 대신 윈도우 정규화된 projected daily 사용.
            "merge_locked_est":      merge_locked_observed,
            "merge_margin_est_pct":  est_merge_margin_pct,
        },
        "daily_averages": {
            "merge_payout":     merge_daily,
            "redeem_payout":    redeem_daily,
            "all_payouts":      payouts_daily,
            "rebate":           rebate_daily,
            "gross_buy":        buy_daily,
            "projected_net":    projected_daily_net,
        },
        "forensic_baseline": forensic,
        "verification": {
            // 정직한 비교만 — apples-to-apples
            "gross_buy_vs_baseline_pct":     gross_buy_vs_baseline_pct,
            "rebate_vs_baseline_pct":        rebate_vs_baseline_pct,
            "merge_locked_vs_baseline_pct":  merge_locked_vs_baseline_pct,
            "projected_net_vs_baseline_pct": projected_net_vs_baseline_pct,
            "rebate_assumption_bps":         (rebate_pct * 10000.0) as i64,
            "maker_share_assumption_pct":    (maker_share * 100.0) as i64,
            "merge_margin_assumption_pct":   est_merge_margin_pct,
            "loss_buffer_pct":               loss_buffer_pct * 100.0,
            "caveat":                         format!("REDEEM gross 와 forensic residual_pnl 은 다른 metric (gross vs net) — 직접 비교 불가. window mismatch: payouts={:.1}d backfill, fills={:.2}d only.", actions_days, fills_days),
        },
    })))
}

/// (1) Lifecycle pattern aggregate — 여러 마켓의 lifecycle 메트릭 분포로
/// "RN1 의 표준 진입 패턴" 추출. first_leg side bias, time_to_second 히스토그램,
/// final_balance 분포 + 패턴 별 대표 마켓 sample.
#[derive(Deserialize)]
pub struct LifecyclePatternsQ {
    #[serde(default = "default_lp_hours")] pub hours: i64,
}
fn default_lp_hours() -> i64 { 24 }

pub async fn lifecycle_patterns(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LifecyclePatternsQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let hours = q.hours.clamp(1, 168);

    // 마켓별 first_leg / second_leg / 최종 leg shares 계산
    let rows = sqlx::query(&format!(r#"
        WITH leg_meta AS (
            SELECT condition_id, outcome_index,
                   MIN(ts) AS first_ts,
                   SUM(size)::float8 AS shares,
                   SUM(usdc_size)::float8 AS usdc,
                   COUNT(*)::bigint AS fills,
                   (ARRAY_AGG(price ORDER BY ts ASC))[1]::float8 AS first_price,
                   AVG(price)::float8 AS avg_price,
                   MAX(title) AS title,
                   MAX(event_slug) AS event_slug
            FROM fills
            WHERE is_ours = false AND ts > NOW() - INTERVAL '{hours} hours'
            GROUP BY condition_id, outcome_index
        ),
        market_legs AS (
            SELECT condition_id,
                   COUNT(*) AS sides_traded,
                   MIN(first_ts) AS first_ts,
                   -- 두 번째로 시작된 leg ts (sides 2+ 일 때)
                   (SELECT first_ts FROM leg_meta lm2
                     WHERE lm2.condition_id = lm.condition_id
                     ORDER BY first_ts ASC OFFSET 1 LIMIT 1) AS second_ts,
                   (SELECT outcome_index FROM leg_meta lm2
                     WHERE lm2.condition_id = lm.condition_id
                     ORDER BY first_ts ASC LIMIT 1) AS first_leg,
                   (SELECT first_price FROM leg_meta lm2
                     WHERE lm2.condition_id = lm.condition_id
                     ORDER BY first_ts ASC LIMIT 1) AS first_leg_price,
                   (SELECT avg_price FROM leg_meta lm2
                     WHERE lm2.condition_id = lm.condition_id
                     ORDER BY first_ts ASC LIMIT 1) AS first_leg_avg,
                   (SELECT title FROM leg_meta lm2
                     WHERE lm2.condition_id = lm.condition_id LIMIT 1) AS title,
                   (SELECT event_slug FROM leg_meta lm2
                     WHERE lm2.condition_id = lm.condition_id LIMIT 1) AS event_slug,
                   SUM(CASE WHEN outcome_index=0 THEN shares END)::float8 AS shares_0,
                   SUM(CASE WHEN outcome_index=1 THEN shares END)::float8 AS shares_1,
                   SUM(shares)::float8 AS total_shares,
                   SUM(usdc)::float8 AS total_usdc,
                   SUM(fills)::bigint AS total_fills
            FROM leg_meta lm
            GROUP BY condition_id
        )
        SELECT * FROM market_legs
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // 분포 buckets
    let mut first_leg_0 = 0i64;
    let mut first_leg_1 = 0i64;
    let mut single_leg = 0i64;
    let mut two_sided = 0i64;

    // time_to_second buckets (분)
    let buckets_t2s = ["<1m", "1-5m", "5-15m", "15-30m", "30-60m", "1-2h", "2h+"];
    let mut t2s_counts: Vec<i64> = vec![0; buckets_t2s.len()];
    let mut t2s_values: Vec<f64> = Vec::new();

    // balance buckets (%)
    let buckets_bal = ["<10%", "10-30%", "30-50%", "50-70%", "70-90%", "90-100%"];
    let mut bal_counts: Vec<i64> = vec![0; buckets_bal.len()];
    let mut bal_values: Vec<f64> = Vec::new();

    // first_leg favorite vs underdog (price>0.5 = fav side)
    let mut fl_favorite = 0i64;
    let mut fl_underdog = 0i64;
    let mut fl_even = 0i64;

    // 대표 마켓 sample (각 패턴 별 1-2 개)
    let mut representatives: Vec<serde_json::Value> = Vec::new();
    // 산점도용 (time_to_second vs balance)
    let mut scatter: Vec<serde_json::Value> = Vec::new();

    for r in &rows {
        let cid: String = r.get("condition_id");
        let title: Option<String> = r.get("title");
        let event_slug: Option<String> = r.get("event_slug");
        let sides_traded: i64 = r.get("sides_traded");
        let first_leg: Option<i16> = r.get("first_leg");
        let first_leg_avg: Option<f64> = r.get("first_leg_avg");
        let first_leg_price: Option<f64> = r.get("first_leg_price");
        let first_ts: Option<DateTime<Utc>> = r.get("first_ts");
        let second_ts: Option<DateTime<Utc>> = r.get("second_ts");
        let shares_0: Option<f64> = r.get("shares_0");
        let shares_1: Option<f64> = r.get("shares_1");
        let total_usdc: f64 = r.get("total_usdc");
        let total_fills: i64 = r.get("total_fills");

        // first_leg bias
        match first_leg {
            Some(0) => first_leg_0 += 1,
            Some(1) => first_leg_1 += 1,
            _ => {}
        }
        if sides_traded < 2 { single_leg += 1; } else { two_sided += 1; }

        // favorite/underdog
        if let Some(p) = first_leg_avg {
            if p > 0.55 { fl_favorite += 1; }
            else if p < 0.45 { fl_underdog += 1; }
            else { fl_even += 1; }
        }

        // time to second leg
        let t2s = match (first_ts, second_ts) {
            (Some(f), Some(s)) => Some((s - f).num_seconds() as f64 / 60.0),
            _ => None,
        };
        if let Some(t) = t2s {
            t2s_values.push(t);
            let idx = if t < 1.0 { 0 }
                else if t < 5.0 { 1 }
                else if t < 15.0 { 2 }
                else if t < 30.0 { 3 }
                else if t < 60.0 { 4 }
                else if t < 120.0 { 5 }
                else { 6 };
            t2s_counts[idx] += 1;
        }

        // balance ratio
        let bal = match (shares_0, shares_1) {
            (Some(a), Some(b)) if a > 0.0 && b > 0.0 => Some(a.min(b) / a.max(b)),
            _ => None,
        };
        if let Some(b) = bal {
            bal_values.push(b);
            let bp = b * 100.0;
            let idx = if bp < 10.0 { 0 }
                else if bp < 30.0 { 1 }
                else if bp < 50.0 { 2 }
                else if bp < 70.0 { 3 }
                else if bp < 90.0 { 4 }
                else { 5 };
            bal_counts[idx] += 1;
        }

        // 대표 마켓 후보 — gross 상위로 30 markets sample
        if representatives.len() < 30 {
            representatives.push(serde_json::json!({
                "condition_id": cid.clone(),
                "title": title.clone(),
                "event_slug": event_slug.clone(),
                "first_leg": first_leg,
                "first_leg_price": first_leg_price,
                "time_to_second_min": t2s,
                "balance_ratio": bal,
                "total_usdc": total_usdc,
                "total_fills": total_fills,
                "sides_traded": sides_traded,
            }));
        }
        if let (Some(t), Some(b)) = (t2s, bal) {
            scatter.push(serde_json::json!({
                "cid": cid,
                "title": title,
                "t2s_min": t,
                "balance": b,
                "gross": total_usdc,
            }));
        }
    }

    // Top representatives by gross
    representatives.sort_by(|a, b| {
        b["total_usdc"].as_f64().unwrap_or(0.0)
            .partial_cmp(&a["total_usdc"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    fn median(v: &mut Vec<f64>) -> Option<f64> {
        if v.is_empty() { return None; }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(v[v.len() / 2])
    }
    let mut t2s_sorted = t2s_values.clone();
    let mut bal_sorted = bal_values.clone();

    Ok(Json(serde_json::json!({
        "window_hours": hours,
        "total_markets": rows.len(),
        "first_leg_distribution": {
            "outcome_0": first_leg_0,
            "outcome_1": first_leg_1,
            "single_leg_only": single_leg,
            "two_sided": two_sided,
            "two_sided_pct": if rows.is_empty() { 0.0 } else { two_sided as f64 / rows.len() as f64 * 100.0 },
        },
        "first_leg_classification": {
            "favorite (avg>0.55)": fl_favorite,
            "underdog (avg<0.45)": fl_underdog,
            "even (0.45-0.55)":    fl_even,
        },
        "time_to_second_buckets": buckets_t2s.iter().zip(t2s_counts.iter()).map(|(l, c)| {
            serde_json::json!({ "label": l, "count": c })
        }).collect::<Vec<_>>(),
        "balance_buckets": buckets_bal.iter().zip(bal_counts.iter()).map(|(l, c)| {
            serde_json::json!({ "label": l, "count": c })
        }).collect::<Vec<_>>(),
        "stats": {
            "median_time_to_second_min": median(&mut t2s_sorted),
            "median_balance_ratio":      median(&mut bal_sorted),
            "n_with_t2s":                t2s_values.len(),
            "n_with_balance":            bal_values.len(),
        },
        "scatter": scatter,
        "representatives": representatives,
    })))
}

/// (2) First-leg context — 각 마켓의 첫 leg 의 첫 fill price 와 OTHER leg 의 첫 fill price
/// 비교 → RN1 이 cheaper(underdog) / expensive(favorite) 어느 쪽부터 시작하는지 정량화.
/// book_events 보존 윈도우 (7d) 안의 첫 fill 만 book mid 까지 LATERAL lookup.
pub async fn first_leg_context(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LifecyclePatternsQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let hours = q.hours.clamp(1, 168);

    // 마켓별 leg 0/1 의 첫 fill price + ts 비교
    let rows = sqlx::query(&format!(r#"
        WITH leg_firsts AS (
            SELECT condition_id, outcome_index,
                   MIN(ts) AS first_ts,
                   (ARRAY_AGG(price ORDER BY ts ASC))[1]::float8 AS first_price,
                   (ARRAY_AGG(size ORDER BY ts ASC))[1]::float8 AS first_size
            FROM fills
            WHERE is_ours = false AND ts > NOW() - INTERVAL '{hours} hours'
            GROUP BY condition_id, outcome_index
        ),
        market_first_leg AS (
            SELECT DISTINCT ON (condition_id)
                condition_id, outcome_index AS first_leg, first_ts, first_price
            FROM leg_firsts ORDER BY condition_id, first_ts
        )
        SELECT
            mfl.condition_id, mfl.first_leg, mfl.first_ts, mfl.first_price AS self_first_price,
            other.first_price AS other_first_price,
            other.first_ts    AS other_first_ts,
            other.outcome_index AS other_leg,
            m.game_start,
            f0.title, f0.event_slug, f0.icon
        FROM market_first_leg mfl
        LEFT JOIN leg_firsts other
            ON other.condition_id = mfl.condition_id AND other.outcome_index != mfl.first_leg
        LEFT JOIN markets m ON m.condition_id = mfl.condition_id
        LEFT JOIN LATERAL (
            SELECT title, event_slug, icon FROM fills
            WHERE condition_id = mfl.condition_id LIMIT 1
        ) f0 ON true
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let mut fl_was_cheaper = 0i64;
    let mut fl_was_expensive = 0i64;
    let mut fl_was_equal = 0i64;
    let mut single_leg = 0i64;
    let mut markets: Vec<serde_json::Value> = Vec::new();

    // 분포: first_leg first_price 분포 (10¢ buckets)
    let mut price_buckets: Vec<i64> = vec![0; 10];

    // 시간 갭 분포 (second_leg first_ts - first_leg first_ts)
    let mut gap_minutes: Vec<f64> = Vec::new();

    for r in &rows {
        let cid: String = r.get("condition_id");
        let first_leg: i16 = r.get("first_leg");
        let first_ts: DateTime<Utc> = r.get("first_ts");
        let self_first: f64 = r.get("self_first_price");
        let other_first: Option<f64> = r.get("other_first_price");
        let other_ts: Option<DateTime<Utc>> = r.get("other_first_ts");
        let other_leg: Option<i16> = r.get("other_leg");
        let game_start: Option<DateTime<Utc>> = r.get("game_start");
        let title: Option<String> = r.get("title");
        let event_slug: Option<String> = r.get("event_slug");
        let icon: Option<String> = r.get("icon");

        // price bucket
        let bucket_idx = ((self_first * 10.0).floor() as usize).min(9);
        price_buckets[bucket_idx] += 1;

        // 비교
        let cheaper = match other_first {
            None => { single_leg += 1; None }
            Some(o) if (self_first - o).abs() < 0.01 => { fl_was_equal += 1; Some(false) }
            Some(o) if self_first < o => { fl_was_cheaper += 1; Some(true) }
            Some(o) if self_first > o => { fl_was_expensive += 1; Some(false) }
            _ => None,
        };

        // gap
        let gap_min = match (first_ts, other_ts) {
            (f, Some(o)) => Some((o - f).num_seconds() as f64 / 60.0),
            _ => None,
        };
        if let Some(g) = gap_min { gap_minutes.push(g); }

        let secs_from_game = game_start.map(|gs| (first_ts - gs).num_seconds() as f64 / 60.0);

        if markets.len() < 60 {
            markets.push(serde_json::json!({
                "condition_id": cid,
                "title": title,
                "event_slug": event_slug,
                "icon": icon,
                "first_leg": first_leg,
                "other_leg": other_leg,
                "first_ts":  first_ts,
                "other_ts":  other_ts,
                "self_first_price":  self_first,
                "other_first_price": other_first,
                "implied_sum":       other_first.map(|o| self_first + o),  // 1.0 에 가까우면 binary 정상
                "gap_min":           gap_min,
                "first_leg_cheaper": cheaper,
                "secs_from_game_min": secs_from_game,
            }));
        }
    }

    // sort markets by secs_from_game_min (정렬 일관성)
    markets.sort_by(|a, b| {
        a["secs_from_game_min"].as_f64().unwrap_or(f64::INFINITY)
            .partial_cmp(&b["secs_from_game_min"].as_f64().unwrap_or(f64::INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total = rows.len() as f64;
    fn median_f(v: &mut Vec<f64>) -> Option<f64> {
        if v.is_empty() { return None; }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(v[v.len() / 2])
    }
    let median_gap = median_f(&mut gap_minutes);

    Ok(Json(serde_json::json!({
        "window_hours": hours,
        "total_markets": rows.len(),
        "first_leg_side_selection": {
            "cheaper_side":    fl_was_cheaper,
            "expensive_side":  fl_was_expensive,
            "equal":           fl_was_equal,
            "single_leg_only": single_leg,
            "cheaper_pct":     if total > 0.0 { fl_was_cheaper as f64 / total * 100.0 } else { 0.0 },
        },
        "first_leg_price_distribution": (0..10).map(|i| {
            serde_json::json!({
                "label": format!("{}-{}¢", i*10, (i+1)*10),
                "count": price_buckets[i as usize],
            })
        }).collect::<Vec<_>>(),
        "second_leg_gap_median_min": median_gap,
        "markets": markets,
    })))
}

/// (3) Quote update timeline — 단일 마켓의 book_events (price_change 포함) 와
/// RN1 fills 를 시간 정렬 통합. quote 갱신 메커니즘 정밀 분석.
pub async fn quote_timeline(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(cid): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // outcome → asset 매핑
    let assets: Vec<(String, i16)> = sqlx::query_as(r#"
        SELECT asset_id, outcome_index FROM asset_to_condition WHERE condition_id = $1
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    if assets.is_empty() {
        return Ok(Json(serde_json::json!({ "error": "no asset mapping", "condition_id": cid })));
    }

    // book events — 모든 type 중 best_bid/ask 어느 한쪽이라도 존재하는 것만 (price_change 도 포함)
    let book_rows = sqlx::query(r#"
        SELECT ts, asset_id, event_type, best_bid, best_ask
        FROM book_events
        WHERE asset_id = ANY($1)
          AND (best_bid IS NOT NULL OR best_ask IS NOT NULL)
        ORDER BY ts ASC
        LIMIT 5000
    "#).bind(assets.iter().map(|(a,_)| a.clone()).collect::<Vec<_>>())
        .fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // 자산 ↔ outcome 매핑
    let asset_idx: std::collections::HashMap<String, i16> = assets.iter().cloned()
        .map(|(a, i)| (a, i)).collect();

    let book_events: Vec<serde_json::Value> = book_rows.into_iter().map(|r| {
        let asset_id: String = r.get("asset_id");
        let outcome_index = asset_idx.get(&asset_id).copied().unwrap_or(-1);
        let ts: DateTime<Utc> = r.get("ts");
        let event_type: String = r.get("event_type");
        let best_bid: Option<f64> = r.get("best_bid");
        let best_ask: Option<f64> = r.get("best_ask");
        serde_json::json!({
            "ts": ts,
            "outcome_index": outcome_index,
            "event_type": event_type,
            "best_bid": best_bid,
            "best_ask": best_ask,
        })
    }).collect();

    // fills
    let fill_rows = sqlx::query(r#"
        SELECT ts, outcome_index, price, size, usdc_size
        FROM fills
        WHERE condition_id = $1 AND is_ours = false
        ORDER BY ts ASC
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let fills: Vec<serde_json::Value> = fill_rows.into_iter().map(|r| {
        serde_json::json!({
            "ts": r.get::<DateTime<Utc>, _>("ts"),
            "outcome_index": r.get::<i16, _>("outcome_index"),
            "price": r.get::<f64, _>("price"),
            "size": r.get::<f64, _>("size"),
            "usdc_size": r.get::<f64, _>("usdc_size"),
        })
    }).collect();

    Ok(Json(serde_json::json!({
        "condition_id": cid,
        "assets": assets.iter().map(|(a, i)| serde_json::json!({"asset_id": a, "outcome_index": i})).collect::<Vec<_>>(),
        "book_events": book_events,
        "fills": fills,
        "book_event_count": book_events.len(),
        "fill_count": fills.len(),
    })))
}

/// 단일 마켓의 fill lifecycle replay — 어떤 leg 부터, 언제, 어떻게 two-side 완성하는지.
///
/// 반환:
///  · market metadata (event_slug, title, game_start, end_date, outcomes)
///  · fills 시간순 (각 row: ts, outcome_index, price, size, usdc,
///    누적 leg shares/usdc, best_bid/ask 직전, mid, distance, classification)
///  · per-leg summary (first_ts, total_shares, total_usdc, avg_price)
///  · lifecycle 단계 분석 (first_leg / time_to_second_leg / balance_progression)
pub async fn market_lifecycle(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(cid): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 마켓 메타
    let meta_row = sqlx::query(r#"
        SELECT m.condition_id, m.question, m.slug, m.event_slug, m.outcomes,
               m.icon, m.game_start, m.end_date, m.uma_status,
               m.best_bid, m.best_ask, m.volume_num, m.neg_risk
        FROM markets m
        WHERE m.condition_id = $1
    "#).bind(&cid).fetch_optional(&state.pool).await.map_err(ErrorBody::new)?;

    let meta = if let Some(r) = meta_row {
        let outcomes: serde_json::Value = r.get("outcomes");
        serde_json::json!({
            "condition_id": r.get::<String, _>("condition_id"),
            "question":     r.get::<Option<String>, _>("question"),
            "slug":         r.get::<String, _>("slug"),
            "event_slug":   r.get::<Option<String>, _>("event_slug"),
            "outcomes":     outcomes,
            "icon":         r.get::<Option<String>, _>("icon"),
            "game_start":   r.get::<Option<DateTime<Utc>>, _>("game_start"),
            "end_date":     r.get::<Option<DateTime<Utc>>, _>("end_date"),
            "uma_status":   r.get::<Option<String>, _>("uma_status"),
            "best_bid":     r.get::<Option<f64>, _>("best_bid"),
            "best_ask":     r.get::<Option<f64>, _>("best_ask"),
            "volume_num":   r.get::<Option<f64>, _>("volume_num"),
            "neg_risk":     r.get::<bool, _>("neg_risk"),
        })
    } else {
        serde_json::json!({ "condition_id": cid, "outcomes": serde_json::Value::Null })
    };

    // 모든 fills 시간순 + book lateral
    let rows = sqlx::query(r#"
        SELECT f.ts, f.outcome_index, f.outcome_name, f.price, f.size, f.usdc_size,
               f.side, f.tx_hash,
               bs.best_bid, bs.best_ask, bs.ts AS book_ts
        FROM fills f
        LEFT JOIN asset_to_condition atc
               ON atc.condition_id = f.condition_id AND atc.outcome_index = f.outcome_index
        LEFT JOIN LATERAL (
            SELECT best_bid, best_ask, ts
            FROM book_events
            WHERE asset_id = atc.asset_id
              AND event_type = 'book'
              AND best_bid IS NOT NULL AND best_ask IS NOT NULL
              AND ts <= f.ts + INTERVAL '5 seconds'
            ORDER BY ts DESC LIMIT 1
        ) bs ON true
        WHERE f.condition_id = $1 AND f.is_ours = false
        ORDER BY f.ts ASC
    "#).bind(&cid).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // 누적 + 분류 계산
    let game_start: Option<DateTime<Utc>> = meta["game_start"].as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));

    let mut cum_shares: std::collections::HashMap<i16, f64> = std::collections::HashMap::new();
    let mut cum_usdc: std::collections::HashMap<i16, f64> = std::collections::HashMap::new();
    let mut first_ts: std::collections::HashMap<i16, DateTime<Utc>> = std::collections::HashMap::new();
    let mut leg_count: std::collections::HashMap<i16, i64> = std::collections::HashMap::new();
    let mut leg_sum_price_weighted: std::collections::HashMap<i16, f64> = std::collections::HashMap::new();

    let tick = 0.005;
    let fills: Vec<serde_json::Value> = rows.iter().map(|r| {
        let ts: DateTime<Utc> = r.get("ts");
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: Option<String> = r.get("outcome_name");
        let price: f64 = r.get("price");
        let size: f64 = r.get("size");
        let usdc: f64 = r.get("usdc_size");
        let side: String = r.get("side");
        let tx_hash: Option<String> = r.get("tx_hash");
        let best_bid: Option<f64> = r.get("best_bid");
        let best_ask: Option<f64> = r.get("best_ask");
        let book_ts: Option<DateTime<Utc>> = r.get("book_ts");

        // 누적 업데이트
        *cum_shares.entry(outcome_index).or_insert(0.0) += size;
        *cum_usdc.entry(outcome_index).or_insert(0.0) += usdc;
        *leg_count.entry(outcome_index).or_insert(0) += 1;
        *leg_sum_price_weighted.entry(outcome_index).or_insert(0.0) += price * size;
        first_ts.entry(outcome_index).or_insert(ts);

        // book + 분류
        let (mid, dist, classification) = match (best_bid, best_ask) {
            (Some(b), Some(a)) if a > b => {
                let m = (a + b) / 2.0;
                let d = price - m;
                let c = if price <= b + tick { "maker" }
                        else if price >= a - tick { "taker" }
                        else { "inside" };
                (Some(m), Some(d), c)
            }
            _ => (None, None, "no-book"),
        };

        let secs_since_game = game_start.map(|gs| (ts - gs).num_seconds() as f64 / 60.0);

        // 누적 leg 0 / 1 (binary 가정; 다른 idx 이면 null)
        let cum0 = cum_shares.get(&0).copied().unwrap_or(0.0);
        let cum1 = cum_shares.get(&1).copied().unwrap_or(0.0);
        let cum_total = cum0 + cum1;
        let balance_ratio = if cum_total > 0.0 {
            (cum0.min(cum1) / cum0.max(cum1)).min(1.0)
        } else { 0.0 };

        serde_json::json!({
            "ts": ts,
            "secs_from_game_min": secs_since_game,
            "outcome_index": outcome_index,
            "outcome_name": outcome_name,
            "side": side,
            "price": price,
            "size": size,
            "usdc_size": usdc,
            "tx_hash": tx_hash,
            "best_bid": best_bid,
            "best_ask": best_ask,
            "mid": mid,
            "dist_from_mid": dist,
            "classification": classification,
            "cum_shares_outcome": cum_shares.get(&outcome_index).copied().unwrap_or(0.0),
            "cum_usdc_outcome":   cum_usdc.get(&outcome_index).copied().unwrap_or(0.0),
            "cum_shares_0": cum0,
            "cum_shares_1": cum1,
            "balance_ratio": balance_ratio,
        })
    }).collect();

    // Per-leg summary
    let mut legs: Vec<serde_json::Value> = Vec::new();
    let mut keys: Vec<i16> = leg_count.keys().copied().collect();
    keys.sort();
    for k in keys {
        let shares = cum_shares.get(&k).copied().unwrap_or(0.0);
        let usdc = cum_usdc.get(&k).copied().unwrap_or(0.0);
        let avg = if shares > 0.0 { leg_sum_price_weighted.get(&k).copied().unwrap_or(0.0) / shares } else { 0.0 };
        legs.push(serde_json::json!({
            "outcome_index": k,
            "fills":  leg_count.get(&k).copied().unwrap_or(0),
            "shares": shares,
            "usdc":   usdc,
            "avg_price": avg,
            "first_ts": first_ts.get(&k),
        }));
    }

    // Lifecycle 단계 분석
    // first_leg = 가장 먼저 fill 들어간 outcome
    let first_leg = first_ts.iter().min_by_key(|(_, t)| **t).map(|(idx, _)| *idx);
    // second_leg 시작 시각 - first 시각 (분)
    let time_to_second_leg_min: Option<f64> = if first_ts.len() >= 2 {
        let mut ts_sorted: Vec<DateTime<Utc>> = first_ts.values().copied().collect();
        ts_sorted.sort();
        Some((ts_sorted[1] - ts_sorted[0]).num_seconds() as f64 / 60.0)
    } else { None };
    let two_sided = first_ts.len() >= 2;

    let final_balance_ratio = if let (Some(s0), Some(s1)) = (cum_shares.get(&0), cum_shares.get(&1)) {
        if s0.max(*s1) > 0.0 { Some(s0.min(*s1) / s0.max(*s1)) } else { None }
    } else { None };

    let total_fills = fills.len();
    let active_minutes = if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
        let f_ts: DateTime<Utc> = first.get("ts");
        let l_ts: DateTime<Utc> = last.get("ts");
        Some((l_ts - f_ts).num_seconds() as f64 / 60.0)
    } else { None };

    Ok(Json(serde_json::json!({
        "meta": meta,
        "fills": fills,
        "legs": legs,
        "total_fills": total_fills,
        "active_minutes": active_minutes,
        "lifecycle": {
            "first_leg":            first_leg,
            "time_to_second_leg_min": time_to_second_leg_min,
            "two_sided":            two_sided,
            "final_balance_ratio":  final_balance_ratio,
        },
    })))
}

/// 종목별 (slug prefix) 진입 패턴 브레이크다운 — RN1 의 universe 분포 + 종목 sweet spot 확인.
/// 각 종목별 진입 timing, inter-fill, two_sided %, fills per market 등 구분 비교.
#[derive(Deserialize)]
pub struct PerSportQ {
    #[serde(default = "default_ps_hours")]
    pub hours: i64,
}
fn default_ps_hours() -> i64 { 24 }

pub async fn per_sport_breakdown(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PerSportQ>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let hours = q.hours.clamp(1, 168);

    // 종목별 (slug prefix 첫 단어) 집계 — 마켓 단위 메트릭 평균.
    // 같은 (cid) 가 outcome 0/1 둘 다 fill 있을 수 있으니 cid 단위 1 row 로 묶고
    // 그 다음 종목으로 group.
    let rows = sqlx::query(&format!(r#"
        WITH market_metrics AS (
            SELECT
                f.condition_id,
                COALESCE(SUBSTRING(MIN(f.slug) FROM '^[a-z0-9]+'), 'other') AS sport,
                MIN(m.game_start) AS game_start,
                MIN(f.ts) AS first_fill,
                MAX(f.ts) AS last_fill,
                COUNT(*)::bigint AS fills,
                SUM(f.usdc_size)::float8 AS gross,
                SUM(f.size)::float8 AS shares,
                AVG(f.price)::float8 AS avg_price,
                COALESCE(STDDEV_POP(f.price), 0)::float8 AS price_stddev,
                MIN(f.price)::float8 AS min_price,
                MAX(f.price)::float8 AS max_price,
                COUNT(DISTINCT f.outcome_index)::int AS sides
            FROM fills f
            LEFT JOIN markets m ON m.condition_id = f.condition_id
            WHERE f.is_ours = false
              AND f.ts > NOW() - INTERVAL '{hours} hours'
            GROUP BY f.condition_id
        )
        SELECT
            sport,
            COUNT(*)::bigint AS markets,
            SUM(fills)::bigint AS fills,
            SUM(gross)::float8 AS gross_usdc,
            AVG(gross)::float8 AS avg_gross_per_market,
            AVG(fills)::float8 AS avg_fills_per_market,
            AVG(EXTRACT(EPOCH FROM (last_fill - first_fill))/60.0)::float8 AS avg_active_min,
            AVG(price_stddev)::float8 AS avg_price_stddev,
            AVG(max_price - min_price)::float8 AS avg_price_range,
            AVG(avg_price)::float8 AS avg_price_mean,
            -- 양면 매수 비율
            COUNT(*) FILTER (WHERE sides = 2)::bigint AS two_sided_count,
            -- 진입 시점 — 메타 있는 마켓만
            AVG(EXTRACT(EPOCH FROM (first_fill - game_start))/60.0) FILTER (WHERE game_start IS NOT NULL)::float8 AS avg_first_vs_game_min,
            COUNT(*) FILTER (WHERE game_start IS NOT NULL)::bigint AS markets_with_meta
        FROM market_metrics
        GROUP BY sport
        ORDER BY gross_usdc DESC NULLS LAST
        LIMIT 20
    "#)).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let sports: Vec<serde_json::Value> = rows.into_iter().map(|r| {
        let sport: String = r.get("sport");
        let markets: i64 = r.get("markets");
        let fills: i64 = r.get("fills");
        let gross: f64 = r.get("gross_usdc");
        let avg_gross: f64 = r.get("avg_gross_per_market");
        let avg_fills: f64 = r.get("avg_fills_per_market");
        let avg_active: f64 = r.get("avg_active_min");
        let avg_stddev: f64 = r.get("avg_price_stddev");
        let avg_range: f64 = r.get("avg_price_range");
        let avg_pmean: f64 = r.get("avg_price_mean");
        let two_sided: i64 = r.get("two_sided_count");
        let avg_entry: Option<f64> = r.get("avg_first_vs_game_min");
        let with_meta: i64 = r.get("markets_with_meta");

        // 평균 inter-fill = active_min × 60 / (fills/markets)
        let avg_inter_fill_sec = if avg_fills > 1.0 && avg_active > 0.0 {
            (avg_active * 60.0) / (avg_fills - 1.0).max(1.0)
        } else { 0.0 };

        serde_json::json!({
            "sport": sport,
            "markets": markets,
            "fills": fills,
            "gross_usdc": gross,
            "avg_gross_per_market": avg_gross,
            "avg_fills_per_market": avg_fills,
            "avg_active_min": avg_active,
            "avg_inter_fill_sec": avg_inter_fill_sec,
            "avg_price_stddev": avg_stddev,
            "avg_price_range": avg_range,
            "avg_price_mean": avg_pmean,
            "two_sided_count": two_sided,
            "two_sided_pct": if markets > 0 { two_sided as f64 / markets as f64 * 100.0 } else { 0.0 },
            "avg_first_vs_game_min": avg_entry,
            "markets_with_meta": with_meta,
        })
    }).collect();

    // 전체 집계
    let total_markets: i64 = sports.iter().map(|s| s["markets"].as_i64().unwrap_or(0)).sum();
    let total_gross: f64 = sports.iter().map(|s| s["gross_usdc"].as_f64().unwrap_or(0.0)).sum();

    Ok(Json(serde_json::json!({
        "window_hours": hours,
        "now": Utc::now(),
        "total_markets": total_markets,
        "total_gross_usdc": total_gross,
        "sports": sports,
    })))
}

/// 데이터 커버리지 진단 — fills 관측치 vs positions API 의 total_bought 격차.
/// W4: 30% 누락 (관측 $1.3M/d vs forensic $1.9M/d) 원인 파악.
pub async fn coverage_diagnostics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 마켓별 우리 관측 fills 합계 vs positions API 의 total_bought
    let rows = sqlx::query(r#"
        WITH observed AS (
            SELECT condition_id, outcome_index,
                   SUM(size)::float8       AS obs_shares,
                   SUM(usdc_size)::float8  AS obs_usdc,
                   COUNT(*)::bigint        AS obs_fills,
                   MIN(ts)                 AS first_obs,
                   MAX(ts)                 AS last_obs
            FROM fills WHERE is_ours = false
            GROUP BY condition_id, outcome_index
        )
        SELECT p.condition_id, p.outcome_index, p.outcome_name, p.title, p.event_slug,
               p.total_bought, p.size, p.avg_price,
               COALESCE(o.obs_shares, 0)::float8 AS obs_shares,
               COALESCE(o.obs_usdc, 0)::float8 AS obs_usdc,
               COALESCE(o.obs_fills, 0)::bigint AS obs_fills,
               o.first_obs, o.last_obs
        FROM positions p
        LEFT JOIN observed o ON o.condition_id = p.condition_id AND o.outcome_index = p.outcome_index
        WHERE p.total_bought > 0
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let mut total_api_bought = 0.0f64;
    let mut total_observed = 0.0f64;
    let mut gap_markets: Vec<serde_json::Value> = Vec::new();
    let mut covered_markets = 0usize;
    let mut zero_observed = 0usize;
    let total_positions = rows.len();

    for r in &rows {
        let cid: String = r.get("condition_id");
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: String = r.get("outcome_name");
        let title: Option<String> = r.get("title");
        let event_slug: Option<String> = r.get("event_slug");
        let total_bought: f64 = r.get("total_bought");
        let obs_shares: f64 = r.get("obs_shares");
        let obs_fills: i64 = r.get("obs_fills");
        let first_obs: Option<DateTime<Utc>> = r.get("first_obs");
        let last_obs: Option<DateTime<Utc>> = r.get("last_obs");

        total_api_bought += total_bought;
        total_observed += obs_shares;
        let coverage = if total_bought > 0.0 { obs_shares / total_bought * 100.0 } else { 0.0 };
        if coverage >= 90.0 { covered_markets += 1; }
        if obs_fills == 0 { zero_observed += 1; }

        // 큰 gap 만 상위 노출 (절대 누락이 큰 순)
        let gap_shares = total_bought - obs_shares;
        if gap_shares > 50.0 {
            gap_markets.push(serde_json::json!({
                "condition_id": cid,
                "outcome_index": outcome_index,
                "outcome_name": outcome_name,
                "title": title,
                "event_slug": event_slug,
                "api_total_bought": total_bought,
                "observed_shares": obs_shares,
                "observed_fills": obs_fills,
                "gap_shares": gap_shares,
                "coverage_pct": coverage,
                "first_observed": first_obs,
                "last_observed": last_obs,
            }));
        }
    }
    // gap 큰 순
    gap_markets.sort_by(|a, b| b["gap_shares"].as_f64().unwrap_or(0.0)
        .partial_cmp(&a["gap_shares"].as_f64().unwrap_or(0.0))
        .unwrap_or(std::cmp::Ordering::Equal));
    gap_markets.truncate(50);

    let overall_coverage_pct = if total_api_bought > 0.0 {
        total_observed / total_api_bought * 100.0
    } else { 0.0 };

    Ok(Json(serde_json::json!({
        "now": Utc::now(),
        "total_positions": total_positions,
        "covered_markets":  covered_markets,            // >=90% observed
        "zero_observed":    zero_observed,              // fills 0건
        "overall_coverage_pct": overall_coverage_pct,
        "total_api_bought":  total_api_bought,
        "total_observed_shares": total_observed,
        "gap_markets": gap_markets,
    })))
}

/// M1.0 Shadow runner 의 현재 상태 — minimal monitoring endpoint.
/// curl 또는 psql 보다 한 번에 데이터 모양을 볼 수 있게.
/// rich UI 는 12-24h 데이터 누적 후 만들 예정.
pub async fn m1_shadow_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // 전체 집계
    let agg: (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>) =
        sqlx::query_as(r#"
            SELECT
              COUNT(*)::bigint,
              COUNT(*) FILTER (WHERE filled_at IS NOT NULL)::bigint,
              COUNT(*) FILTER (WHERE cancelled_at IS NOT NULL AND filled_at IS NULL)::bigint,
              COUNT(*) FILTER (WHERE cancelled_at IS NULL AND filled_at IS NULL)::bigint,
              COALESCE(SUM(fill_size), 0)::float8,
              COALESCE(SUM(est_rebate_usdc), 0)::float8
            FROM shadow_quotes
        "#).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;

    // Cancellation reasons
    let cancel_breakdown_rows: Vec<(Option<String>, i64)> = sqlx::query_as(r#"
        SELECT cancelled_reason, COUNT(*)::bigint
        FROM shadow_quotes
        WHERE cancelled_at IS NOT NULL AND filled_at IS NULL
        GROUP BY cancelled_reason ORDER BY 2 DESC
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // Leg breakdown
    let leg_breakdown: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(r#"
        SELECT
          COUNT(*) FILTER (WHERE is_first_leg)::bigint,
          COUNT(*) FILTER (WHERE NOT is_first_leg)::bigint,
          COUNT(*) FILTER (WHERE is_first_leg AND filled_at IS NOT NULL)::bigint,
          COUNT(*) FILTER (WHERE NOT is_first_leg AND filled_at IS NOT NULL)::bigint
        FROM shadow_quotes
    "#).fetch_one(&state.pool).await.map_err(ErrorBody::new)?;

    // Sport breakdown
    let by_sport: Vec<(Option<String>, i64, i64, f64)> = sqlx::query_as(r#"
        SELECT sport, COUNT(*)::bigint,
               COUNT(*) FILTER (WHERE filled_at IS NOT NULL)::bigint,
               COALESCE(SUM(est_rebate_usdc), 0)::float8
        FROM shadow_quotes
        GROUP BY sport ORDER BY 2 DESC
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    // Recent quotes (20)
    let recent_rows = sqlx::query(r#"
        SELECT quote_id, placed_at, cancelled_at, filled_at,
               condition_id, outcome_index, outcome_name, price_cents, size_usdc,
               is_first_leg, mid_at_place, spread_at_place, game_eta_sec,
               fill_size, est_rebate_usdc, cancelled_reason,
               market_title, sport
        FROM shadow_quotes
        ORDER BY placed_at DESC LIMIT 20
    "#).fetch_all(&state.pool).await.map_err(ErrorBody::new)?;

    let recent: Vec<serde_json::Value> = recent_rows.into_iter().map(|r| {
        let status = if r.get::<Option<DateTime<Utc>>, _>("filled_at").is_some() { "filled" }
            else if r.get::<Option<DateTime<Utc>>, _>("cancelled_at").is_some() { "cancelled" }
            else { "open" };
        serde_json::json!({
            "quote_id":     r.get::<i64, _>("quote_id"),
            "placed_at":    r.get::<DateTime<Utc>, _>("placed_at"),
            "cancelled_at": r.get::<Option<DateTime<Utc>>, _>("cancelled_at"),
            "filled_at":    r.get::<Option<DateTime<Utc>>, _>("filled_at"),
            "status":       status,
            "condition_id": r.get::<String, _>("condition_id"),
            "outcome_index":r.get::<i16, _>("outcome_index"),
            "outcome_name": r.get::<Option<String>, _>("outcome_name"),
            "price_cents":  r.get::<i16, _>("price_cents"),
            "size_usdc":    r.get::<f64, _>("size_usdc"),
            "is_first_leg": r.get::<bool, _>("is_first_leg"),
            "mid_at_place": r.get::<Option<f64>, _>("mid_at_place"),
            "spread_at_place": r.get::<Option<f64>, _>("spread_at_place"),
            "game_eta_sec": r.get::<Option<i64>, _>("game_eta_sec"),
            "fill_size":    r.get::<f64, _>("fill_size"),
            "est_rebate_usdc": r.get::<f64, _>("est_rebate_usdc"),
            "cancelled_reason": r.get::<Option<String>, _>("cancelled_reason"),
            "market_title": r.get::<Option<String>, _>("market_title"),
            "sport":        r.get::<Option<String>, _>("sport"),
        })
    }).collect();

    // Heartbeat — 마지막 cycle 메타
    let heartbeat = sqlx::query(r#"
        SELECT ts, payload FROM system_heartbeats WHERE component = 'lifecycle.m1_shadow'
    "#).fetch_optional(&state.pool).await.map_err(ErrorBody::new)?;
    let hb = heartbeat.map(|r| serde_json::json!({
        "ts": r.get::<DateTime<Utc>, _>("ts"),
        "payload": r.get::<Option<serde_json::Value>, _>("payload"),
    }));

    let total = agg.0.unwrap_or(0);
    let filled = agg.1.unwrap_or(0);
    let fill_rate_pct = if total > 0 { filled as f64 / total as f64 * 100.0 } else { 0.0 };

    Ok(Json(serde_json::json!({
        "now": Utc::now(),
        "totals": {
            "placed":     total,
            "filled":     filled,
            "cancelled":  agg.2.unwrap_or(0),
            "open":       agg.3.unwrap_or(0),
            "fill_size_usdc":   agg.4.unwrap_or(0.0),
            "rebate_estimate":  agg.5.unwrap_or(0.0),
            "fill_rate_pct":    fill_rate_pct,
        },
        "cancel_reasons": cancel_breakdown_rows.into_iter().map(|(r, n)| serde_json::json!({
            "reason": r.unwrap_or_else(|| "(none)".into()),
            "count":  n,
        })).collect::<Vec<_>>(),
        "leg_breakdown": {
            "first_placed":   leg_breakdown.0.unwrap_or(0),
            "second_placed":  leg_breakdown.1.unwrap_or(0),
            "first_filled":   leg_breakdown.2.unwrap_or(0),
            "second_filled":  leg_breakdown.3.unwrap_or(0),
        },
        "by_sport": by_sport.into_iter().map(|(s, p, f, r)| serde_json::json!({
            "sport":  s.unwrap_or_else(|| "?".into()),
            "placed": p, "filled": f, "rebate": r,
        })).collect::<Vec<_>>(),
        "recent_quotes": recent,
        "last_heartbeat": hb,
    })))
}

pub async fn live_summary(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // Alias to trader_state — `/api/live/summary` is what the web shell hits.
    trader_state(State(state)).await
}

pub async fn system_status(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let beats = db::sync::latest_heartbeats(&state.pool).await.map_err(ErrorBody::new)?;
    let hb: Vec<serde_json::Value> = beats.into_iter().map(|(c, ts, p)| serde_json::json!({
        "component": c, "ts": ts, "payload": p,
    })).collect();
    Ok(Json(serde_json::json!({ "heartbeats": hb, "now": Utc::now() })))
}

pub async fn stream(State(state): State<Arc<AppState>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // 매 N초마다 portfolio summary snapshot 을 SSE 로 push (NOTIFY 통합은 phase 2).
    let s = stream::unfold((state, 0u64), |(state, tick)| async move {
        if tick > 0 { tokio::time::sleep(Duration::from_secs(3)).await; }
        let mut payload = serde_json::json!({ "tick": tick, "ts": Utc::now() });
        // quick KPI
        if let Ok((n_pos,)) = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM positions WHERE size > 0 OR redeemable OR mergeable"
        ).fetch_one(&state.pool).await {
            payload["position_legs"] = serde_json::json!(n_pos);
        }
        if let Ok(row) = sqlx::query_as::<_, (i64, Option<f64>)>(
            "SELECT COUNT(*), COALESCE(SUM(usdc_size),0) FROM fills WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours'"
        ).fetch_one(&state.pool).await {
            payload["fills_24h"] = serde_json::json!(row.0);
            payload["gross_24h"] = serde_json::json!(row.1.unwrap_or(0.0));
        }
        let ev = Event::default().event("tick").data(payload.to_string());
        Some((Ok::<_, Infallible>(ev), (state, tick + 1)))
    });
    Sse::new(s).keep_alive(axum::response::sse::KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("keep-alive"))
}
