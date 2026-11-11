//! M1.0 Shadow runner — quote 계산 + 로그만 (실제 post X).
//!
//! 30s 주기 cycle:
//!   1. 마켓 스캔 — markets 테이블 + LATERAL book event 로 필터 통과한 cid 추출
//!   2. open shadow_quotes 확인 — 각 cid 의 현재 leg 상태
//!   3. Quote 결정 — rules 1-7 (M1 strategy spec v0) 적용
//!      · 첫 leg: narrower-spread side, best_bid, $30
//:      · 두 leg: 첫 leg fill ≥ $30 도달 시 반대 outcome
//!   4. Stale cancellation — 5min 이상 미체결 / mid ±2¢ drift / spread 좁아짐
//!   5. Would-fill detection — recent RN1 fills 중 우리 quote (cid, idx, price_cents) 매치 시 filled_at 갱신
//!   6. heartbeat 기록
//!
//! Universe (v0): WTA + ATP only. M1.3 에서 EPL 추가.

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use rn1_core::constants::*;
use rn1_db as db;
use sqlx::{PgPool, Row};
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug, Clone)]
#[command(name = "rn1-m1-shadow", version, about = "M1.0 shadow quote runner")]
struct Args {
    /// Cycle 주기 (초) — 짧을수록 가격 변화 추적 빠름
    #[arg(long, env = "M1_CYCLE_S", default_value_t = 15)]
    cycle_s: u64,

    /// Quote size (USDC)
    #[arg(long, env = "M1_QUOTE_USDC", default_value_t = 30.0)]
    quote_usdc: f64,

    /// 최대 동시 open markets
    #[arg(long, env = "M1_MAX_OPEN_MARKETS", default_value_t = 5)]
    max_open_markets: usize,

    /// 단일 leg max usdc cap
    #[arg(long, env = "M1_SINGLE_LEG_CAP", default_value_t = 30.0)]
    single_leg_cap: f64,

    /// Per-market max exposure (both legs)
    #[arg(long, env = "M1_MARKET_CAP", default_value_t = 60.0)]
    market_cap: f64,

    /// Universe sport slugs (comma separated).
    /// Shadow phase 는 24h coverage 위해 광범위하게 — 종목별 fill rate 비교도 가능.
    /// M1.1 live 진입 시엔 sweet spot (atp,wta) 으로 좁힐 수 있음 (env override).
    #[arg(long, env = "M1_UNIVERSE", default_value = "atp,wta,epl,mlb,lib,sud,cs2,nhl")]
    universe: String,

    /// Stale quote 시간 (초)
    #[arg(long, env = "M1_STALE_S", default_value_t = 180)]
    stale_s: i64,

    /// Quote 재호가 mid drift 임계 (cents) — 작을수록 가격 변동에 더 빨리 재호가
    #[arg(long, env = "M1_MID_DRIFT_CENTS", default_value_t = 1)]
    mid_drift_cents: i32,

    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,rn1=debug")))
        .with_target(false)
        .compact()
        .init();
    let args = Args::parse();
    info!(
        universe = %args.universe, quote_usdc = args.quote_usdc,
        max_open_markets = args.max_open_markets, cycle_s = args.cycle_s,
        "M1.0 shadow runner starting (wallet={})", RN1_WALLET
    );

    if let Some(url) = &args.database_url { std::env::set_var("DATABASE_URL", url); }
    let pool = db::pool_from_env().await?;
    db::migrate(&pool).await?;

    let universe: Vec<String> = args.universe.split(',').map(|s| s.trim().to_lowercase()).collect();
    info!(?universe, "scoped sports");

    let mut tick: u64 = 0;
    loop {
        tick += 1;
        let cycle_start = std::time::Instant::now();
        match run_cycle(&pool, &args, &universe).await {
            Ok(stats) => {
                if tick % 4 == 0 || stats.placed + stats.filled + stats.cancelled > 0 {
                    info!(tick, ?stats, "shadow cycle");
                }
                let _ = db::sync::record_heartbeat(&pool, "lifecycle.m1_shadow",
                    serde_json::json!({
                        "tick": tick,
                        "candidates": stats.candidates,
                        "open": stats.open,
                        "placed": stats.placed,
                        "filled": stats.filled,
                        "cancelled": stats.cancelled,
                        "cycle_ms": cycle_start.elapsed().as_millis(),
                    })).await;
            }
            Err(e) => warn!(err=?e, "shadow cycle failed"),
        }
        tokio::time::sleep(Duration::from_secs(args.cycle_s)).await;
    }
}

