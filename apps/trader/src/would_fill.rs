//! Paper-mode would-fill detector — ported from rn1-live/src/would_fill.rs.
//!
//! Polls data-api `/trades?market=<cid>&limit=200` for every market we have an
//! open paper maker quote on. A would-fill is recorded when an external trade
//! satisfies all of:
//!   1. outcome_index matches our quote leg
//!   2. trade.timestamp > our quote.signed_at (after we'd have been resting)
//!   3. Side-specific cross condition:
//!      A. Our BUY maker + SELL aggressor @ trade.price ≤ our_price
//!         → seller hit our bid directly
//!      B. Our BUY maker + BUY aggressor @ trade.price ≤ our_price
//!         → ASK existed at ≤ our price; in real CLOB our higher bid would
//!         have matched first (price priority). Paper fill at our_price.
//!         This catches the "ask drops below our bid for minutes but no
//!         SELL aggressor crosses" case that case (A) alone misses.
//!      C. Our SELL maker + BUY aggressor @ trade.price ≥ our_price
//!         → buyer took our ask directly
//!      D. Our SELL maker + SELL aggressor @ trade.price ≥ our_price
//!         → symmetric mirror of (B) for SELL makers
//!
//! Conservative assumption: any trade satisfying the above = fill (ignores
//! queue position). Strategies are compared apples-to-apples under the same
//! rule, so this approximation is fine.
//!
//! §5.1 over-fill cap: each open quote is capped at its `size` WITHIN this
//! call. The DB `fill_size_shares` is a stale snapshot — we additionally
//! track `filled_now: HashMap<order_id, f64>` to bound multi-trade matches
//! per pass.

use anyhow::{Context, Result};
use reqwest::Client;
use rn1_core::constants::DATA_API;
use rn1_db as db;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashMap;

// 2026-05-25: bumped from 200 → 500 after observing Polymarket data-api
// returning DIFFERENT trade windows for different limit values (non-
// deterministic pagination). Rinderknech-Rodionov SELL at 0.18 (10:32 UTC)
// missed by limit=200 (which returned only up to 10:30:33), caught by
// limit=300+. Use 500 for safety margin on high-volume markets.
const PER_MARKET_LIMIT: usize = 500;

#[derive(Debug, Deserialize)]
struct RawTrade {
    #[serde(rename = "conditionId")]  condition_id: Option<String>,
    #[serde(rename = "outcomeIndex")] outcome_index: Option<u8>,
    side: Option<String>,    // taker side: "BUY" | "SELL"
    price: Option<f64>,
    size: Option<f64>,
    timestamp: Option<i64>,
}

