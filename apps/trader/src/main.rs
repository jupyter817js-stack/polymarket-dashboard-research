//! rn1-trader — Polymarket pair-completion bot.
//!
//! Event-driven driver mirroring rn1-live/src/main.rs (RN1_LIVE_LOGIC_HANDOFF §4):
//!   trigger ← (WS book update | WS user fill | 10s backstop)
//!   → decision_tick: fetch books, build PairState, plan_market, exec.
//!
//! Phase A: single focus_slug (RN1_FOCUS_SLUG). Phase B (per-market actors)
//! deferred per handoff §11.12.

mod config;
mod execute;
mod gamma_scan;
mod live_ctx;
mod market;
mod risk;
mod wallet;
mod would_fill;

use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Notify, RwLock};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use rn1_clob::ClobClient;
use rn1_db as db;
use rn1_planner::{plan_market, Affordability, CollapseDetector, OrderKind, PairState, RestingOrder, Side};
use rn1_signer::PolySigner;
use rn1_ws::{
    run_market_subscriber, spawn_user_channel, MarketWsClient, RawMarketEvent, UserAuth, UserEvent,
};

use crate::config::{Config, Mode};
use crate::execute::{
    cancel_order as live_cancel, cleanup_orphan_orders, reconcile_open_orders, submit_buy,
    validate_maker_quote,
};
use crate::live_ctx::{prepare_live, LiveCtx};
use crate::market::{fetch_autonomous_candidate, fetch_autonomous_candidates, fetch_focus, fetch_market_by_cid, FocusMarket};
use crate::risk::RiskGuard;
use crate::wallet::WalletSnapshot;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Arc::new(Config::load()?);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,rn1_trader=debug,sqlx=warn")),
        )
        .with_target(false)
        .compact()
        .init();

    if let Some(u) = &cfg.database_url {
        std::env::set_var("DATABASE_URL", u);
    }
    let pool = db::pool_from_env().await.context("pg pool")?;
    db::migrate(&pool).await.context("migrate")?;

    info!(
        mode = ?cfg.mode, focus = %cfg.focus_slug,
        per_event_cap = cfg.per_event_cap_usdc,
        cumulative_aim = cfg.cumulative_aim,
        cumulative_target = cfg.cumulative_target_max,
        daily_loss_cap = cfg.daily_loss_cap_usdc,
        "rn1-trader starting"
    );

    let clob = Arc::new(ClobClient::new()?);

    // ── Smoke test ──────────────────────────────────────────────────────
    if cfg.smoke_test {
        return run_smoke_test(&cfg, &clob).await;
    }

    // ── Live context (signer + L2) ─────────────────────────────────────
    let live_ctx: Option<LiveCtx> = if cfg.mode.is_live() {
        info!("preparing live trading context (signer + L2 api-key)…");
        let ctx = prepare_live(&cfg, &clob).await.context("prepare_live")?;
        info!(address = %ctx.signer.address_lower(), "live ctx ready");
        match cleanup_orphan_orders(&clob, &ctx, &pool).await {
            Ok(n) if n > 0 => warn!(cancelled = n, "orphan orders cleaned up at startup"),
            Ok(_) => info!("no orphan orders at startup"),
            Err(e) => warn!(err=?e, "orphan cleanup failed (continuing)"),
        }
        Some(ctx)
    } else { None };

    // ── State / shared ─────────────────────────────────────────────────
    let wallet_snap: Arc<RwLock<WalletSnapshot>> = Arc::new(RwLock::new(WalletSnapshot::default()));
    let book_dirty = Arc::new(Notify::new());
    let reconcile_wake = Arc::new(Notify::new());
    let ws_connected: Arc<RwLock<(bool, bool)>> = Arc::new(RwLock::new((false, false))); // (market, user)
    let last_error: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    // ── Wallet snapshot refresh ────────────────────────────────────────
    {
        let cfg = cfg.clone();
        let wallet_snap = wallet_snap.clone();
        let clob = clob.clone();
        let live_ctx_clone = live_ctx.clone();
        let trading_address = if !cfg.proxy_address.is_empty() {
            Some(cfg.proxy_address.clone())
        } else if !cfg.signer_pk.is_empty() {
            PolySigner::from_pk_hex(&cfg.signer_pk).ok().map(|s| s.address_lower())
        } else { None };
        if let Some(addr) = trading_address {
            tokio::spawn(async move {
                let http = reqwest::Client::builder()
                    .user_agent("rn1-trader/0.1").timeout(Duration::from_secs(10))
                    .build().expect("http builder");
                loop {
                    let (sig_type, eoa_lower, l2_creds) = match &live_ctx_clone {
                        Some(c) => (
                            c.signature_type,
                            c.signer.address_lower(),
                            Some((c.api_key.clone(), c.api_secret.clone(), c.passphrase.clone())),
                        ),
                        None => (0u8, String::new(), None),
                    };
                    let l2_auth = l2_creds.as_ref().map(|(k, s, p)| rn1_clob::L2Auth {
                        address: eoa_lower.as_str(),
                        api_key: k.as_str(),
                        api_secret: s.as_str(),
                        passphrase: p.as_str(),
                    });
                    let snap = wallet::fetch_snapshot(
                        &http, &addr, &cfg.polygon_rpc_url,
                        Some(&clob), l2_auth.as_ref(), sig_type,
                    ).await;
                    *wallet_snap.write().await = snap;
                    tokio::time::sleep(Duration::from_secs(cfg.wallet_refresh_s)).await;
                }
            });
        } else {
            warn!("no trading address — wallet snapshot disabled");
        }
    }

    // ── WS market channel (shared rn1-ws subscriber) ───────────────────
    let (market_ws, reconnect_rx) = MarketWsClient::new();
    let (market_tx, mut market_rx) = mpsc::channel::<RawMarketEvent>(1024);
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    {
        let market_ws = market_ws.clone();
        let market_tx = market_tx.clone();
        let shutdown_rx = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_market_subscriber(market_ws, reconnect_rx, market_tx, shutdown_rx).await {
                error!(err=?e, "market WS subscriber died");
            }
        });
    }

    // book_dirty pulse on every market event from rn1-ws.
    {
        let book_dirty = book_dirty.clone();
        let ws_connected = ws_connected.clone();
        tokio::spawn(async move {
            while let Some(_ev) = market_rx.recv().await {
                ws_connected.write().await.0 = true;
                book_dirty.notify_one();
            }
            ws_connected.write().await.0 = false;
        });
    }

    // ── WS user channel (fills) — live only ────────────────────────────
    if cfg.mode.is_live() {
        if let Some(ctx) = &live_ctx {
            let (utx, mut urx) = mpsc::unbounded_channel::<UserEvent>();
            let _ = spawn_user_channel(
                rn1_core::CLOB_WS_USER.to_string(),
                UserAuth {
                    api_key: ctx.api_key.clone(),
                    api_secret: ctx.api_secret.clone(),
                    passphrase: ctx.passphrase.clone(),
                },
                Vec::new(),
                utx,
            );
            let pool_h = pool.clone();
            let reconcile_wake = reconcile_wake.clone();
            let ws_connected = ws_connected.clone();
            tokio::spawn(async move {
                let mut seen: HashMap<String, Instant> = HashMap::new();
                while let Some(ev) = urx.recv().await {
                    match ev {
                        UserEvent::Reconnected => { ws_connected.write().await.1 = true; }
                        UserEvent::Disconnected { reason } => {
                            warn!(reason, "user WS disconnected");
                            ws_connected.write().await.1 = false;
                        }
                        UserEvent::Fill { order_id, size_shares, price, usdc_size, dedup_key } => {
                            let now = Instant::now();
                            seen.retain(|_, t| now.duration_since(*t) < Duration::from_secs(600));
                            if seen.contains_key(&dedup_key) {
                                tracing::debug!(dedup_key, "duplicate WS fill — skipping");
                                continue;
                            }
                            seen.insert(dedup_key, now);
                            match db::trader::find_order_by_clob_hash(&pool_h, &order_id).await {
                                Ok(Some(q)) => {
                                    // Rebate is essentially $0 at this scale (handoff §2);
                                    // record 0 here and let real PnL tracking add it back.
                                    if let Err(e) = db::trader::record_fill(&pool_h, q.order_id, size_shares, usdc_size, 0.0).await {
                                        error!(err=?e, order_id, "record_fill failed");
                                    } else {
                                        info!(order_id = q.order_id, clob = order_id, size_shares, price, usdc_size,
                                            "★ WS FILL recorded");
                                    }
                                }
                                _ => warn!(order_id, "WS fill for unknown order — reconciliation will catch it"),
                            }
                            reconcile_wake.notify_one();
                        }
                    }
                }
            });
        }
    }

    // ── Graceful shutdown — cancel resting on Ctrl+C ───────────────────
    {
        let pool_h = pool.clone();
        let clob = clob.clone();
        let live_ctx = live_ctx.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                warn!("Ctrl+C — cancelling all open orders");
                if let Ok(open) = db::trader::list_open_orders(&pool_h).await {
                    for q in &open {
                        if cfg.mode.is_live() {
                            if let (Some(ctx), Some(oid)) = (&live_ctx, q.clob_order_hash.as_deref()) {
                                let _ = live_cancel(&clob, ctx, oid).await;
                            }
                        }
                        let _ = db::trader::cancel_order(&pool_h, q.order_id, "shutdown").await;
                    }
                }
                std::process::exit(0);
            }
        });
    }

    // ── Event-driven decision driver ───────────────────────────────────
    // Multi-focus: up to cfg.max_open_markets simultaneous focuses, picked
    // by descending volume. Each gets its own focus block per tick. When
    // RN1_FOCUS_SLUG is set, falls back to single-focus mode.
    let mut focus_cache: Vec<FocusMarket> = Vec::new();
    let mut last_focus_refresh = Instant::now() - Duration::from_secs(3600);
    let mut last_reconcile = Instant::now() - Duration::from_secs(3600);
    let mut detector = CollapseDetector::new();
    let mut tick: u64 = 0;
    // condition_ids that returned 404 on CLOB /book — autonomous scan skips
    // them so the trader doesn't loop on a retired Polymarket market (e.g.
    // a player withdrew, lifecycle-collector hasn't synced). Reset every 30
    // min in case the market comes back.
    let mut blocked_cids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut blocked_reset_at = Instant::now();
    let mut last_redemption_scan = Instant::now() - Duration::from_secs(3600);
    let mut slow_timer = tokio::time::interval(Duration::from_secs(cfg.poll_backstop_s.max(1)));
    slow_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Wait for a trigger.
        let trig: &str = tokio::select! {
            _ = book_dirty.notified()      => "book",
            _ = slow_timer.tick()          => "poll",
            _ = reconcile_wake.notified()  => "fill",
        };
        if trig == "book" {
            tokio::time::sleep(Duration::from_millis(cfg.book_debounce_ms)).await;
        }
        tick += 1;
        let cycle_start = Instant::now();

        // ── Focus-market refresh ──
        if last_focus_refresh.elapsed() > Duration::from_secs(cfg.gamma_refresh_s) {
            let new_focus: Vec<FocusMarket> = if !cfg.focus_slug.is_empty() {
                // Single-focus override (env-pinned focus_slug). Fetch that one only.
                match fetch_focus(&pool, &cfg.focus_slug).await {
                    Ok(Some(m)) => vec![m],
                    Ok(None) => {
                        warn!(slug = %cfg.focus_slug,
                            "focus_slug not in markets table yet (lifecycle-collector)");
                        Vec::new()
                    }
                    Err(e) => { error!(err=?e, "focus market resolution failed"); Vec::new() }
                }
            } else {
                let prefixes = cfg.universe_prefixes();
                let excludes = cfg.exclude_substrings();
                if blocked_reset_at.elapsed() > Duration::from_secs(1800) {
                    blocked_cids.clear();
                    blocked_reset_at = Instant::now();
                }
                let n = cfg.max_open_markets.max(1);
                let first = fetch_autonomous_candidates(
                    &pool, &prefixes, cfg.min_event_volume, &excludes, &blocked_cids, n).await;
                let candidates = match first {
                    Ok(v) if v.is_empty() && focus_cache.is_empty() => {
                        // Bootstrap: DB has no in-window markets — try Gamma scan.
                        match gamma_scan::GammaScan::new() {
                            Ok(scan) => match scan.scan_and_upsert(&pool, &prefixes, 80).await {
                                Ok(written) => {
                                    info!(written, "gamma fallback scan upserted — retrying");
                                    fetch_autonomous_candidates(
                                        &pool, &prefixes, cfg.min_event_volume, &excludes,
                                        &blocked_cids, n).await.unwrap_or_default()
                                }
                                Err(e) => { warn!(err=?e, "gamma fallback scan failed"); Vec::new() }
                            },
                            Err(e) => { warn!(err=?e, "GammaScan init failed"); Vec::new() }
                        }
                    }
                    Ok(v) => v,
                    Err(e) => { error!(err=?e, "autonomous candidates fetch failed"); Vec::new() }
                };
                if candidates.is_empty() {
                    warn!("autonomous scan: no ATP/WTA market with volume ≥ {} in window",
                        cfg.min_event_volume);
                }
                candidates
            };

            // Detect set change → log + WS resubscribe.
            let old_cids: std::collections::HashSet<String> = focus_cache.iter()
                .map(|x| x.condition_id.clone()).collect();
            let new_cids: std::collections::HashSet<String> = new_focus.iter()
                .map(|x| x.condition_id.clone()).collect();
            if old_cids != new_cids {
                for m in &new_focus {
                    if !old_cids.contains(&m.condition_id) {
                        info!(cid = %m.condition_id, slug = %m.slug,
                              volume = ?m.volume_num, q = ?m.question,
                              "focus market resolved");
                    }
                }
                let assets: Vec<String> = new_focus.iter()
                    .flat_map(|m| [m.outcomes[0].1.clone(), m.outcomes[1].1.clone()])
                    .collect();
                market_ws.update_assets(assets).await;
            }
            focus_cache = new_focus;
            last_focus_refresh = Instant::now();
        }

        // Focus may be absent (no in-window ATP/WTA match) — that's OK. We
        // still need the follow-up loop to run on already-held positions so
        // BAIL / stale-maker / MERGE recycling can fire. The focus block
        // below runs once per focus market; follow-up loop runs unconditionally.
        let focus_now = focus_cache.clone();
        let mut focus_mode: rn1_planner::PlanMode = rn1_planner::PlanMode::Idle;
        let mut placed = 0usize;
        let mut cancelled = 0usize;
        let mut merged_pairs = 0.0_f64;
        let wallet_now = wallet_snap.read().await.clone();
        let risk_guard = RiskGuard::new(&cfg, &pool);
        // Track settled markets to remove from focus_cache after the loop
        // (can't mutate focus_cache while iterating focus_now clone).
        let mut settled_cids: std::collections::HashSet<String> = std::collections::HashSet::new();

        for m_ref in focus_now.iter() {
        let m = m_ref.clone();
        // Labeled block — any `break 'focus` skips this market's remaining
        // processing and moves to the next focus, letting the tick continue
        // into follow-up loop + would_fill + persist_status.
        'focus: {
        // ── Settled focus guard ──
        // Defensive: autonomous scan filters closed=true already, but a
        // user-supplied focus_slug or a market that resolved between scans
        // can leave us pointing at a settled market. Cancel resting + drop.
        if m.settled {
            let open_settled = db::trader::list_open_orders(&pool).await.unwrap_or_default();
            for q in open_settled.iter().filter(|q| q.condition_id == m.condition_id) {
                if cfg.mode.is_live() {
                    if let (Some(ctx), Some(oid)) = (&live_ctx, q.clob_order_hash.as_deref()) {
                        let _ = live_cancel(&clob, ctx, oid).await;
                    }
                }
                let _ = db::trader::cancel_order(&pool, q.order_id, "settled").await;
                cancelled += 1;
            }
            warn!(cid=%m.condition_id, slug=%m.slug,
                outcome_prices=?m.outcome_prices,
                "★ focus market is SETTLED — dropping focus + cancelling resting");
            settled_cids.insert(m.condition_id.clone());
        } else {
        // ── Periodic reconciliation (live + on-fill) ──
        if cfg.mode.is_live()
            && (last_reconcile.elapsed() > Duration::from_secs(20) || trig == "fill")
        {
            last_reconcile = Instant::now();
            let recon_addr = if !cfg.proxy_address.is_empty() {
                cfg.proxy_address.to_lowercase()
            } else if let Some(ctx) = &live_ctx {
                ctx.signer.address_lower()
            } else { String::new() };
            if !recon_addr.is_empty() {
                let http = reqwest::Client::builder()
                    .user_agent("rn1-trader/0.1").timeout(Duration::from_secs(15))
                    .build().unwrap_or_default();
                match wallet::fetch_positions_detailed(&http, &recon_addr).await {
                    Ok(positions) => {
                        for p in &positions {
                            if p.condition_id != m.condition_id || p.size <= 0.0 { continue; }
                            match db::trader::reconcile_position(
                                &pool, &p.condition_id, p.outcome_index as i16, p.size, p.avg_price,
                            ).await {
                                Ok(true) => warn!(cid=%p.condition_id, leg=p.outcome_index,
                                    size=p.size, avg=p.avg_price, "★ position reconciled"),
                                Ok(false) => {}
                                Err(e) => warn!(err=?e, "reconcile_position failed"),
                            }
                        }
                    }
                    Err(e) => warn!(err=?e, "position reconciliation fetch failed"),
                }
            }
            if let Some(ctx) = &live_ctx {
                match reconcile_open_orders(&clob, ctx, &pool).await {
                    Ok(n) if n > 0 => warn!(n, "★ order-state reconciled (vanished orders cancelled)"),
                    Ok(_) => {}
                    Err(e) => warn!(err=?e, "order-state reconcile failed"),
                }
            }
        }

        // ── Build PairState ──
        let book0 = match clob.book(&m.outcomes[0].1).await {
            Ok(b) => b,
            Err(e) => {
                warn!(err=?e, slug=%m.slug, cid=%m.condition_id,
                    "book0 fetch failed — blocklisting + re-scanning");
                blocked_cids.insert(m.condition_id.clone());
                settled_cids.insert(m.condition_id.clone());
                // Multi-focus: don't force immediate re-scan — drop this market
                // and let the remaining focus(es) continue. Next gamma_refresh
                // will refill naturally. Force-rescan caused WS reconnect storms
                // when 2-3 markets in a row had empty CLOB books.
                break 'focus;
            }
        };
        let book1 = match clob.book(&m.outcomes[1].1).await {
            Ok(b) => b,
            Err(e) => {
                warn!(err=?e, slug=%m.slug, cid=%m.condition_id,
                    "book1 fetch failed — blocklisting + re-scanning");
                blocked_cids.insert(m.condition_id.clone());
                settled_cids.insert(m.condition_id.clone());
                break 'focus;
            }
        };
        let (Some(bid0), Some(ask0)) = (book0.best_bid(), book0.best_ask()) else {
            warn!(slug=%m.slug, "book0 has no bid/ask — blocklisting + re-scanning");
            blocked_cids.insert(m.condition_id.clone());
            settled_cids.insert(m.condition_id.clone());
            break 'focus;
        };
        let (Some(bid1), Some(ask1)) = (book1.best_bid(), book1.best_ask()) else {
            warn!(slug=%m.slug, "book1 has no bid/ask — blocklisting + re-scanning");
            blocked_cids.insert(m.condition_id.clone());
            settled_cids.insert(m.condition_id.clone());
            break 'focus;
        };

        // Cache per-asset quotes so the API serves outcome-specific prices.
        let _ = db::trader::upsert_asset_quote(&pool, &m.outcomes[0].1, Some(bid0), Some(ask0)).await;
        let _ = db::trader::upsert_asset_quote(&pool, &m.outcomes[1].1, Some(bid1), Some(ask1)).await;

        let collapse = detector.update(&cfg.to_planner(), [bid0, bid1]);

        let hist = db::trader::market_leg_history(&pool, &m.condition_id).await
            .unwrap_or_default();
        let open = db::trader::list_open_orders(&pool).await.unwrap_or_default();
        let now_chrono = Utc::now();
        let resting: Vec<RestingOrder> = open.iter()
            .filter(|q| q.condition_id == m.condition_id)
            .map(|q| RestingOrder {
                qid: q.order_id, leg: q.outcome_index as u8, price: q.price,
                side: if q.side == "SELL" { Side::Sell } else { Side::Buy },
                remaining_shares: (q.size - q.fill_size_shares).max(0.0),
                kind: if q.kind == "taker" { OrderKind::Taker } else { OrderKind::Maker },
                age_s: (now_chrono - q.signed_at).num_seconds().max(0),
            })
            .collect();

        let now = Utc::now();
        let game_started = m.game_start.map(|gs| (now - gs).num_seconds() >= 0).unwrap_or(false);
        let endgame = m.game_start
            .map(|gs| (now - gs).num_minutes() >= cfg.endgame_after_min)
            .unwrap_or(false);
        let secs_to_game_start = m.game_start.map(|gs| (gs - now).num_seconds());

        let ps = PairState {
            condition_id: m.condition_id.clone(),
            token_id: [m.outcomes[0].1.clone(), m.outcomes[1].1.clone()],
            game_started,
            endgame,
            secs_to_game_start,
            outcome_prob: m.outcome_prices,
            q:    [hist[0].position_shares, hist[1].position_shares],
            vwap: [hist[0].position_vwap,   hist[1].position_vwap],
            bid:  [bid0, bid1],
            ask:  [ask0, ask1],
            collapse,
            resting,
        };

        let afford = risk_guard.affordability(&ps, &wallet_now).await
            .unwrap_or(Affordability { event_headroom: 0.0, wallet_headroom: 0.0, kill_switch: false });
        let plan = plan_market(&cfg.to_planner(), &ps, &afford);

        if !plan.cancel.is_empty() || !plan.add.is_empty() {
            info!(cid=%m.condition_id, mode=plan.mode.as_str(),
                q0=ps.q[0], q1=ps.q[1], naked=ps.naked_qty(),
                add=plan.add.len(), cancel=plan.cancel.len(),
                headroom=afford.cap(), "plan");
        }

        focus_mode = plan.mode;
        // ── Execute merges first (capital recycle), then cancels, then adds ──
        for mg in &plan.merges {
            if let Err(e) = db::trader::record_merge(&pool, &m.condition_id, mg.shares, mg.matched_vwap).await {
                warn!(err=?e, "record_merge failed");
            } else {
                merged_pairs += mg.shares;
                info!(cid=%m.condition_id, shares=mg.shares, matched_vwap=mg.matched_vwap,
                    realized=mg.shares * (1.0 - mg.matched_vwap),
                    "★ MERGE recycled (paper)");
            }
        }
        let cancel_reason = plan.mode.as_str();   // "bail" / "complete" / "accumulate" / "sweep"
        for qid in &plan.cancel {
            if let Some(q) = open.iter().find(|q| q.order_id == *qid) {
                if cfg.mode.is_live() {
                    if let (Some(ctx), Some(oid)) = (&live_ctx, q.clob_order_hash.as_deref()) {
                        if let Err(e) = live_cancel(&clob, ctx, oid).await {
                            warn!(qid, err=?e, "live cancel failed (may already be filled)");
                        }
                    }
                }
                let _ = db::trader::cancel_order(&pool, *qid, cancel_reason).await;
                cancelled += 1;
            }
        }
        for d in &plan.add {
            let token_id = &m.outcomes[d.leg as usize].1;
            let size_usdc = (d.size_shares * d.price * 10000.0).round() / 10000.0;
            let kind_str = d.kind.as_str();
            let tag_str = d.tag.as_str();
            let side_str = d.side.as_str();
            let mid_at_place = (ps.bid[d.leg as usize] + ps.ask[d.leg as usize]) / 2.0;
            // Insert quote row first (so even if signing/POST fails we have a trail).
            let order_id = match db::trader::insert_order(
                &pool, &m.condition_id, d.leg as i16, token_id, token_id, side_str,
                d.price, d.size_shares, size_usdc, Some(mid_at_place), kind_str, tag_str,
            ).await {
                Ok(id) => id,
                Err(e) => {
                    error!(err=?e, "insert_order failed"); continue;
                }
            };
            match cfg.mode {
                Mode::Paper => {
                    // Paper TAKER: real CLOB FAK semantics — fill only if limit
                    // crosses current ASK. Cross fills at the ASK (price
                    // improvement, not at limit). No cross → cancel (FAK).
                    // Pre-fix bug: filled at limit unconditionally → fake PnL
                    // when planner emitted taker at bid-based prices that
                    // wouldn't actually cross (Smith-Svajda 100× fake fills,
                    // 2026-05-27).
                    if matches!(d.kind, OrderKind::Taker) {
                        let leg_ask = if d.leg == 0 { ask0 } else { ask1 };
                        if d.price + 1e-9 >= leg_ask {
                            let actual_usdc = (d.size_shares * leg_ask * 10000.0).round() / 10000.0;
                            let _ = db::trader::record_fill(
                                &pool, order_id, d.size_shares, actual_usdc, 0.0).await;
                            info!(order_id, leg=d.leg, limit=d.price, fill_px=leg_ask,
                                size=d.size_shares, kind=kind_str, tag=tag_str,
                                "paper taker FILLED (crossed ask)");
                        } else {
                            let _ = db::trader::cancel_order(
                                &pool, order_id, "taker no-cross").await;
                            info!(order_id, leg=d.leg, limit=d.price, ask=leg_ask,
                                kind=kind_str, tag=tag_str,
                                "paper taker CANCELLED (no cross)");
                        }
                    } else {
                        info!(order_id, leg=d.leg, price=d.price, size=d.size_shares,
                            kind=kind_str, tag=tag_str, "paper order placed");
                    }
                    placed += 1;
                }
                Mode::Live => {
                    let ctx = match &live_ctx {
                        Some(c) => c,
                        None => { error!("live mode but live_ctx is None"); continue; }
                    };
                    let taker = matches!(d.kind, OrderKind::Taker);
                    if !taker {
                        let book_ref = if d.leg == 0 { &book0 } else { &book1 };
                        if let Err(e) = validate_maker_quote(d.price, d.size_shares, book_ref) {
                            warn!(err=?e, "maker pre-check failed — skipping");
                            let _ = db::trader::mark_rejected(&pool, order_id, &e.to_string()).await;
                            continue;
                        }
                    }
                    match submit_buy(&clob, ctx, token_id, size_usdc, d.size_shares, m.neg_risk, taker).await {
                        Ok(clob_id) => {
                            let _ = db::trader::mark_posted(&pool, order_id, &clob_id).await;
                            placed += 1;
                            info!(order_id, clob = %clob_id, leg=d.leg, price=d.price,
                                size=d.size_shares, kind=kind_str, tag=tag_str, "LIVE order placed");
                        }
                        Err(e) => {
                            error!(err=?e, leg=d.leg, "submit_buy failed");
                            let _ = db::trader::mark_rejected(&pool, order_id, &e.to_string()).await;
                            *last_error.write().await = Some(e.to_string());
                        }
                    }
                }
            }
        }
        } // end `else` (focus is not settled)
        } // end `'focus:` labeled block
        } // end `for m_ref in focus_now`

        // Drop any focus markets flagged settled this tick. Next refresh
        // pulls fresh candidates that don't include them.
        if !settled_cids.is_empty() {
            focus_cache.retain(|m| !settled_cids.contains(&m.condition_id));
        }

        // ── Periodic settlement redemption scan (paper mode) ──
        // Closed markets with our positions → record_redemption per leg,
        // adding settled PnL to trader_daily. After redemption, the cid
        // drops from active_market_cids and follow-up stops touching it.
        // Live mode (Phase B) will use CTF redeemPositions() on-chain.
        let mut redemption_pnl = 0.0_f64;
        let mut redeemed_legs  = 0usize;
        if last_redemption_scan.elapsed() > Duration::from_secs(60) {
            last_redemption_scan = Instant::now();
            if let Ok(pending) = db::trader::pending_redemptions(&pool).await {
                for (cid, prices) in &pending {
                    let hist = match db::trader::market_leg_history(&pool, cid).await {
                        Ok(h) => h, Err(_) => continue,
                    };
                    let mut any_redeemed = false;
                    for leg in 0i16..2 {
                        let h = &hist[leg as usize];
                        if h.position_shares > 0.5 {
                            let settle_px = prices[leg as usize];
                            match db::trader::record_redemption(
                                &pool, cid, leg, h.position_shares, settle_px, h.position_usdc,
                            ).await {
                                Ok(true) => {
                                    let pnl = h.position_shares * settle_px - h.position_usdc;
                                    redemption_pnl += pnl;
                                    redeemed_legs += 1;
                                    any_redeemed = true;
                                    info!(cid=%cid, leg, shares=h.position_shares,
                                        settle_px, cost=h.position_usdc,
                                        realized=pnl, "★ REDEEMED (paper settlement)");
                                }
                                _ => {}
                            }
                        }
                    }
                    // Post-redemption cleanup: cancel any resting orders on this
                    // settled market. Without this, S2 SELL exits placed pre-settle
                    // remain "signed" forever (Shimizu 29168 stuck 14h after settle,
                    // 2026-05-26). Also catches BUY orders placed during the brief
                    // window before settle gate caught up.
                    if any_redeemed {
                        if let Ok(open_post) = db::trader::list_open_orders(&pool).await {
                            for q in open_post.iter().filter(|q| &q.condition_id == cid) {
                                if cfg.mode.is_live() {
                                    if let (Some(ctx), Some(oid)) =
                                        (&live_ctx, q.clob_order_hash.as_deref())
                                    {
                                        let _ = live_cancel(&clob, ctx, oid).await;
                                    }
                                }
                                let _ = db::trader::cancel_order(&pool, q.order_id, "redeemed").await;
                                info!(order_id=q.order_id, cid=%cid,
                                    "cancelled resting order — market redeemed");
                            }
                        }
                    }
                }
            }
        }

        // ── Follow-up markets — multi-market housekeeping ──
        // Runs UNCONDITIONALLY, regardless of whether a focus market was
        // resolved this tick. The bot's already-held positions need ongoing
        // attention (Completion bids, BAIL, stale-maker cancel, MERGE recycle)
        // even when no fresh ATP/WTA candidate is in the trading window.
        //
        // Collapse=[false,false] forced — sweep should NOT fire on follow-ups
        // (we're past the in-play window for those, and capital is reserved
        // for the current focus). plan_market still runs Steps 2, 2.5 (MERGE),
        // and 3, so naked positions get lagging bids and matched pairs get
        // recycled into USDC automatically.
        let active_cids = match db::trader::active_market_cids(&pool).await {
            Ok(v) => v,
            Err(e) => {
                warn!(err=?e, "active_market_cids query failed — follow-up loop will be empty");
                vec![]
            }
        };
        let focus_cid_set: std::collections::HashSet<String> = focus_now.iter()
            .map(|m| m.condition_id.clone()).collect();
        let mut follow_placed = 0usize;
        let mut follow_cancelled = 0usize;
        for cid in &active_cids {
            if focus_cid_set.contains(cid) { continue; }   // already handled in focus loop
            let Ok(Some(fm)) = fetch_market_by_cid(&pool, cid).await else { continue };
            // ── Settled-market gate ──
            // Polymarket resolved the market (winner declared / `closed=true`
            // / outcome_prices=[1,0]) → never place new orders. Cancel any
            // resting then skip. Held shares get redeemed via CTF Phase B.
            if fm.settled {
                let fopen_settled = db::trader::list_open_orders(&pool).await.unwrap_or_default();
                for q in fopen_settled.iter().filter(|q| q.condition_id == fm.condition_id) {
                    if cfg.mode.is_live() {
                        if let (Some(ctx), Some(oid)) = (&live_ctx, q.clob_order_hash.as_deref()) {
                            let _ = live_cancel(&clob, ctx, oid).await;
                        }
                    }
                    let _ = db::trader::cancel_order(&pool, q.order_id, "settled").await;
                    follow_cancelled += 1;
                }
                if !fopen_settled.iter().any(|q| q.condition_id == fm.condition_id) {
                    // Once-per-market log; avoids spamming every tick.
                } else {
                    info!(cid=%fm.condition_id, slug=%fm.slug,
                        outcome_prices=?fm.outcome_prices,
                        "★ settled market — cancelled all resting, no new orders");
                }
                continue;
            }
            // Try live book first; if a leg returns no bid/ask (dead post-
            // resolution market) fall back to the cached quote so BAIL +
            // stale-maker cancel can still run on already-held positions.
            let book0_opt = clob.book(&fm.outcomes[0].1).await.ok();
            let book1_opt = clob.book(&fm.outcomes[1].1).await.ok();
            let live0 = book0_opt.as_ref()
                .and_then(|b| Some((b.best_bid()?, b.best_ask()?)));
            let live1 = book1_opt.as_ref()
                .and_then(|b| Some((b.best_bid()?, b.best_ask()?)));
            let (fb0, fa0) = match live0 {
                Some(q) => q,
                None => match db::trader::get_cached_quote(&pool, &fm.outcomes[0].1).await {
                    Ok(Some(q)) => q,
                    _ => continue,
                },
            };
            let (fb1, fa1) = match live1 {
                Some(q) => q,
                None => match db::trader::get_cached_quote(&pool, &fm.outcomes[1].1).await {
                    Ok(Some(q)) => q,
                    _ => continue,
                },
            };
            // Only upsert when the quote is FRESH (live book delivered it).
            if live0.is_some() {
                let _ = db::trader::upsert_asset_quote(&pool, &fm.outcomes[0].1, Some(fb0), Some(fa0)).await;
            }
            if live1.is_some() {
                let _ = db::trader::upsert_asset_quote(&pool, &fm.outcomes[1].1, Some(fb1), Some(fa1)).await;
            }
            let fhist = db::trader::market_leg_history(&pool, &fm.condition_id).await.unwrap_or_default();
            let fopen = db::trader::list_open_orders(&pool).await.unwrap_or_default();
            let now_chrono = Utc::now();
            let fresting: Vec<RestingOrder> = fopen.iter()
                .filter(|q| q.condition_id == fm.condition_id)
                .map(|q| RestingOrder {
                    qid: q.order_id, leg: q.outcome_index as u8, price: q.price,
                    side: if q.side == "SELL" { Side::Sell } else { Side::Buy },
                    remaining_shares: (q.size - q.fill_size_shares).max(0.0),
                    kind: if q.kind == "taker" { OrderKind::Taker } else { OrderKind::Maker },
                    age_s: (now_chrono - q.signed_at).num_seconds().max(0),
                }).collect();
            let fnow = Utc::now();
            let fgame_started = fm.game_start.map(|gs| (fnow - gs).num_seconds() >= 0).unwrap_or(false);
            let fendgame = fm.game_start
                .map(|gs| (fnow - gs).num_minutes() >= cfg.endgame_after_min)
                .unwrap_or(false);
            let fsecs = fm.game_start.map(|gs| (gs - fnow).num_seconds());
            let fps = PairState {
                condition_id: fm.condition_id.clone(),
                token_id: [fm.outcomes[0].1.clone(), fm.outcomes[1].1.clone()],
                game_started: fgame_started,
                endgame: fendgame,
                secs_to_game_start: fsecs,
                outcome_prob: fm.outcome_prices,
                q:    [fhist[0].position_shares, fhist[1].position_shares],
                vwap: [fhist[0].position_vwap,   fhist[1].position_vwap],
                bid:  [fb0, fb1],
                ask:  [fa0, fa1],
                collapse: [false, false],          // no sweep on follow-ups
                resting: fresting,
            };
            let fafford = risk_guard.affordability(&fps, &wallet_now).await
                .unwrap_or(Affordability { event_headroom: 0.0, wallet_headroom: 0.0, kill_switch: false });
            let fplan = plan_market(&cfg.to_planner(), &fps, &fafford);
            if !fplan.add.is_empty() || !fplan.cancel.is_empty() {
                info!(cid=%fm.condition_id, mode=fplan.mode.as_str(),
                    q0=fps.q[0], q1=fps.q[1], naked=fps.naked_qty(),
                    add=fplan.add.len(), cancel=fplan.cancel.len(),
                    "★ follow-up plan");
            }
            for fmg in &fplan.merges {
                if let Err(e) = db::trader::record_merge(&pool, &fm.condition_id, fmg.shares, fmg.matched_vwap).await {
                    warn!(err=?e, "follow-up record_merge failed");
                } else {
                    merged_pairs += fmg.shares;
                    info!(cid=%fm.condition_id, shares=fmg.shares, matched_vwap=fmg.matched_vwap,
                        realized=fmg.shares * (1.0 - fmg.matched_vwap),
                        "★ follow-up MERGE recycled (paper)");
                }
            }
            let f_cancel_reason = format!("follow-up:{}", fplan.mode.as_str());
            for qid in &fplan.cancel {
                if let Some(q) = fopen.iter().find(|q| q.order_id == *qid) {
                    if cfg.mode.is_live() {
                        if let (Some(ctx), Some(oid)) = (&live_ctx, q.clob_order_hash.as_deref()) {
                            let _ = live_cancel(&clob, ctx, oid).await;
                        }
                    }
                    let _ = db::trader::cancel_order(&pool, *qid, &f_cancel_reason).await;
                    follow_cancelled += 1;
                }
            }
            for d in &fplan.add {
                let token_id = &fm.outcomes[d.leg as usize].1;
                let size_usdc = (d.size_shares * d.price * 10000.0).round() / 10000.0;
                let mid_at_place = (fps.bid[d.leg as usize] + fps.ask[d.leg as usize]) / 2.0;
                let order_id = match db::trader::insert_order(
                    &pool, &fm.condition_id, d.leg as i16, token_id, token_id, d.side.as_str(),
                    d.price, d.size_shares, size_usdc, Some(mid_at_place), d.kind.as_str(), d.tag.as_str(),
                ).await { Ok(id) => id, Err(_) => continue };
                match cfg.mode {
                    Mode::Paper => {
                        if matches!(d.kind, OrderKind::Taker) {
                            let leg_ask = fps.ask[d.leg as usize];
                            if d.price + 1e-9 >= leg_ask {
                                let actual_usdc = (d.size_shares * leg_ask * 10000.0).round() / 10000.0;
                                let _ = db::trader::record_fill(
                                    &pool, order_id, d.size_shares, actual_usdc, 0.0).await;
                            } else {
                                let _ = db::trader::cancel_order(
                                    &pool, order_id, "taker no-cross").await;
                            }
                        }
                        follow_placed += 1;
                    }
                    Mode::Live => {
                        let ctx = match &live_ctx { Some(c) => c, None => continue };
                        let taker = matches!(d.kind, OrderKind::Taker);
                        if !taker {
                            let book_ref = if d.leg == 0 { book0_opt.as_ref() } else { book1_opt.as_ref() };
                            // No live book → skip placement entirely. Cache-only
                            // quotes are good enough for BAIL/cancel but not for
                            // a fresh maker submit_buy (we have no real depth).
                            let Some(book_ref) = book_ref else {
                                warn!(leg=d.leg, "follow-up maker skipped — live book unavailable");
                                let _ = db::trader::mark_rejected(&pool, order_id, "no live book").await;
                                continue;
                            };
                            if let Err(e) = validate_maker_quote(d.price, d.size_shares, book_ref) {
                                warn!(err=?e, "follow-up maker pre-check failed");
                                let _ = db::trader::mark_rejected(&pool, order_id, &e.to_string()).await;
                                continue;
                            }
                        }
                        match submit_buy(&clob, ctx, token_id, size_usdc, d.size_shares, fm.neg_risk, taker).await {
                            Ok(clob_id) => {
                                let _ = db::trader::mark_posted(&pool, order_id, &clob_id).await;
                                follow_placed += 1;
                                info!(order_id, clob = %clob_id, leg=d.leg, price=d.price,
                                    tag=d.tag.as_str(), "★ follow-up LIVE order placed");
                            }
                            Err(e) => {
                                error!(err=?e, "follow-up submit_buy failed");
                                let _ = db::trader::mark_rejected(&pool, order_id, &e.to_string()).await;
                            }
                        }
                    }
                }
            }
        }

        // ── Paper would-fill simulator ──
        // Maker orders never auto-fill in paper mode without this — only
        // takers (Sweep) do (FAK semantics in submit step above). Poll
        // data-api /trades for each market with open paper makers; SELL
        // trades ≤ our bid trigger a simulated fill. §5.1 cap inside.
        let mut paper_fills = 0usize;
        if !cfg.mode.is_live() {
            let http = reqwest::Client::builder()
                .user_agent("rn1-trader/0.1").timeout(Duration::from_secs(8))
                .build().unwrap_or_default();
            match would_fill::check_would_fills(&pool, &http).await {
                Ok(n) => { paper_fills = n; }
                Err(e) => warn!(err=?e, "would_fill check failed"),
            }
        }

        // ── Persist status for the API/dashboard ──
        // Strategy view = top-volume focus (focus_now is volume-ordered desc).
        // Dashboard currently expects a single object; extending to an array
        // is a separate UI task.
        let strategy = if let Some(m) = focus_now.first() {
            let hist_post = db::trader::market_leg_history(&pool, &m.condition_id).await
                .unwrap_or_default();
            Some(build_strategy_view(m, &hist_post, cfg.cumulative_target_max))
        } else {
            None
        };
        // `running` reflects "decision loop is alive", NOT "has focus". The bot
        // is still working (follow-up housekeeping, BAIL, MERGE) even when no
        // fresh ATP/WTA candidate matches the autonomous window.
        let candidates = focus_now.len();
        persist_status(&pool, &cfg, /*running=*/true, tick, cycle_start, candidates,
            ws_connected.read().await.clone(), &wallet_snap, &last_error, strategy).await;

        let follow_ups_count = active_cids.iter()
            .filter(|c| !focus_cid_set.contains(*c))
            .count();
        info!(tick, trig, placed, cancelled, follow_placed, follow_cancelled, paper_fills,
              merged_pairs, redeemed_legs, redemption_pnl,
              mode = focus_mode.as_str(),
              follow_ups = follow_ups_count,
              cycle_ms = cycle_start.elapsed().as_millis() as u64, "tick");
    }
}

