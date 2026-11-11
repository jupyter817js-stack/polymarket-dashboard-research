//! Trader-side helpers operating on the rn1-bot `orders` table. Mirrors what
//! rn1-live's SQLite `state.rs` did, adapted to Postgres + the schema already
//! present in 0001_init.sql + 0006_trader.sql.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

/// A single order row as the trader cares about it (post 0006_trader columns).
#[derive(Debug, Clone, Serialize)]
pub struct TraderOrder {
    pub order_id: i64,
    pub clob_order_hash: Option<String>,
    pub condition_id: String,
    pub outcome_index: i16,
    pub token_id: Option<String>,
    pub asset_id: String,
    pub side: String,
    pub price: f64,
    pub size: f64,
    pub size_usdc: f64,
    pub fill_size_shares: f64,
    pub fill_usdc: f64,
    pub rebate_usdc: f64,
    pub mid_at_place: Option<f64>,
    pub kind: String,
    pub tag: String,
    pub status: String,
    pub signed_at: DateTime<Utc>,
    pub posted_at: Option<DateTime<Utc>>,
    pub filled_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancelled_reason: Option<String>,
}

fn row_to_order(r: &sqlx::postgres::PgRow) -> TraderOrder {
    TraderOrder {
        order_id: r.get("order_id"),
        clob_order_hash: r.get("clob_order_hash"),
        condition_id: r.get("condition_id"),
        outcome_index: r.get("outcome_index"),
        token_id: r.get("token_id"),
        asset_id: r.get("asset_id"),
        side: r.get("side"),
        price: r.get("price"),
        size: r.get("size"),
        size_usdc: r.get("size_usdc"),
        fill_size_shares: r.get("fill_size_shares"),
        fill_usdc: r.get("fill_usdc"),
        rebate_usdc: r.get("rebate_usdc"),
        mid_at_place: r.get("mid_at_place"),
        kind: r.get("kind"),
        tag: r.get("tag"),
        status: r.get("status"),
        signed_at: r.get("signed_at"),
        posted_at: r.get("posted_at"),
        filled_at: r.get("filled_at"),
        cancelled_at: r.get("cancelled_at"),
        cancelled_reason: r.get("cancelled_reason"),
    }
}

const OPEN_STATUSES: [&str; 4] = ["signed", "posted", "resting", "partially_filled"];

/// Insert a new order in `signed` state. Returns the freshly minted order_id.
#[allow(clippy::too_many_arguments)]
pub async fn insert_order(
    pool: &PgPool,
    condition_id: &str,
    outcome_index: i16,
    asset_id: &str,
    token_id: &str,
    side: &str,
    price: f64,
    size_shares: f64,
    size_usdc: f64,
    mid_at_place: Option<f64>,
    kind: &str,
    tag: &str,
) -> Result<i64> {
    let row = sqlx::query(
        r#"INSERT INTO orders
             (condition_id, outcome_index, asset_id, token_id, side, price, size,
              size_usdc, mid_at_place, kind, tag, status, signed_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'signed',NOW())
           RETURNING order_id"#,
    )
    .bind(condition_id)
    .bind(outcome_index)
    .bind(asset_id)
    .bind(token_id)
    .bind(side)
    .bind(price)
    .bind(size_shares)
    .bind(size_usdc)
    .bind(mid_at_place)
    .bind(kind)
    .bind(tag)
    .fetch_one(pool).await?;
    Ok(row.get("order_id"))
}

/// After signing+POST succeeded — attach the CLOB-returned order id, mark posted.
pub async fn mark_posted(pool: &PgPool, order_id: i64, clob_order_hash: &str) -> Result<()> {
    sqlx::query(
        r#"UPDATE orders
             SET clob_order_hash = $1,
                 status = 'posted',
                 posted_at = NOW()
           WHERE order_id = $2"#,
    )
    .bind(clob_order_hash)
    .bind(order_id)
    .execute(pool).await?;
    Ok(())
}

