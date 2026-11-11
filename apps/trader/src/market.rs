//! Focus-market resolver — reads from the `markets` table populated by
//! `apps/lifecycle-collector`. For Phase A we restrict the trader to a single
//! `RN1_FOCUS_SLUG` (matches RN1_LIVE_LOGIC_HANDOFF §10.3). The full universe
//! scan path is left for Phase B (per-market actor refactor).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Serialize)]
pub struct FocusMarket {
    pub condition_id: String,
    pub slug: String,
    pub event_slug: Option<String>,
    pub question: Option<String>,
    pub neg_risk: bool,
    pub game_start: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub volume_num: Option<f64>,
    /// outcome_index 0 / 1 → (outcome_name, token_id)
    pub outcomes: [(String, String); 2],
    /// Polymarket has resolved the market (winner declared) or marked it
    /// inactive. The trader must not place new orders here — only cancel
    /// resting ones. Held positions get redeemed via the CTF redeemPositions
    /// contract call (Phase B). For paper-mode this just means "stop trading".
    pub settled: bool,
    /// `[outcome0_price, outcome1_price]` after resolution, e.g. [1.0, 0.0].
    /// `None` before resolution.
    pub outcome_prices: Option<[f64; 2]>,
}

/// Fetch the focus market by slug. Returns `Ok(None)` if not yet ingested by the
/// collector. The trader will retry on the next gamma_refresh tick.
pub async fn fetch_focus(pool: &PgPool, slug: &str) -> Result<Option<FocusMarket>> {
    let row = sqlx::query(
        r#"SELECT condition_id, slug, event_slug, question, neg_risk, game_start,
                  end_date, volume_num, outcomes, clob_token_ids,
                  closed, active, outcome_prices
             FROM markets WHERE slug = $1 LIMIT 1"#,
    )
    .bind(slug)
    .fetch_optional(pool).await
    .context("query markets by slug")?;
    parse_row(row)
}

/// Fetch a market by condition_id. Used by the follow-up loop to revisit
/// markets where the bot already holds positions (focus may have moved on).
pub async fn fetch_market_by_cid(pool: &PgPool, cid: &str) -> Result<Option<FocusMarket>> {
    let row = sqlx::query(
        r#"SELECT condition_id, slug, event_slug, question, neg_risk, game_start,
                  end_date, volume_num, outcomes, clob_token_ids,
                  closed, active, outcome_prices
             FROM markets WHERE condition_id = $1 LIMIT 1"#,
    )
    .bind(cid)
    .fetch_optional(pool).await
    .context("query markets by cid")?;
    parse_row(row)
}

/// Autonomous universe scan — pick the most-liquid ATP/WTA market currently in
/// the window `[game_start − 3h, game_start + 6h]` whose volume_num clears
/// `min_event_volume` and whose slug doesn't match `exclude_substrings`.
///
/// Matches RN1_LIVE_LOGIC_HANDOFF §2 (volume gating is the universe selector).
/// `universe_prefixes` typically `["atp", "wta"]` — these select the slug HEAD
/// before the first `-` (e.g. `atp-paris-djokovic-...` → `atp`).
/// Returns up to `n` top-volume eligible markets (descending by volume). Used
/// when `RN1_MAX_OPEN_MARKETS > 1` — the trader processes each as its own
/// focus in a single tick, so capital can spread across multiple high-vol
/// matches instead of saturating one and ignoring the rest.
pub async fn fetch_autonomous_candidates(
    pool: &PgPool,
    universe_prefixes: &[String],
    min_event_volume: f64,
    exclude_substrings: &[String],
    blocked_cids: &std::collections::HashSet<String>,
    n: usize,
) -> Result<Vec<FocusMarket>> {
    if universe_prefixes.is_empty() || n == 0 {
        return Ok(Vec::new());
    }
    let prefix_like: Vec<String> = universe_prefixes.iter()
        .map(|p| format!("{}-%", p.trim().to_lowercase()))
        .collect();
    let rows = sqlx::query(
        r#"SELECT condition_id, slug, event_slug, question, neg_risk, game_start,
                  end_date, volume_num, outcomes, clob_token_ids,
                  closed, active, outcome_prices
             FROM markets
            WHERE closed = false
              AND active = true
              AND volume_num >= $2
              AND game_start IS NOT NULL
              AND game_start BETWEEN NOW() - INTERVAL '3 hours' AND NOW() + INTERVAL '6 hours'
              AND slug LIKE ANY($1)
            ORDER BY volume_num DESC NULLS LAST
            LIMIT 50"#,
    )
    .bind(&prefix_like[..])
    .bind(min_event_volume)
    .fetch_all(pool).await
    .context("autonomous scan: query markets")?;

    let mut out: Vec<FocusMarket> = Vec::with_capacity(n);
    for r in rows {
        let cid: String = r.get("condition_id");
        if blocked_cids.contains(&cid) { continue; }
        let slug: String = r.get("slug");
        let s_lower = slug.to_lowercase();
        if exclude_substrings.iter().any(|x| s_lower.contains(x.as_str())) { continue; }
        if let Some(m) = parse_row(Some(r))? {
            out.push(m);
            if out.len() >= n { break; }
        }
    }
    Ok(out)
}

