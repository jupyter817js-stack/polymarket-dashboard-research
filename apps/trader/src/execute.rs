//! Live execution glue — submit_buy / cancel / order-state reconcile.
//! Mirrors rn1-live/src/live.rs minus the SELL path (BUY-only strategy per §2).

use alloy_primitives::{B256, U256};
use anyhow::{Context, Result};
use rn1_clob::{ClobClient, ClobOrder, ClobPostOrderBody, OrderBook};
use rn1_signer::{random_salt, now_unix_millis, Order, SIDE_BUY};
use sqlx::PgPool;
use std::collections::HashSet;

use crate::live_ctx::LiveCtx;

pub const USDC_DECIMALS: u32 = 6;
pub const SHARES_DECIMALS: u32 = 6;

/// Defense-in-depth: reject a quote price that crosses the spread.
pub fn validate_maker_quote(price: f64, size_shares: f64, current_book: &OrderBook) -> Result<()> {
    if price <= 0.0 || price >= 1.0 {
        anyhow::bail!("invalid price {}", price);
    }
    if let Some(best_ask) = current_book.best_ask() {
        if price >= best_ask {
            anyhow::bail!("price {} >= best_ask {} (would cross spread)", price, best_ask);
        }
    }
    if size_shares <= 0.0 {
        anyhow::bail!("invalid size_shares {}", size_shares);
    }
    Ok(())
}

/// Build, sign, POST a single-leg BUY. `taker=true` → FAK (used for sweeps and
/// optionally completions); `taker=false` → GTC (maker-only).
#[allow(clippy::too_many_arguments)]
pub async fn submit_buy(
    clob: &ClobClient,
    ctx: &LiveCtx,
    token_id: &str,
    size_usdc: f64,
    size_shares: f64,
    neg_risk: bool,
    taker: bool,
) -> Result<String> {
    let maker_amount = (size_usdc * 10f64.powi(USDC_DECIMALS as i32)).round() as u128;
    let taker_amount = (size_shares * 10f64.powi(SHARES_DECIMALS as i32)).round() as u128;
    if maker_amount == 0 || taker_amount == 0 {
        anyhow::bail!("zero amount after unit conversion");
    }
    let token_u256: U256 = token_id.parse().context("parse token_id as U256")?;
    let timestamp_ms = now_unix_millis() as u64;
    let order = Order {
        salt:          random_salt().parse().context("salt to U256")?,
        maker:         ctx.maker_address,
        signer:        ctx.signer_field(),
        tokenId:       token_u256,
        makerAmount:   U256::from(maker_amount),
        takerAmount:   U256::from(taker_amount),
        side:          SIDE_BUY,
        signatureType: ctx.signature_type,
        timestamp:     U256::from(timestamp_ms),
        metadata:      B256::ZERO,
        builder:       B256::ZERO,
    };
    let verifying_contract = ClobClient::exchange_for(neg_risk);
    let signature = ctx.signer.sign_order(&order, verifying_contract)?;

    let to_lower = |a: alloy_primitives::Address| format!("{:?}", a).to_lowercase();
    let salt_u64: u64 = order.salt.try_into().unwrap_or(u64::MAX);
    let clob_order = ClobOrder {
        salt:           salt_u64,
        maker:          to_lower(order.maker),
        signer:         to_lower(order.signer),
        taker:          String::new(),
        token_id:       token_id.to_string(),
        maker_amount:   order.makerAmount.to_string(),
        taker_amount:   order.takerAmount.to_string(),
        side:           "BUY".to_string(),
        signature_type: order.signatureType,
        timestamp:      order.timestamp.to_string(),
        expiration:     "0".to_string(),
        metadata:       format!("0x{}", hex::encode(order.metadata.as_slice())),
        builder:        format!("0x{}", hex::encode(order.builder.as_slice())),
        signature,
    };

    let body = ClobPostOrderBody {
        defer_exec: false,
        post_only:  false,
        order: clob_order,
        owner: ctx.api_key.clone(),
        order_type: if taker { "FAK".to_string() } else { "GTC".to_string() },
    };

    let addr_lower = ctx.signer.address_lower();
    let auth = ctx.l2_auth(&addr_lower);
    let resp = clob.post_order(&auth, &body).await?;
    if resp.success == Some(false) {
        anyhow::bail!("order rejected: {:?}", resp.error_msg);
    }
    resp.order_id().map(String::from)
        .ok_or_else(|| anyhow::anyhow!("CLOB returned no order_id: {:?}", resp))
}