pub async fn mark_rejected(pool: &PgPool, order_id: i64, reason: &str) -> Result<()> {
    sqlx::query(
        r#"UPDATE orders
             SET status = 'rejected',
                 rejection_reason = $1,
                 cancelled_at = NOW()
           WHERE order_id = $2"#,
    )
    .bind(reason)
    .bind(order_id)
    .execute(pool).await?;
    Ok(())
}

pub async fn cancel_order(pool: &PgPool, order_id: i64, reason: &str) -> Result<()> {
    sqlx::query(
        r#"UPDATE orders
             SET status = 'cancelled',
                 cancelled_at = NOW(),
                 cancelled_reason = $2
           WHERE order_id = $1
             AND status = ANY($3)"#,
    )
    .bind(order_id)
    .bind(reason)
    .bind(&OPEN_STATUSES[..])
    .execute(pool).await?;
    Ok(())
}

pub async fn list_open_orders(pool: &PgPool) -> Result<Vec<TraderOrder>> {
    let rows = sqlx::query(
        r#"SELECT * FROM orders WHERE status = ANY($1) ORDER BY signed_at DESC"#,
    )
    .bind(&OPEN_STATUSES[..])
    .fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_order).collect())
}

pub async fn recent_closed_orders(pool: &PgPool, limit: i64) -> Result<Vec<TraderOrder>> {
    let rows = sqlx::query(
        r#"SELECT * FROM orders
            WHERE status IN ('filled','cancelled','rejected','expired')
            ORDER BY COALESCE(filled_at, cancelled_at, signed_at) DESC
            LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_order).collect())
}

pub async fn find_order_by_clob_hash(pool: &PgPool, clob_hash: &str) -> Result<Option<TraderOrder>> {
    let row = sqlx::query(
        "SELECT * FROM orders WHERE clob_order_hash = $1 LIMIT 1",
    )
    .bind(clob_hash)
    .fetch_optional(pool).await?;
    Ok(row.as_ref().map(row_to_order))
}

/// Record a fill onto an order. Increments fill_size_shares / fill_usdc /
/// rebate. Promotes the status to 'filled' once the order is fully filled
/// (within 0.1%); otherwise marks it 'partially_filled'.
pub async fn record_fill(
    pool: &PgPool,
    order_id: i64,
    size_shares: f64,
    usdc: f64,
    rebate: f64,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"UPDATE orders SET
              fill_size_shares = fill_size_shares + $2,
              fill_usdc        = fill_usdc + $3,
              rebate_usdc      = rebate_usdc + $4,
              filled_at        = COALESCE(filled_at, NOW()),
              status           = CASE
                  WHEN fill_size_shares + $2 >= size * 0.999 THEN 'filled'
                  ELSE 'partially_filled'
              END
           WHERE order_id = $1"#,
    )
    .bind(order_id)
    .bind(size_shares)
    .bind(usdc)
    .bind(rebate)
    .execute(&mut *tx).await?;

    // Mirror the fill into the canonical `fills` table so positions reconcile
    // through the same path RN1 observation uses.
    sqlx::query(
        r#"INSERT INTO fills
              (tx_hash, condition_id, outcome_index, side, size, price,
               usdc_size, fee_paid, rebate_received, ts, is_ours, is_maker)
            SELECT NULL, condition_id, outcome_index, side, $2, price,
                   $3, 0, $4, NOW(), TRUE,
                   CASE WHEN kind = 'maker' THEN TRUE ELSE FALSE END
            FROM orders WHERE order_id = $1
            ON CONFLICT DO NOTHING"#,
    )
    .bind(order_id)
    .bind(size_shares)
    .bind(usdc)
    .bind(rebate)
    .execute(&mut *tx).await?;

    sqlx::query(
        r#"INSERT INTO trader_daily (date, gross_filled, rebate, fills_count)
           VALUES (CURRENT_DATE, $1, $2, 1)
           ON CONFLICT (date) DO UPDATE SET
              gross_filled = trader_daily.gross_filled + EXCLUDED.gross_filled,
              rebate       = trader_daily.rebate       + EXCLUDED.rebate,
              fills_count  = trader_daily.fills_count  + 1,
              updated_at   = NOW()"#,
    )
    .bind(usdc)
    .bind(rebate)
    .execute(&mut *tx).await?;

    tx.commit().await?;
    Ok(())
}