#[derive(Debug, Default)]
struct CycleStats {
    candidates: i64,
    open: i64,
    placed: i64,
    filled: i64,
    cancelled: i64,
}

async fn run_cycle(pool: &PgPool, args: &Args, universe: &[String]) -> Result<CycleStats> {
    let mut stats = CycleStats::default();

    // 1. 매칭 마켓 후보 — universe 종목 + 시간창 + spread + 가격대
    let candidates = scan_candidates(pool, universe).await?;
    stats.candidates = candidates.len() as i64;

    // 2. 현재 open shadow quotes — 마켓별 leg 상태
    let open = fetch_open_quotes(pool).await?;
    stats.open = open.len() as i64;

    // 3. Stale + drift cancellation
    let stale_cutoff = Utc::now() - chrono::Duration::seconds(args.stale_s);
    for q in &open {
        // filled 된 quote 는 cancel 대상 아님 — second-leg 판단용으로만 유지.
        if q.filled_at.is_some() { continue; }
        let mut cancel_reason: Option<&'static str> = None;
        if q.placed_at < stale_cutoff {
            cancel_reason = Some("stale_5min");
        } else {
            // mid drift 검사 — leg 단위 비교 (시장 평균 mid 가 아니라 outcome 별 mid).
            // mid_at_place 는 placement 시 book.mid (outcome 별), 현재도 leg 의 mid 가져옴.
            let current_leg_mid = candidates.iter()
                .find(|c| c.condition_id == q.condition_id)
                .and_then(|c| c.book.iter().find(|b| b.outcome_index == q.outcome_index))
                .map(|b| b.mid);
            if let (Some(current_mid), Some(placed_mid)) = (current_leg_mid, q.mid_at_place) {
                if ((current_mid - placed_mid).abs() * 100.0) as i32 > args.mid_drift_cents {
                    cancel_reason = Some("mid_drift");
                }
            }
        }
        if let Some(r) = cancel_reason {
            cancel_quote(pool, q.quote_id, r).await?;
            stats.cancelled += 1;
        }
    }

    // 4. 시장별 quote 결정
    let open_after_cancel = fetch_open_quotes(pool).await?;
    let open_markets_count = open_after_cancel.iter()
        .map(|q| &q.condition_id)
        .collect::<std::collections::HashSet<_>>().len();

    if open_markets_count < args.max_open_markets {
        let slots_remaining = args.max_open_markets - open_markets_count;
        let mut placed_this_cycle = 0;

        for market in &candidates {
            if placed_this_cycle >= slots_remaining { break; }
            // 이미 이 cid 에 open quote 있으면 skip (first leg 만 우선)
            let market_open: Vec<_> = open_after_cancel.iter()
                .filter(|q| q.condition_id == market.condition_id).collect();
            if !market_open.is_empty() {
                // first leg 가 fill 되었고 second leg quote 가 아직 없으면 그것을 placing 시도
                let filled_legs: std::collections::HashSet<i16> = market_open.iter()
                    .filter(|q| q.filled_at.is_some()).map(|q| q.outcome_index).collect();
                let open_legs:   std::collections::HashSet<i16> = market_open.iter()
                    .filter(|q| q.filled_at.is_none() && q.cancelled_at.is_none()).map(|q| q.outcome_index).collect();

                if !filled_legs.is_empty() {
                    // 첫 leg fill 된 거 있음 → 반대 outcome 에 second leg quote 시도
                    let other_idx = if filled_legs.contains(&0) { 1i16 } else { 0i16 };
                    if !open_legs.contains(&other_idx) && !filled_legs.contains(&other_idx) {
                        if let Some(other_book) = market.book.iter().find(|b| b.outcome_index == other_idx) {
                            place_quote(pool, market, other_book, args, /*first_leg=*/false).await?;
                            placed_this_cycle += 1;
                            stats.placed += 1;
                        }
                    }
                }
                continue;
            }

            // 새 market — first leg 결정 + place
            // narrower-spread 가 우선, 동률이면 cheaper (lower mid)
            let first_book = pick_first_leg(market);
            if let Some(book) = first_book {
                place_quote(pool, market, book, args, /*first_leg=*/true).await?;
                placed_this_cycle += 1;
                stats.placed += 1;
            }
        }
    }

    // 5. Would-fill detection — 최근 fills 와 open quotes 매칭
    let detected = detect_would_fills(pool, args).await?;
    stats.filled += detected;

    Ok(stats)
}