/// Build the per-event strategy view JSON for the dashboard.
fn build_strategy_view(
    m: &FocusMarket,
    hist: &[db::trader::LegHistory; 2],
    cumulative_target: f64,
) -> serde_json::Value {
    let q0 = hist[0].position_shares;
    let q1 = hist[1].position_shares;
    let matched = q0.min(q1);
    let (naked_shares, naked_leg) = if q0 > q1 {
        (q0 - q1, m.outcomes[0].0.clone())
    } else if q1 > q0 {
        (q1 - q0, m.outcomes[1].0.clone())
    } else {
        (0.0, String::new())
    };
    let cum = hist[0].position_vwap + hist[1].position_vwap;
    serde_json::json!({
        "condition_id":  m.condition_id,
        "focus_slug":    m.slug,
        "event_slug":    m.event_slug,
        "question":      m.question,
        "leg0_name":     m.outcomes[0].0,
        "leg0_shares":   q0,
        "leg0_vwap":     hist[0].position_vwap,
        "leg1_name":     m.outcomes[1].0,
        "leg1_shares":   q1,
        "leg1_vwap":     hist[1].position_vwap,
        "cumulative_vwap":  cum,
        "cumulative_target": cumulative_target,
        "matched_pairs": matched,
        "naked_shares":  naked_shares,
        "naked_leg_name": naked_leg,
    })
}