/// Scan every paper maker order's market for would-fills. Returns the count of
/// fills recorded. Call this from the driver loop every tick (paper mode only).
pub async fn check_would_fills(pool: &PgPool, http: &Client) -> Result<usize> {
    let open = db::trader::list_open_orders(pool).await.context("list_open_orders")?;
    if open.is_empty() { return Ok(0); }

    // Group by condition_id — only paper orders (clob_order_hash is None) are
    // eligible. In live mode all orders have a clob_order_hash, so this
    // function is a no-op there.
    let mut by_cid: HashMap<String, Vec<db::trader::TraderOrder>> = HashMap::new();
    for q in open {
        if q.clob_order_hash.is_some() { continue; }     // live order
        if q.kind != "maker" { continue; }                // takers fill immediately
        // S2 exit (2026-05-25): SELL makers are also simulated here.
        // BUY  → external SELL trade ≤ our price fills us.
        // SELL → external BUY  trade ≥ our price fills us.
        if q.side != "BUY" && q.side != "SELL" { continue; }
        by_cid.entry(q.condition_id.clone()).or_default().push(q);
    }
    if by_cid.is_empty() { return Ok(0); }

    let mut fills_recorded = 0usize;
    for (cid, quotes) in &by_cid {
        // ── Book-snapshot cross detection (2026-05-26) ──
        // Trade-based fill misses a real case: ASK posted at ≤ our BUY price
        // but no taker arrives → no trade event → would_fill blind to it. In
        // real CLOB our higher bid would have matched the ask immediately on
        // post (price-time priority). Detect via atc snapshot: if currently
        // best_ask ≤ our BUY → fill at our price. Symmetric for SELL.
        // Guard: skip if quote is stale (>120s old) — avoid filling on dead
        // markets where book hasn't ticked in minutes.
        if let Ok(book_rows) = sqlx::query(
            r#"SELECT outcome_index, best_bid, best_ask,
                      EXTRACT(EPOCH FROM (NOW() - quoted_at))::bigint AS age_s
                 FROM asset_to_condition
                WHERE condition_id = $1"#,
        ).bind(cid).fetch_all(pool).await {
            for r in &book_rows {
                use sqlx::Row;
                let oi: i16 = r.get("outcome_index");
                let bb: Option<f64> = r.try_get("best_bid").ok().flatten();
                let ba: Option<f64> = r.try_get("best_ask").ok().flatten();
                let age: Option<i64> = r.try_get("age_s").ok().flatten();
                if age.map(|a| a > 120).unwrap_or(true) { continue; }
                for q in quotes {
                    if q.outcome_index != oi { continue; }
                    let cross = match q.side.as_str() {
                        "BUY"  => ba.map(|a| a <= q.price + 1e-9).unwrap_or(false),
                        "SELL" => bb.map(|b| b >= q.price - 1e-9).unwrap_or(false),
                        _ => false,
                    };
                    if !cross { continue; }
                    let remaining = (q.size - q.fill_size_shares).max(0.0);
                    if remaining <= 0.0 { continue; }
                    let fill_usdc = remaining * q.price;
                    if q.side == "SELL" {
                        let hist = match db::trader::market_leg_history(pool, &q.condition_id).await {
                            Ok(h) => h, Err(_) => continue,
                        };
                        let vwap_buy = hist[q.outcome_index as usize].position_vwap;
                        if let Err(e) = db::trader::record_sell(pool, q.order_id, remaining, fill_usdc, vwap_buy).await {
                            tracing::warn!(order_id=q.order_id, err=?e, "record_sell (book-cross) failed");
                            continue;
                        }
                        let pnl = fill_usdc - remaining * vwap_buy;
                        tracing::info!(
                            order_id=q.order_id, cid=%cid, leg=q.outcome_index,
                            quote_price=q.price, book_bid=?bb, fill_shares=remaining, fill_usdc,
                            vwap=vwap_buy, realized=pnl,
                            "★ paper SELL FILL (book-cross)"
                        );
                    } else {
                        if let Err(e) = db::trader::record_fill(pool, q.order_id, remaining, fill_usdc, 0.0).await {
                            tracing::warn!(order_id=q.order_id, err=?e, "record_fill (book-cross) failed");
                            continue;
                        }
                        tracing::info!(
                            order_id=q.order_id, cid=%cid, leg=q.outcome_index,
                            quote_price=q.price, book_ask=?ba, fill_shares=remaining, fill_usdc,
                            "★ paper BUY FILL (book-cross)"
                        );
                    }
                    fills_recorded += 1;
                }
            }
        }

        // Cache-bust with nonce — Polymarket's edge cache can serve stale
        // responses for the same URL within seconds, which is the difference
        // between catching a SELL at our maker bid or missing it entirely.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis()).unwrap_or(0);
        let url = format!("{DATA_API}/trades?market={cid}&limit={PER_MARKET_LIMIT}&_t={nonce}");
        let resp = match http.get(&url)
            .header("Cache-Control", "no-cache")
            .header("Pragma", "no-cache")
            .send().await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => { tracing::warn!(%cid, status=?r.status(), "trades fetch non-200"); continue; }
            Err(e) => { tracing::warn!(%cid, err=?e, "trades fetch error"); continue; }
        };
        let trades: Vec<RawTrade> = match resp.json().await {
            Ok(t) => t,
            Err(e) => { tracing::warn!(%cid, err=?e, "trades decode"); continue; }
        };

        // §5.1 cap — bound each quote at its size WITHIN this pass.
        let mut filled_now: HashMap<i64, f64> = HashMap::new();

        for t in &trades {
            let (Some(side), Some(price), Some(size), Some(ts), Some(out_idx)) =
                (t.side.as_deref(), t.price, t.size, t.timestamp, t.outcome_index)
            else { continue };
            // Both SELL and BUY trades are relevant now (BUY trades fill our
            // SELL makers; SELL trades fill our BUY makers).
            if side != "SELL" && side != "BUY" { continue; }
            for q in quotes {
                if q.outcome_index as u8 != out_idx { continue; }
                if ts <= q.signed_at.timestamp() { continue; }     // before we placed
                // Side-specific fill condition:
                //  - Our BUY  + SELL aggressor ≤ our_price → direct hit on bid
                //  - Our BUY  + BUY  aggressor ≤ our_price → cross-detect: ASK
                //    was at trade.price, our higher bid would have matched first
                //  - Our SELL + BUY  aggressor ≥ our_price → direct hit on ask
                //  - Our SELL + SELL aggressor ≥ our_price → symmetric cross
                let fills_match = match (q.side.as_str(), side) {
                    ("BUY",  "SELL") => price <= q.price + 1e-9,
                    ("BUY",  "BUY")  => price <= q.price + 1e-9,
                    ("SELL", "BUY")  => price >= q.price - 1e-9,
                    ("SELL", "SELL") => price >= q.price - 1e-9,
                    _ => false,
                };
                if !fills_match { continue; }
                let prior = filled_now.get(&q.order_id).copied().unwrap_or(0.0);
                let remaining = (q.size - q.fill_size_shares - prior).max(0.0);
                if remaining <= 0.0 { continue; }
                let fill_shares = remaining.min(size);
                let fill_usdc = fill_shares * q.price;
                if q.side == "SELL" {
                    // S2 exit — realized PnL = fill_usdc - shares × vwap_buy.
                    // We need cost basis. Look up leg's BUY vwap from history.
                    let hist = match db::trader::market_leg_history(pool, &q.condition_id).await {
                        Ok(h) => h, Err(_) => continue,
                    };
                    let vwap_buy = hist[q.outcome_index as usize].position_vwap;
                    if let Err(e) = db::trader::record_sell(pool, q.order_id, fill_shares, fill_usdc, vwap_buy).await {
                        tracing::warn!(order_id=q.order_id, err=?e, "record_sell failed");
                        continue;
                    }
                    let pnl = fill_usdc - fill_shares * vwap_buy;
                    tracing::info!(
                        order_id=q.order_id, cid=%cid, leg=q.outcome_index,
                        quote_price=q.price, trade_price=price, fill_shares, fill_usdc,
                        vwap=vwap_buy, realized=pnl,
                        "★ paper SELL FILL (S2 exit)"
                    );
                } else {
                    let rebate = 0.0;
                    if let Err(e) = db::trader::record_fill(pool, q.order_id, fill_shares, fill_usdc, rebate).await {
                        tracing::warn!(order_id=q.order_id, err=?e, "record_fill failed");
                        continue;
                    }
                    tracing::info!(
                        order_id=q.order_id, cid=%cid, leg=q.outcome_index,
                        quote_price=q.price, trade_price=price, fill_shares, fill_usdc,
                        "★ paper FILL (would-have)"
                    );
                }
                *filled_now.entry(q.order_id).or_insert(0.0) += fill_shares;
                fills_recorded += 1;
            }
        }
    }
    Ok(fills_recorded)
}