#[derive(Debug, Clone)]
struct Candidate {
    condition_id: String,
    title: Option<String>,
    slug: String,
    event_slug: Option<String>,
    sport: Option<String>,
    mid: f64,
    book: Vec<OutcomeBook>,
    game_eta_sec: Option<i64>,
}

#[derive(Debug, Clone)]
struct OutcomeBook {
    outcome_index: i16,
    outcome_name: Option<String>,
    best_bid: f64,
    best_ask: f64,
    spread: f64,
    mid: f64,
}

/// 후보 마켓 스캔.
/// 필터:
///  · sport ∈ universe (slug prefix)
///  · active=true, closed=false
///  · game_start: now-3h ~ now+1h (live 또는 임박 pre-game)
///  · 두 outcome 모두 bid/ask 있음, spread ≤ 5¢
///  · mid ∈ [0.25, 0.65]
///  · rewards_max_spread > 0
async fn scan_candidates(pool: &PgPool, universe: &[String]) -> Result<Vec<Candidate>> {
    // markets 테이블 + asset_to_condition + 최신 book event LATERAL
    // sport prefix 매칭은 SQL ILIKE 로
    let sport_filter = universe.iter()
        .map(|s| format!("'{}%'", s.replace('\'', "")))
        .collect::<Vec<_>>().join(", ");
    if sport_filter.is_empty() { return Ok(vec![]); }

    let sql = format!(r#"
        WITH market_outcomes AS (
            SELECT m.condition_id, m.question, m.slug, m.event_slug, m.outcomes,
                   m.game_start, m.rewards_max_spread,
                   atc.outcome_index, atc.asset_id, atc.outcome_name,
                   SUBSTRING(m.slug FROM '^[a-z0-9]+') AS sport
            FROM markets m
            JOIN asset_to_condition atc ON atc.condition_id = m.condition_id
            WHERE m.active = true AND m.closed = false
              AND m.rewards_max_spread > 0
              AND m.game_start IS NOT NULL
              -- live (3h 이내 시작) ~ pre-game 6h 까지 (ATP avg entry = 2.4h pre).
              -- per-sport 분석: ATP -143min, WTA +15min, EPL -62min, MLB +28min
              AND m.game_start BETWEEN NOW() - INTERVAL '3 hours' AND NOW() + INTERVAL '6 hours'
              AND SUBSTRING(m.slug FROM '^[a-z0-9]+') ILIKE ANY (ARRAY[{sports}])
        )
        SELECT mo.condition_id, mo.question, mo.slug, mo.event_slug,
               mo.game_start, mo.outcome_index, mo.outcome_name, mo.sport,
               b.best_bid, b.best_ask
        FROM market_outcomes mo
        LEFT JOIN LATERAL (
            SELECT best_bid, best_ask
            FROM book_events
            WHERE asset_id = mo.asset_id AND event_type = 'book'
              AND best_bid IS NOT NULL AND best_ask IS NOT NULL
            ORDER BY ts DESC LIMIT 1
        ) b ON true
        WHERE b.best_bid IS NOT NULL
        ORDER BY mo.condition_id, mo.outcome_index
    "#, sports = sport_filter);

    let rows = sqlx::query(&sql).fetch_all(pool).await?;

    let now = Utc::now();
    // group by condition_id
    use std::collections::HashMap;
    let mut grouped: HashMap<String, Candidate> = HashMap::new();
    for r in &rows {
        let cid: String = r.get("condition_id");
        let question: Option<String> = r.get("question");
        let slug: String = r.get("slug");
        let event_slug: Option<String> = r.get("event_slug");
        let sport: Option<String> = r.get("sport");
        let game_start: Option<chrono::DateTime<Utc>> = r.get("game_start");
        let outcome_index: i16 = r.get("outcome_index");
        let outcome_name: Option<String> = r.get("outcome_name");
        let bid: f64 = r.get("best_bid");
        let ask: f64 = r.get("best_ask");

        if ask <= bid { continue; }
        let spread = ask - bid;
        let mid = (ask + bid) / 2.0;
        if spread > 0.05 { continue; }              // 5¢ spread cap
        // 가격대 범위 — 15-85¢ (이전 25-65 는 양면 binary 의 favorite/underdog 쌍을
        // 둘 다 cut 하는 문제. 15-85 면 매우 극단 (winning leg 거의 확정) 만 제외).
        if mid < 0.15 || mid > 0.85 { continue; }

        let game_eta = game_start.map(|gs| (gs - now).num_seconds());

        let entry = grouped.entry(cid.clone()).or_insert_with(|| Candidate {
            condition_id: cid.clone(),
            title: question.clone(),
            slug: slug.clone(),
            event_slug: event_slug.clone(),
            sport: sport.clone(),
            mid: 0.0,
            book: Vec::new(),
            game_eta_sec: game_eta,
        });
        entry.book.push(OutcomeBook {
            outcome_index, outcome_name, best_bid: bid, best_ask: ask, spread, mid,
        });
    }

    // 양 outcome 모두 quote 가능한 마켓만 (binary 검증)
    let mut out: Vec<Candidate> = grouped.into_values()
        .filter(|c| c.book.len() == 2)
        .map(|mut c| {
            c.mid = c.book.iter().map(|b| b.mid).sum::<f64>() / c.book.len() as f64;
            c
        })
        .collect();
    // sort: spread 좁은 순 (narrower = priority)
    out.sort_by(|a, b| {
        let sa: f64 = a.book.iter().map(|b| b.spread).sum();
        let sb: f64 = b.book.iter().map(|b| b.spread).sum();
        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(out)
}

/// First leg 선택: narrower-spread 측 우선, 동률이면 cheaper (lower mid)
fn pick_first_leg(market: &Candidate) -> Option<&OutcomeBook> {
    if market.book.is_empty() { return None; }
    market.book.iter().min_by(|a, b| {
        a.spread.partial_cmp(&b.spread)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.mid.partial_cmp(&b.mid).unwrap_or(std::cmp::Ordering::Equal))
    })
}

#[derive(Debug, Clone)]
struct OpenQuote {
    quote_id: i64,
    condition_id: String,
    outcome_index: i16,
    price_cents: i16,
    size_usdc: f64,
    placed_at: chrono::DateTime<Utc>,
    mid_at_place: Option<f64>,
    is_first_leg: bool,
    cancelled_at: Option<chrono::DateTime<Utc>>,
    filled_at: Option<chrono::DateTime<Utc>>,
}

/// 현재 "활성" 상태의 quotes — 진짜 open + 최근 1h 내 filled (second leg 후속 판단용).
/// 1h 이상 된 filled 는 "완료" 로 간주 → slot 카운트에서 제외 → 새 마켓 placement 허용.
async fn fetch_open_quotes(pool: &PgPool) -> Result<Vec<OpenQuote>> {
    let rows = sqlx::query(r#"
        SELECT quote_id, condition_id, outcome_index, price_cents, size_usdc,
               placed_at, mid_at_place, is_first_leg, cancelled_at, filled_at
        FROM shadow_quotes
        WHERE cancelled_at IS NULL
          AND (filled_at IS NULL OR filled_at > NOW() - INTERVAL '1 hour')
        ORDER BY placed_at DESC
    "#).fetch_all(pool).await?;
    let out = rows.into_iter().map(|r| OpenQuote {
        quote_id:      r.get("quote_id"),
        condition_id:  r.get("condition_id"),
        outcome_index: r.get("outcome_index"),
        price_cents:   r.get("price_cents"),
        size_usdc:     r.get("size_usdc"),
        placed_at:     r.get("placed_at"),
        mid_at_place:  r.get("mid_at_place"),
        is_first_leg:  r.get("is_first_leg"),
        cancelled_at:  r.get("cancelled_at"),
        filled_at:     r.get("filled_at"),
    }).collect();
    Ok(out)
}

async fn place_quote(pool: &PgPool, market: &Candidate, book: &OutcomeBook,
                     args: &Args, first_leg: bool) -> Result<()> {
    let price_cents = (book.best_bid * 100.0).round() as i16;
    let now = Utc::now();
    sqlx::query(r#"
        INSERT INTO shadow_quotes
            (placed_at, condition_id, outcome_index, outcome_name, side, price_cents,
             size_usdc, is_first_leg, mid_at_place, spread_at_place, game_eta_sec,
             market_title, market_slug, event_slug, sport)
        VALUES ($1, $2, $3, $4, 'BUY', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
    "#)
    .bind(now)
    .bind(&market.condition_id)
    .bind(book.outcome_index)
    .bind(&book.outcome_name)
    .bind(price_cents)
    .bind(args.quote_usdc)
    .bind(first_leg)
    .bind(book.mid)
    .bind(book.spread)
    .bind(market.game_eta_sec)
    .bind(&market.title)
    .bind(&market.slug)
    .bind(&market.event_slug)
    .bind(&market.sport)
    .execute(pool).await?;

    info!(
        cid = %market.condition_id, leg = book.outcome_index, price_cents,
        first = first_leg, mid = book.mid, spread = book.spread,
        "shadow quote placed"
    );
    Ok(())
}

async fn cancel_quote(pool: &PgPool, quote_id: i64, reason: &str) -> Result<()> {
    sqlx::query(r#"
        UPDATE shadow_quotes
        SET cancelled_at = NOW(), cancelled_reason = $2
        WHERE quote_id = $1 AND cancelled_at IS NULL AND filled_at IS NULL
    "#).bind(quote_id).bind(reason).execute(pool).await?;
    Ok(())
}

/// Recent RN1 fills 중 우리 open quote (cid, idx, price_cents) 매칭 시 filled_at 갱신.
/// price_cents 매칭 — RN1 의 0.42-0.43 이 우리 42¢ quote 와 일치.
/// 같은 quote 가 여러 fills 에 매칭 가능 — fill_size 누적, est_rebate 갱신.
async fn detect_would_fills(pool: &PgPool, args: &Args) -> Result<i64> {
    let rebate_pct = 0.0075;  // 0.75% sports maker

    // 최근 cycle_s × 3 동안의 fills 와 모든 open quote 매칭
    let lookback_s = (args.cycle_s as i64) * 3;
    let rows_affected: u64 = sqlx::query(&format!(r#"
        WITH matches AS (
            SELECT q.quote_id,
                   SUM(f.usdc_size)::float8 AS fill_usdc,
                   MIN(f.ts) AS first_fill_ts
            FROM shadow_quotes q
            JOIN fills f
              ON f.condition_id = q.condition_id
             AND f.outcome_index = q.outcome_index
             AND ROUND((f.price * 100)::numeric)::int = q.price_cents
            WHERE q.cancelled_at IS NULL
              AND q.filled_at IS NULL
              AND f.is_ours = false
              AND f.ts > q.placed_at
              AND f.ts > NOW() - INTERVAL '{lookback_s} seconds'
            GROUP BY q.quote_id
        )
        UPDATE shadow_quotes
        SET filled_at = matches.first_fill_ts,
            fill_size = LEAST(matches.fill_usdc, shadow_quotes.size_usdc),
            est_rebate_usdc = LEAST(matches.fill_usdc, shadow_quotes.size_usdc) * {rebate_pct}
        FROM matches
        WHERE shadow_quotes.quote_id = matches.quote_id
    "#)).execute(pool).await?.rows_affected();

    if rows_affected > 0 {
        info!(rows = rows_affected, "shadow would-fill detected");
    }
    Ok(rows_affected as i64)
}