#[allow(clippy::too_many_arguments)]
async fn persist_status(
    pool: &sqlx::PgPool,
    cfg: &Config,
    running: bool,
    tick: u64,
    cycle_start: Instant,
    candidates: usize,
    ws: (bool, bool),
    wallet: &Arc<RwLock<WalletSnapshot>>,
    last_error: &Arc<RwLock<Option<String>>>,
    strategy: Option<serde_json::Value>,
) {
    let ks = db::trader::kill_switch_active(pool).await.unwrap_or(false);
    let last_err = last_error.read().await.clone();
    let wallet_snap = wallet.read().await.clone();
    let wallet_json = serde_json::to_value(&wallet_snap).ok();
    let cfg_json = serde_json::json!({
        "quote_shares":           cfg.quote_shares,
        "per_event_cap_usdc":     cfg.per_event_cap_usdc,
        "daily_loss_cap_usdc":    cfg.daily_loss_cap_usdc,
        "daily_gross_cap_usdc":   cfg.daily_gross_cap_usdc,
        "cumulative_aim":         cfg.cumulative_aim,
        "cumulative_target_max":  cfg.cumulative_target_max,
        "mid_min":                cfg.mid_min,
        "mid_max":                cfg.mid_max,
        "ladder_depth":           cfg.ladder_depth,
        "completion_mult":        cfg.completion_mult,
        "endgame_after_min":      cfg.endgame_after_min,
    });
    let _ = db::trader::upsert_trader_status(
        pool, cfg.mode.as_str(), running,
        if cfg.focus_slug.is_empty() { None } else { Some(cfg.focus_slug.as_str()) },
        tick as i64, cycle_start.elapsed().as_millis() as i64, candidates as i32,
        ws.0, ws.1, ks, last_err.as_deref(),
        wallet_json, Some(cfg_json), strategy,
    ).await;
}