/// Record a paper-mode SELL fill (S2 exit layer). Updates the order row
/// (fill columns) AND inserts a side='SELL' row into the `fills` table so
/// `market_leg_history` subtracts the shares. Realized PnL = `usdc_received
/// − shares × vwap_at_sell` added to `trader_daily`.
pub async fn record_sell(
    pool: &PgPool,
    order_id: i64,
    size_shares: f64,
    usdc_received: f64,
    cost_basis_per_share: f64,
) -> Result<()> {
    if size_shares <= 0.0 { return Ok(()); }
    let realized = usdc_received - size_shares * cost_basis_per_share;
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"UPDATE orders SET
              fill_size_shares = fill_size_shares + $2,
              fill_usdc        = fill_usdc + $3,
              filled_at        = COALESCE(filled_at, NOW()),
              status           = CASE
                  WHEN fill_size_shares + $2 >= size * 0.999 THEN 'filled'
                  ELSE 'partially_filled'
              END
           WHERE order_id = $1"#,
    )
    .bind(order_id).bind(size_shares).bind(usdc_received)
    .execute(&mut *tx).await?;

    // Mirror into `fills` with side='SELL' so leg_history subtracts properly.
    sqlx::query(
        r#"INSERT INTO fills
              (tx_hash, condition_id, outcome_index, side, size, price,
               usdc_size, fee_paid, rebate_received, ts, is_ours, is_maker)
            SELECT NULL, condition_id, outcome_index, 'SELL', $2, price,
                   $3, 0, 0, NOW(), TRUE,
                   CASE WHEN kind = 'maker' THEN TRUE ELSE FALSE END
            FROM orders WHERE order_id = $1
            ON CONFLICT DO NOTHING"#,
    )
    .bind(order_id).bind(size_shares).bind(usdc_received)
    .execute(&mut *tx).await?;

    sqlx::query(
        r#"INSERT INTO trader_daily (date, realized_pnl, fills_count)
           VALUES (CURRENT_DATE, $1, 1)
           ON CONFLICT (date) DO UPDATE SET
              realized_pnl = trader_daily.realized_pnl + EXCLUDED.realized_pnl,
              fills_count  = trader_daily.fills_count + 1,
              updated_at   = NOW()"#,
    )
    .bind(realized).execute(&mut *tx).await?;

    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct LegHistory {
    pub position_shares: f64,
    pub position_usdc: f64,
    pub position_vwap: f64,
}