pub async fn cancel_order(clob: &ClobClient, ctx: &LiveCtx, clob_order_id: &str) -> Result<()> {
    let addr_lower = ctx.signer.address_lower();
    let auth = ctx.l2_auth(&addr_lower);
    clob.cancel_order(&auth, clob_order_id).await
}

/// Order-state reconciliation (RN1_LIVE_LOGIC_HANDOFF §4.5).
/// Any DB-open order whose clob_order_hash isn't in the live CLOB set is marked
/// cancelled, so the planner re-places. 30s placement-age grace; on
/// API/parse error returns Err and marks nothing.
pub async fn reconcile_open_orders(
    clob: &ClobClient,
    ctx: &LiveCtx,
    pool: &PgPool,
) -> Result<usize> {
    let addr = ctx.signer.address_lower();
    let auth = ctx.l2_auth(&addr);
    let resps = clob.get_open_orders(&auth, None).await
        .context("reconcile_open_orders: get_open_orders")?;
    let body = match resps.first() { Some(s) => s, None => return Ok(0) };
    let parsed: serde_json::Value = serde_json::from_str(body)
        .context("reconcile_open_orders: decode")?;
    let live: HashSet<String> = match parsed["data"].as_array() {
        Some(a) => a.iter()
            .filter_map(|o| o.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect(),
        None => return Ok(0),
    };
    let mut reconciled = 0usize;
    let open = rn1_db::trader::list_open_orders(pool).await?;
    let now = chrono::Utc::now();
    for q in open {
        let Some(hash) = q.clob_order_hash.as_deref() else { continue };
        if (now - q.signed_at).num_seconds() < 30 { continue; }
        if !live.contains(hash) {
            let _ = rn1_db::trader::cancel_order(pool, q.order_id, "vanished").await;
            tracing::warn!(order_id = q.order_id, leg = q.outcome_index, price = q.price,
                "★ order vanished from CLOB — marked cancelled (not bot-initiated)");
            reconciled += 1;
        }
    }
    Ok(reconciled)
}

/// Startup orphan cleanup — cancel CLOB orders that the bot has no record of.
pub async fn cleanup_orphan_orders(
    clob: &ClobClient,
    ctx: &LiveCtx,
    pool: &PgPool,
) -> Result<usize> {
    let addr = ctx.signer.address_lower();
    let auth = ctx.l2_auth(&addr);
    let resps = clob.get_open_orders(&auth, None).await
        .context("orphan-cleanup: get_open_orders")?;
    let body = match resps.first() { Some(s) => s, None => return Ok(0) };
    let parsed: serde_json::Value = serde_json::from_str(body)
        .context("orphan-cleanup: decode")?;
    let orders = match parsed["data"].as_array() { Some(a) => a, None => return Ok(0) };

    let known: HashSet<String> = rn1_db::trader::list_open_orders(pool).await?
        .into_iter().filter_map(|q| q.clob_order_hash).collect();

    let mut cancelled = 0usize;
    for o in orders {
        let id = o.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() || known.contains(id) { continue; }
        match cancel_order(clob, ctx, id).await {
            Ok(()) => {
                tracing::warn!(orphan=%id, "★ orphan order cancelled");
                cancelled += 1;
            }
            Err(e) => tracing::warn!(orphan=%id, err=?e, "orphan cancel failed"),
        }
    }
    Ok(cancelled)
}