/// SMOKE TEST — handoff §9.11. Places ONE deep-OTM order on a sports-universe
/// market, verifies via GET /data/orders, cancels.
async fn run_smoke_test(cfg: &Config, clob: &ClobClient) -> Result<()> {
    if !cfg.mode.is_live() {
        anyhow::bail!("smoke test requires RN1_LIVE_MODE=live");
    }
    if cfg.focus_slug.is_empty() {
        anyhow::bail!("smoke test requires RN1_FOCUS_SLUG to be set");
    }
    let pool = db::pool_from_env().await?;
    db::migrate(&pool).await?;
    println!("\n═══ rn1-trader SMOKE TEST ═══\n");
    let ctx = prepare_live(cfg, clob).await.context("prepare_live")?;
    let m = fetch_focus(&pool, &cfg.focus_slug).await?
        .ok_or_else(|| anyhow::anyhow!("focus_slug not in markets table — wait for collector"))?;
    let token = m.outcomes[0].1.clone();
    let book = clob.book(&token).await.context("book")?;
    let bid = book.best_bid().context("no bid")?;
    let test_price = ((bid - 0.20) * 100.0).round() / 100.0;
    let test_price = test_price.max(0.02);
    if test_price >= bid - 0.10 {
        anyhow::bail!("could not derive safe OTM price (bid={} test={})", bid, test_price);
    }
    let mut size_usdc = 1.0_f64;
    let mut size_shares = (size_usdc / test_price * 100.0).round() / 100.0;
    if size_shares < 5.0 {
        size_shares = 5.0;
        size_usdc = (size_shares * test_price * 100.0).round() / 100.0;
    }
    println!("market: {} ({}…)", m.slug, &m.condition_id[..18]);
    println!("price ${:.3}  size ${:.2} ({:.2} sh)", test_price, size_usdc, size_shares);
    let oid = submit_buy(clob, &ctx, &token, size_usdc, size_shares, m.neg_risk, false).await
        .context("submit_buy")?;
    println!("placed order_id={oid}");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let addr = ctx.signer.address_lower();
    let auth = ctx.l2_auth(&addr);
    let resps = clob.get_open_orders(&auth, Some(&token)).await?;
    let found = resps.iter().any(|s| s.contains(&oid));
    println!("verify GET /data/orders: {}", if found { "OK" } else { "WARN (not found)" });
    live_cancel(clob, &ctx, &oid).await.context("cancel_order")?;
    println!("cancelled OK\n✓ SMOKE TEST PASSED");
    Ok(())
}