/// Per-leg `(shares, vwap)` for one market — read from `fills` (is_ours=true
/// BUY rows), with MERGE recycling AND settlement REDEMPTIONS subtracted.
/// MERGE reduces both legs equally (matched pairs recovered at $1/pair).
/// REDEMPTION zeros out the leg (closed market, shares paid out at outcome).
pub async fn market_leg_history(pool: &PgPool, condition_id: &str) -> Result<[LegHistory; 2]> {
    // BUY-side aggregate. SELLs are subtracted below — preserves vwap of the
    // remaining held position (selling at vwap doesn't change average cost
    // basis; selling above vwap leaves cheaper shares behind, etc.).
    let rows = sqlx::query(
        r#"SELECT outcome_index::int4 AS leg,
                  COALESCE(SUM(size), 0)::float8       AS shares,
                  COALESCE(SUM(usdc_size), 0)::float8  AS usdc
             FROM fills
            WHERE condition_id = $1 AND is_ours = TRUE AND side = 'BUY'
            GROUP BY outcome_index"#,
    )
    .bind(condition_id)
    .fetch_all(pool).await?;

    let mut out = [LegHistory::default(), LegHistory::default()];
    for r in rows {
        let leg: i32 = r.get("leg");
        let shares: f64 = r.get("shares");
        let usdc: f64 = r.get("usdc");
        let vwap = if shares > 0.0 { usdc / shares } else { 0.0 };
        if leg == 0 {
            out[0] = LegHistory { position_shares: shares, position_usdc: usdc, position_vwap: vwap };
        } else if leg == 1 {
            out[1] = LegHistory { position_shares: shares, position_usdc: usdc, position_vwap: vwap };
        }
    }
    // Subtract SELL fills per leg (S2 exit layer, 2026-05-25). Selling at vwap
    // would not change vwap; selling at higher price leaves remaining shares at
    // same (or slightly cheaper) effective vwap — we model this as: subtract
    // shares × vwap from cost basis, keep vwap constant.
    let sell_rows = sqlx::query(
        r#"SELECT outcome_index::int4 AS leg,
                  COALESCE(SUM(size), 0)::float8 AS sold_shares
             FROM fills
            WHERE condition_id = $1 AND is_ours = TRUE AND side = 'SELL'
            GROUP BY outcome_index"#,
    )
    .bind(condition_id)
    .fetch_all(pool).await.unwrap_or_default();
    for r in sell_rows {
        let leg: i32 = r.get("leg");
        let sold: f64 = r.get("sold_shares");
        if (0..=1).contains(&leg) && sold > 0.0 {
            let o = &mut out[leg as usize];
            if o.position_shares > sold {
                let cost_removed = sold * o.position_vwap;
                o.position_shares -= sold;
                o.position_usdc   -= cost_removed;
            } else {
                o.position_shares = 0.0;
                o.position_usdc   = 0.0;
                o.position_vwap   = 0.0;
            }
        }
    }
    // Subtract merges from BOTH legs equally, preserving leg vwap.
    let merged: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(shares), 0)::float8 FROM merges WHERE condition_id = $1",
    )
    .bind(condition_id)
    .fetch_one(pool).await.unwrap_or(0.0);
    if merged > 0.0 {
        for o in out.iter_mut() {
            if o.position_shares > merged {
                let cost_removed = merged * o.position_vwap;
                o.position_shares -= merged;
                o.position_usdc   -= cost_removed;
            } else if o.position_shares > 0.0 {
                o.position_shares = 0.0;
                o.position_usdc   = 0.0;
                o.position_vwap   = 0.0;
            }
        }
    }
    // Subtract redemptions per leg (zero out the redeemed portion).
    let red_rows = sqlx::query(
        "SELECT outcome_index::int4 AS leg, SUM(shares)::float8 AS shares
           FROM redemptions WHERE condition_id = $1 GROUP BY outcome_index",
    )
    .bind(condition_id)
    .fetch_all(pool).await.unwrap_or_default();
    for r in red_rows {
        let leg: i32 = r.get("leg");
        let redeemed: f64 = r.get("shares");
        if (0..=1).contains(&leg) {
            let o = &mut out[leg as usize];
            o.position_shares = (o.position_shares - redeemed).max(0.0);
            if o.position_shares <= 1e-9 {
                o.position_shares = 0.0;
                o.position_usdc   = 0.0;
                o.position_vwap   = 0.0;
            }
        }
    }
    Ok(out)
}