pub async fn fetch_autonomous_candidate(
    pool: &PgPool,
    universe_prefixes: &[String],
    min_event_volume: f64,
    exclude_substrings: &[String],
    blocked_cids: &std::collections::HashSet<String>,
) -> Result<Option<FocusMarket>> {
    if universe_prefixes.is_empty() {
        return Ok(None);
    }
    // Build SLUG LIKE filter as `(slug LIKE 'atp%' OR slug LIKE 'wta%' …)`.
    let prefix_like: Vec<String> = universe_prefixes.iter()
        .map(|p| format!("{}-%", p.trim().to_lowercase()))
        .collect();

    // We keep the SQL simple — fetch top candidates, then exclude in-process.
    let rows = sqlx::query(
        r#"SELECT condition_id, slug, event_slug, question, neg_risk, game_start,
                  end_date, volume_num, outcomes, clob_token_ids,
                  closed, active, outcome_prices
             FROM markets
            WHERE closed = false
              AND active = true
              AND volume_num >= $2
              AND game_start IS NOT NULL
              AND game_start BETWEEN NOW() - INTERVAL '3 hours' AND NOW() + INTERVAL '6 hours'
              AND slug LIKE ANY($1)
            ORDER BY volume_num DESC NULLS LAST
            LIMIT 50"#,
    )
    .bind(&prefix_like[..])
    .bind(min_event_volume)
    .fetch_all(pool).await
    .context("autonomous scan: query markets")?;

    for r in rows {
        let cid: String = r.get("condition_id");
        if blocked_cids.contains(&cid) {
            continue;            // CLOB book returned 404 on a previous tick
        }
        let slug: String = r.get("slug");
        let s_lower = slug.to_lowercase();
        if exclude_substrings.iter().any(|x| s_lower.contains(x.as_str())) {
            continue;
        }
        match parse_row(Some(r))? {
            Some(m) => return Ok(Some(m)),
            None => continue,
        }
    }
    Ok(None)
}

fn parse_row(row: Option<sqlx::postgres::PgRow>) -> Result<Option<FocusMarket>> {
    let Some(r) = row else { return Ok(None); };

    let slug: String = r.get("slug");
    let outcomes_json: serde_json::Value = r.get("outcomes");
    let token_ids_json: serde_json::Value = r.get("clob_token_ids");
    let outcomes_arr = parse_string_array(&outcomes_json);
    let token_ids_arr = parse_string_array(&token_ids_json);
    if outcomes_arr.len() != 2 || token_ids_arr.len() != 2 {
        anyhow::bail!("market {} has != 2 outcomes/tokens", slug);
    }

    let closed: bool = r.try_get("closed").unwrap_or(false);
    let active: bool = r.try_get("active").unwrap_or(true);
    let outcome_prices_json: Option<serde_json::Value> = r.try_get("outcome_prices").ok();
    let outcome_prices = outcome_prices_json.as_ref().and_then(parse_price_pair);
    // Polymarket sometimes leaves `closed=false` for a brief window post-game
    // even when outcome_prices is already set to [1, 0]; treat any of those as
    // settled. Likewise `active=false` means lifecycle has retired the market.
    let resolved_via_prices = outcome_prices
        .map(|p| (p[0] - 1.0).abs() < 1e-6 || (p[1] - 1.0).abs() < 1e-6)
        .unwrap_or(false);
    let settled = closed || !active || resolved_via_prices;

    Ok(Some(FocusMarket {
        condition_id: r.get("condition_id"),
        slug,
        event_slug: r.get("event_slug"),
        question: r.get("question"),
        neg_risk: r.get("neg_risk"),
        game_start: r.get("game_start"),
        end_date: r.get("end_date"),
        volume_num: r.get("volume_num"),
        outcomes: [
            (outcomes_arr[0].clone(), token_ids_arr[0].clone()),
            (outcomes_arr[1].clone(), token_ids_arr[1].clone()),
        ],
        settled,
        outcome_prices,
    }))
}

/// Parse `outcome_prices` from the DB — typically `[1.0, 0.0]` or `[0.0, 1.0]`
/// once Polymarket resolves the market. Returns `None` if not a 2-element list.
fn parse_price_pair(v: &serde_json::Value) -> Option<[f64; 2]> {
    let arr = if let Some(a) = v.as_array() {
        a.clone()
    } else if let Some(s) = v.as_str() {
        serde_json::from_str::<Vec<serde_json::Value>>(s).ok()?
    } else {
        return None;
    };
    if arr.len() != 2 { return None; }
    let p0 = arr[0].as_f64().or_else(|| arr[0].as_str().and_then(|s| s.parse().ok()))?;
    let p1 = arr[1].as_f64().or_else(|| arr[1].as_str().and_then(|s| s.parse().ok()))?;
    Some([p0, p1])
}

fn parse_string_array(v: &serde_json::Value) -> Vec<String> {
    if let Some(arr) = v.as_array() {
        return arr.iter().filter_map(|x| x.as_str().map(String::from)).collect();
    }
    if let Some(s) = v.as_str() {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(s) { return arr; }
    }
    Vec::new()
}