/// Record a paper-mode settlement redemption. Inserts a row into `redemptions`,
/// credits realized PnL = `shares × settle_price − cost_basis` into
/// `trader_daily`. Idempotent per (condition_id, outcome_index) via UNIQUE.
pub async fn record_redemption(
    pool: &PgPool,
    condition_id: &str,
    outcome_index: i16,
    shares: f64,
    settle_price: f64,
    cost_basis: f64,
) -> Result<bool> {
    if shares <= 0.0 { return Ok(false); }
    let usdc_received = shares * settle_price;
    let realized     = usdc_received - cost_basis;
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        r#"INSERT INTO redemptions
              (condition_id, outcome_index, shares, settle_price,
               usdc_received, cost_basis, realized_pnl, ts)
           VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
           ON CONFLICT (condition_id, outcome_index) DO NOTHING"#,
    )
    .bind(condition_id).bind(outcome_index).bind(shares).bind(settle_price)
    .bind(usdc_received).bind(cost_basis).bind(realized)
    .execute(&mut *tx).await?;

    if res.rows_affected() == 0 {
        tx.rollback().await.ok();
        return Ok(false);
    }

    sqlx::query(
        r#"INSERT INTO trader_daily (date, realized_pnl)
           VALUES (CURRENT_DATE, $1)
           ON CONFLICT (date) DO UPDATE SET
              realized_pnl = trader_daily.realized_pnl + EXCLUDED.realized_pnl,
              updated_at   = NOW()"#,
    )
    .bind(realized).execute(&mut *tx).await?;

    tx.commit().await?;
    Ok(true)
}

/// All redeemable markets — closed + outcome_prices set + we still hold
/// un-redeemed shares. Returns (condition_id, outcome_prices). Driver calls
/// `record_redemption` for each leg of each cid returned here.
pub async fn pending_redemptions(pool: &PgPool) -> Result<Vec<(String, [f64; 2])>> {
    let rows = sqlx::query(
        r#"SELECT DISTINCT m.condition_id, m.outcome_prices
             FROM markets m
             JOIN fills f ON f.condition_id = m.condition_id AND f.is_ours = TRUE
             LEFT JOIN redemptions r ON r.condition_id = m.condition_id
            WHERE m.closed = TRUE
              AND m.outcome_prices IS NOT NULL
              AND r.id IS NULL"#,
    )
    .fetch_all(pool).await?;
    let mut out = Vec::new();
    for r in rows {
        let cid: String = r.get("condition_id");
        let op: serde_json::Value = r.get("outcome_prices");
        let prices = parse_outcome_prices(&op);
        if let Some(p) = prices {
            // Only redeem if at least one outcome is decisive (≥ 0.99 or ≤ 0.01).
            let decisive = (p[0] >= 0.99 || p[0] <= 0.01) || (p[1] >= 0.99 || p[1] <= 0.01);
            if decisive {
                out.push((cid, p));
            }
        }
    }
    Ok(out)
}

fn parse_outcome_prices(v: &serde_json::Value) -> Option<[f64; 2]> {
    let arr = if let Some(a) = v.as_array() { a.clone() }
              else if let Some(s) = v.as_str() {
                  serde_json::from_str::<Vec<serde_json::Value>>(s).ok()?
              } else { return None; };
    if arr.len() != 2 { return None; }
    let p0 = arr[0].as_f64().or_else(|| arr[0].as_str().and_then(|s| s.parse().ok()))?;
    let p1 = arr[1].as_f64().or_else(|| arr[1].as_str().and_then(|s| s.parse().ok()))?;
    Some([p0, p1])
}

/// Record a paper-mode MERGE. Inserts a row into `merges` and credits
/// `realized_pnl = shares × (1.0 − matched_vwap_at_merge)` into
/// `trader_daily`. Used by the planner-driven recycle path (§6.5).
pub async fn record_merge(
    pool: &PgPool,
    condition_id: &str,
    shares: f64,
    matched_vwap: f64,
) -> Result<()> {
    if shares <= 0.0 { return Ok(()); }
    let usdc_recovered = shares;                       // $1 per matched pair
    let realized = shares * (1.0 - matched_vwap);
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO merges
              (condition_id, shares, usdc_recovered, matched_vwap, realized_pnl, ts)
           VALUES ($1, $2, $3, $4, $5, NOW())"#,
    )
    .bind(condition_id).bind(shares).bind(usdc_recovered)
    .bind(matched_vwap).bind(realized)
    .execute(&mut *tx).await?;

    sqlx::query(
        r#"INSERT INTO trader_daily (date, realized_pnl)
           VALUES (CURRENT_DATE, $1)
           ON CONFLICT (date) DO UPDATE SET
              realized_pnl = trader_daily.realized_pnl + EXCLUDED.realized_pnl,
              updated_at   = NOW()"#,
    )
    .bind(realized)
    .execute(&mut *tx).await?;

    tx.commit().await?;
    Ok(())
}

/// Total shares ever merged on a market. Used by the risk module to subtract
/// recovered USDC from event_cost when computing per-event headroom.
pub async fn merged_shares(pool: &PgPool, condition_id: &str) -> Result<f64> {
    let v: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(shares), 0)::float8 FROM merges WHERE condition_id = $1",
    )
    .bind(condition_id)
    .fetch_one(pool).await?;
    Ok(v)
}

/// Insert a synthetic reconcile fill so the leg-history reflects an on-chain
/// position the bot didn't observe directly. Inserts only the GAP — idempotent.
/// Skips when avg_price ∉ (0,1) — data-api briefly returns 0 for just-filled.
pub async fn reconcile_position(
    pool: &PgPool,
    condition_id: &str,
    outcome_index: i16,
    real_shares: f64,
    avg_price: f64,
) -> Result<bool> {
    if !(avg_price > 0.0 && avg_price < 1.0) {
        tracing::warn!(condition_id, outcome_index, avg_price,
            "reconcile skipped — implausible avg_price (data-api not ready); will retry");
        return Ok(false);
    }
    let known: f64 = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(size), 0)::float8
             FROM fills
            WHERE condition_id = $1 AND outcome_index = $2
              AND is_ours = TRUE AND side = 'BUY'"#,
    )
    .bind(condition_id)
    .bind(outcome_index)
    .fetch_one(pool).await?;
    let gap = real_shares - known;
    if gap < 0.5 { return Ok(false); }
    let usdc = gap * avg_price;
    sqlx::query(
        r#"INSERT INTO fills
             (tx_hash, condition_id, outcome_index, side, size, price, usdc_size,
              fee_paid, rebate_received, ts, is_ours, is_maker)
           VALUES (NULL, $1, $2, 'BUY', $3, $4, $5, 0, 0, NOW(), TRUE, NULL)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(condition_id)
    .bind(outcome_index)
    .bind(gap)
    .bind(avg_price)
    .bind(usdc)
    .execute(pool).await?;
    Ok(true)
}

/// Update the per-asset best_bid/best_ask cache. Called by the trader every
/// tick after fetching books — so the API can show **outcome-specific** prices
/// (the `markets` table only stores one bid/ask per row, conflating legs).
pub async fn upsert_asset_quote(
    pool: &PgPool,
    asset_id: &str,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE asset_to_condition
              SET best_bid = COALESCE($2, best_bid),
                  best_ask = COALESCE($3, best_ask),
                  quoted_at = NOW()
            WHERE asset_id = $1"#,
    )
    .bind(asset_id)
    .bind(best_bid)
    .bind(best_ask)
    .execute(pool).await?;
    Ok(())
}

/// Read the last cached best_bid / best_ask for one asset. Used by the trader
/// as a fallback when the live CLOB book has gone empty (dead market post-
/// resolution) — without this, follow-up housekeeping (BAIL, stale-maker
/// cancel) cannot fire because the planner needs both bids.
pub async fn get_cached_quote(
    pool: &PgPool,
    asset_id: &str,
) -> Result<Option<(f64, f64)>> {
    let row = sqlx::query(
        "SELECT best_bid, best_ask FROM asset_to_condition WHERE asset_id = $1",
    )
    .bind(asset_id)
    .fetch_optional(pool).await?;
    let Some(r) = row else { return Ok(None) };
    let bid: Option<f64> = r.get("best_bid");
    let ask: Option<f64> = r.get("best_ask");
    match (bid, ask) {
        (Some(b), Some(a)) if b > 0.0 && a > 0.0 => Ok(Some((b, a))),
        _ => Ok(None),
    }
}

/// All condition_ids where the bot currently holds a net-positive position
/// from is_ours fills (MERGE-recycled shares subtracted). Used by the multi-
/// market follow-up tick — naked legs and remaining matched positions in
/// other markets still need attention even when trader's focus has moved on.
pub async fn active_market_cids(pool: &PgPool) -> Result<Vec<String>> {
    // Note: explicit ON clauses (not USING) — chaining USING clauses across
    // multiple LEFT JOINs produces "common column appears more than once"
    // ambiguity in Postgres. Cost me the entire follow-up loop on 2026-05-25
    // (trader silently saw active_cids=[] for hours).
    let rows = sqlx::query(
        r#"WITH fills_per_leg AS (
              SELECT condition_id, outcome_index, SUM(size) AS shares
                FROM fills
               WHERE is_ours = TRUE AND side = 'BUY'
               GROUP BY condition_id, outcome_index
           ),
           sells_per_leg AS (
              SELECT condition_id, outcome_index, COALESCE(SUM(size), 0) AS sold
                FROM fills
               WHERE is_ours = TRUE AND side = 'SELL'
               GROUP BY condition_id, outcome_index
           ),
           merge_per_cid AS (
              SELECT condition_id, COALESCE(SUM(shares), 0) AS merged
                FROM merges GROUP BY condition_id
           ),
           redeem_per_leg AS (
              SELECT condition_id, outcome_index, COALESCE(SUM(shares), 0) AS redeemed
                FROM redemptions GROUP BY condition_id, outcome_index
           )
           SELECT DISTINCT f.condition_id AS cid
             FROM fills_per_leg f
             LEFT JOIN sells_per_leg s
                    ON s.condition_id  = f.condition_id
                   AND s.outcome_index = f.outcome_index
             LEFT JOIN merge_per_cid m
                    ON m.condition_id  = f.condition_id
             LEFT JOIN redeem_per_leg r
                    ON r.condition_id  = f.condition_id
                   AND r.outcome_index = f.outcome_index
            WHERE (f.shares
                   - COALESCE(s.sold, 0)
                   - COALESCE(m.merged, 0)
                   - COALESCE(r.redeemed, 0)) > 0.5"#,
    )
    .fetch_all(pool).await?;
    Ok(rows.iter().map(|r| r.get::<String, _>("cid")).collect())
}

/// Open notional = unfilled remaining of all resting orders.
pub async fn open_unfilled_notional(pool: &PgPool) -> Result<f64> {
    let v: f64 = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(GREATEST(size_usdc - fill_usdc, 0)), 0)::float8
             FROM orders WHERE status = ANY($1)"#,
    )
    .bind(&OPEN_STATUSES[..])
    .fetch_one(pool).await?;
    Ok(v.max(0.0))
}

/// Today's filled notional — sum of fill_usdc for orders filled today.
pub async fn today_filled_notional(pool: &PgPool) -> Result<f64> {
    let v: f64 = sqlx::query_scalar(
        r#"SELECT COALESCE(gross_filled, 0)::float8
             FROM trader_daily WHERE date = CURRENT_DATE"#,
    )
    .fetch_optional(pool).await?
    .unwrap_or(0.0);
    Ok(v)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TraderDaily {
    pub date: String,
    pub gross_filled: f64,
    pub realized_pnl: f64,
    pub rebate: f64,
    pub fills_count: i32,
    pub kill_switch: bool,
}

pub async fn today_summary(pool: &PgPool) -> Result<TraderDaily> {
    let row = sqlx::query(
        r#"SELECT date::text AS date, gross_filled, realized_pnl, rebate,
                  fills_count, kill_switch
             FROM trader_daily WHERE date = CURRENT_DATE"#,
    )
    .fetch_optional(pool).await?;
    Ok(match row {
        Some(r) => TraderDaily {
            date: r.get("date"),
            gross_filled: r.get("gross_filled"),
            realized_pnl: r.get("realized_pnl"),
            rebate: r.get("rebate"),
            fills_count: r.get("fills_count"),
            kill_switch: r.get("kill_switch"),
        },
        None => TraderDaily {
            date: Utc::now().date_naive().to_string(),
            ..TraderDaily::default()
        },
    })
}

pub async fn set_kill_switch(pool: &PgPool, on: bool) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO trader_daily (date, kill_switch)
            VALUES (CURRENT_DATE, $1)
            ON CONFLICT (date) DO UPDATE SET kill_switch = EXCLUDED.kill_switch,
                                              updated_at = NOW()"#,
    )
    .bind(on)
    .execute(pool).await?;
    Ok(())
}

pub async fn kill_switch_active(pool: &PgPool) -> Result<bool> {
    let v: Option<bool> = sqlx::query_scalar(
        "SELECT kill_switch FROM trader_daily WHERE date = CURRENT_DATE",
    )
    .fetch_optional(pool).await?;
    Ok(v.unwrap_or(false))
}

/// Upsert the single-row trader_status. JSON fields are passed pre-encoded.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_trader_status(
    pool: &PgPool,
    mode: &str,
    running: bool,
    focus_slug: Option<&str>,
    current_tick: i64,
    last_cycle_ms: i64,
    candidates: i32,
    ws_market_connected: bool,
    ws_user_connected: bool,
    kill_switch: bool,
    last_error: Option<&str>,
    wallet: Option<serde_json::Value>,
    config_snapshot: Option<serde_json::Value>,
    strategy: Option<serde_json::Value>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO trader_status
              (id, mode, running, focus_slug, current_tick, last_cycle_at,
               last_cycle_ms, candidates, ws_market_connected, ws_user_connected,
               kill_switch, last_error, wallet, config_snapshot, strategy, updated_at)
           VALUES (1,$1,$2,$3,$4,NOW(),$5,$6,$7,$8,$9,$10,$11,$12,$13,NOW())
           ON CONFLICT (id) DO UPDATE SET
              mode                = EXCLUDED.mode,
              running             = EXCLUDED.running,
              focus_slug          = EXCLUDED.focus_slug,
              current_tick        = EXCLUDED.current_tick,
              last_cycle_at       = EXCLUDED.last_cycle_at,
              last_cycle_ms       = EXCLUDED.last_cycle_ms,
              candidates          = EXCLUDED.candidates,
              ws_market_connected = EXCLUDED.ws_market_connected,
              ws_user_connected   = EXCLUDED.ws_user_connected,
              kill_switch         = EXCLUDED.kill_switch,
              last_error          = EXCLUDED.last_error,
              wallet              = COALESCE(EXCLUDED.wallet,        trader_status.wallet),
              config_snapshot     = COALESCE(EXCLUDED.config_snapshot, trader_status.config_snapshot),
              strategy            = EXCLUDED.strategy,
              updated_at          = NOW()"#,
    )
    .bind(mode)
    .bind(running)
    .bind(focus_slug)
    .bind(current_tick)
    .bind(last_cycle_ms)
    .bind(candidates)
    .bind(ws_market_connected)
    .bind(ws_user_connected)
    .bind(kill_switch)
    .bind(last_error)
    .bind(wallet)
    .bind(config_snapshot)
    .bind(strategy)
    .execute(pool).await?;
    Ok(())
}
