//! M0.5 — RN1 Quote Lifecycle Collector
//!
//! 다음 3개 비동기 작업을 동시 가동:
//!   1. universe_loop   — 최근 N시간 RN1 활동에서 asset_ids 수집 (5min 주기)
//!   2. fills_loop      — /activity?type=TRADE 폴링 → fills 테이블 (30s 주기)
//!   3. ws_loop         — public market WS → book_events 테이블 (real-time)
//!
//! Heartbeat 는 매 cycle 마다 system_heartbeats 에 기록.

use anyhow::Result;
use chrono::{Duration as ChronoDur, Utc};
use clap::Parser;
use rn1_core::constants::*;
use rn1_data_api::{DataApi, ActivityRow};
use rn1_db as db;
use rn1_ws::{MarketWsClient, RawMarketEvent, run_market_subscriber};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn, error};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rn1-lifecycle", version, about = "M0.5 RN1 quote lifecycle collector")]
struct Args {
    /// 추적 대상 wallet (기본 RN1)
    #[arg(long, env = "RN1_WALLET", default_value = RN1_WALLET)]
    wallet: String,

    /// universe 결정에 쓰는 lookback (시간)
    #[arg(long, env = "RN1_LOOKBACK_H", default_value_t = 6)]
    lookback_h: i64,

    /// universe 재평가 주기 (초)
    #[arg(long, env = "RN1_UNIVERSE_REFRESH_S", default_value_t = 300)]
    universe_refresh_s: u64,

    /// /activity 폴링 주기 (초)
    #[arg(long, env = "RN1_FILLS_POLL_S", default_value_t = 30)]
    fills_poll_s: u64,

    /// WS 구독 최대 asset 수 (rate-limit 고려)
    #[arg(long, env = "RN1_MAX_ASSETS", default_value_t = 300)]
    max_assets: usize,

    /// DB URL (override)
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
    info!(wallet = %args.wallet, "starting M0.5 lifecycle collector");

    if let Some(url) = &args.database_url { std::env::set_var("DATABASE_URL", url); }
    let pool = db::pool_from_env().await?;
    info!("db pool ready, running migrations");
    db::migrate(&pool).await?;

    let api = DataApi::new()?;
    let (ws_client, reconnect_rx) = MarketWsClient::new();
    let (event_tx, mut event_rx) = mpsc::channel::<RawMarketEvent>(4096);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // ---- 1. universe loop ----
    let universe_task = {
        let api = api.clone();
        let ws_client = ws_client.clone();
        let pool = pool.clone();
        let args = args.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                let now = Utc::now().timestamp();
                let start = now - args.lookback_h * 3600;
                match api.activity_window(&args.wallet, "TRADE", start, now + 30).await {
                    Ok(rows) => {
                        let mut assets: Vec<String> = Vec::new();
                        let mut seen: HashSet<String> = HashSet::new();
                        for r in &rows {
                            if seen.insert(r.asset.clone()) {
                                assets.push(r.asset.clone());
                            }
                            if assets.len() >= args.max_assets { break; }
                        }
                        info!(tick, total_fills = rows.len(), unique_assets = assets.len(),
                              "universe refresh");
                        ws_client.update_assets(assets.clone()).await;

                        // heartbeat
                        let _ = db::sync::record_heartbeat(&pool, "lifecycle.universe",
                            serde_json::json!({
                                "tick": tick,
                                "asset_count": assets.len(),
                                "recent_fills": rows.len(),
                            })).await;
                    }
                    Err(e) => warn!(err=?e, "universe fetch failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(args.universe_refresh_s)) => {}
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2. fills loop ----
    let fills_task = {
        let api = api.clone();
        let pool = pool.clone();
        let args = args.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            // bootstrap cursor
            let mut last_ts: i64 = db::sync::get_cursor(&pool, "lifecycle.fills.last_ts")
                .await.ok().flatten()
                .unwrap_or_else(|| Utc::now().timestamp() - 60);
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                let now = Utc::now().timestamp();
                let start = last_ts - 5;
                let end = now + 30;
                match api.activity_window(&args.wallet, "TRADE", start, end).await {
                    Ok(mut rows) => {
                        // ascending by ts. cursor 비교는 strict `<` 로 — 같은
                        // timestamp 의 여러 fills (price/size 다름) 가 한 tx 안에
                        // 묶일 때, `<=` 로 비교하면 첫 fill 후 cursor 가 그 ts 로
                        // 갱신되어 나머지가 모두 skip 되는 버그.
                        // unique (tx_hash, cid, oidx, side, size, price, ts) 가
                        // 진짜 dedupe 책임. cursor 는 loop 후 한 번만 갱신.
                        rows.sort_by_key(|r| r.timestamp);
                        // new_count = 진짜 새로 들어간 fill 만. boundary row
                        // (ts == last_ts) 는 late-indexed 같은-ts fill 을 잡으려고
                        // 매 cycle 재조회되는데, conflict 면 insert 가 false 를
                        // 돌려주므로 new_count 에 안 잡힌다 — drought 가 로그에
                        // new_fills=1 로 위장되던 버그 수정.
                        let mut new_count = 0usize;
                        let mut max_ts = last_ts;
                        for r in &rows {
                            if r.timestamp < last_ts { continue; }
                            match insert_observed_fill(&pool, r).await {
                                Ok(inserted) => {
                                    if r.timestamp > max_ts { max_ts = r.timestamp; }
                                    if inserted { new_count += 1; }
                                }
                                Err(e) => warn!(err=?e, "fill insert failed"),
                            }
                        }
                        let cursor_moved = max_ts > last_ts;
                        last_ts = max_ts;
                        if cursor_moved {
                            let _ = db::sync::set_cursor(&pool, "lifecycle.fills.last_ts", last_ts).await;
                        }
                        if new_count > 0 {
                            info!(tick, new_fills = new_count, cursor = last_ts, "fills polled");
                        }
                        let _ = db::sync::record_heartbeat(&pool, "lifecycle.fills",
                            serde_json::json!({
                                "tick": tick,
                                "last_ts": last_ts,
                                "new_in_tick": new_count,
                            })).await;
                    }
                    Err(e) => warn!(err=?e, "fills fetch failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(args.fills_poll_s)) => {}
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2b. positions snapshot loop ----
    // RN1 의 현재 보유 portfolio 를 주기적으로 가져와 positions 테이블에 upsert.
    // 이 데이터가 없으면 frontend 의 Positions Tree 가 빈 상태로 표시됨.
    let positions_task = {
        let api = api.clone();
        let pool = pool.clone();
        let args = args.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                match api.positions_all(&args.wallet).await {
                    Ok(rows) => {
                        let total = rows.len();
                        let mut written = 0usize;
                        for r in rows {
                            let p = r.into_position(None);
                            if let Err(e) = db::positions::upsert(&pool, &p).await {
                                warn!(err=?e, "position upsert failed");
                            } else {
                                written += 1;
                            }
                        }
                        info!(tick, total, written, "positions snapshot");
                        let _ = db::sync::record_heartbeat(&pool, "lifecycle.positions",
                            serde_json::json!({ "tick": tick, "total": total, "written": written })).await;
                    }
                    Err(e) => warn!(err=?e, "positions fetch failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2d. PnL snapshot — 5min 주기로 positions/fills 의 합계 snapshot 을
    //          pnl_snapshots 테이블에 저장. Profit/Loss 그래프의 데이터 원천.
    let pnl_snap_task = {
        let pool = pool.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                // 두 scope 동시 INSERT — 'all' (전체) + 'recent' (24h activity 한정).
                // recent_cids 는 위치 tree filter 와 동일 정의.
                let res = sqlx::query(r#"
                    WITH recent_cids AS (
                        SELECT DISTINCT condition_id FROM fills
                        WHERE is_ours=false AND ts > NOW() - INTERVAL '24 hours'
                    )
                    INSERT INTO pnl_snapshots (
                        ts, scope, gross_buy, gross_sell,
                        realized_pnl, unrealized, merge_locked, rebate,
                        trade_count, active_orders
                    )
                    SELECT date_trunc('minute', NOW()), 'all',
                        COALESCE((SELECT SUM(usdc_size) FROM fills WHERE is_ours=false AND side='BUY'),  0),
                        COALESCE((SELECT SUM(usdc_size) FROM fills WHERE is_ours=false AND side='SELL'), 0),
                        COALESCE((SELECT SUM(realized_pnl) FROM positions), 0),
                        COALESCE((SELECT SUM(cash_pnl)     FROM positions), 0),
                        0, 0,
                        (SELECT COUNT(*) FROM fills WHERE is_ours=false)::int,
                        0
                    UNION ALL
                    SELECT date_trunc('minute', NOW()), 'recent',
                        COALESCE((SELECT SUM(usdc_size) FROM fills WHERE is_ours=false AND side='BUY'
                                  AND ts > NOW() - INTERVAL '24 hours'), 0),
                        COALESCE((SELECT SUM(usdc_size) FROM fills WHERE is_ours=false AND side='SELL'
                                  AND ts > NOW() - INTERVAL '24 hours'), 0),
                        COALESCE((SELECT SUM(realized_pnl) FROM positions
                                  WHERE condition_id IN (SELECT condition_id FROM recent_cids)), 0),
                        COALESCE((SELECT SUM(cash_pnl)     FROM positions
                                  WHERE condition_id IN (SELECT condition_id FROM recent_cids)), 0),
                        0, 0,
                        (SELECT COUNT(*) FROM fills WHERE is_ours=false
                         AND ts > NOW() - INTERVAL '24 hours')::int,
                        0
                    ON CONFLICT (ts, scope) DO UPDATE SET
                        gross_buy = EXCLUDED.gross_buy,
                        gross_sell = EXCLUDED.gross_sell,
                        realized_pnl = EXCLUDED.realized_pnl,
                        unrealized = EXCLUDED.unrealized,
                        trade_count = EXCLUDED.trade_count
                "#).execute(&pool).await;
                match res {
                    Ok(_) => {
                        let _ = db::sync::record_heartbeat(&pool, "lifecycle.pnl_snap",
                            serde_json::json!({ "tick": tick })).await;
                        if tick % 12 == 0 { info!(tick, "pnl snapshot written"); }
                    }
                    Err(e) => warn!(err=?e, "pnl snapshot failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {} // 5min
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2c. markets metadata sync — fills 에 나타난 cid 의 Gamma 메타를
    //          markets 테이블에 채워줘서 spread/volume 등 진입 메트릭 사용 가능. ----
    let markets_meta_task = {
        let api = api.clone();
        let pool = pool.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                // 24h 안 fills 에 등장한 cid 중 markets 테이블에 없거나 24h 이상 stale 한 것
                let cids: Vec<(String,)> = sqlx::query_as(r#"
                    SELECT DISTINCT f.condition_id
                    FROM fills f
                    LEFT JOIN markets m ON m.condition_id = f.condition_id
                    WHERE f.is_ours = false
                      AND f.ts > NOW() - INTERVAL '24 hours'
                      AND (m.condition_id IS NULL OR m.refreshed_at < NOW() - INTERVAL '30 minutes')
                    LIMIT 50
                "#).fetch_all(&pool).await.unwrap_or_default();

                let mut written = 0usize;
                for (cid,) in &cids {
                    match api.fetch_market(cid).await {
                        Ok(Some(m)) => {
                            if let Err(e) = db::markets::upsert(&pool, &m).await {
                                warn!(err=?e, "market upsert failed");
                            } else {
                                written += 1;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => warn!(err=?e, "gamma fetch failed for {cid}"),
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
                if written > 0 {
                    info!(tick, candidates = cids.len(), written, "markets meta sync");
                }
                let _ = db::sync::record_heartbeat(&pool, "lifecycle.markets_meta",
                    serde_json::json!({ "tick": tick, "candidates": cids.len(), "written": written })).await;

                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2e. actions_loop — MERGE / REDEEM / SPLIT polling.
    //          /activity?type=MERGE 등으로 RN1 의 실현/페어 정산 이력 수집.
    //          W1: Forensic baseline 의 MERGE-lock 62% net alpha 를 실시간 추적하는 핵심 데이터.
    let actions_task = {
        let api = api.clone();
        let pool = pool.clone();
        let args = args.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut last_ts: i64 = db::sync::get_cursor(&pool, "lifecycle.actions.last_ts")
                .await.ok().flatten()
                .unwrap_or_else(|| Utc::now().timestamp() - 30 * 86400); // 첫 실행 시 30일 backfill
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                let now = Utc::now().timestamp();
                let mut total_written = 0usize;
                let mut max_ts_seen = last_ts;
                for kind in &["MERGE", "REDEEM", "SPLIT"] {
                    let start = last_ts - 5;
                    match api.activity_window(&args.wallet, kind, start, now + 30).await {
                        Ok(mut rows) => {
                            rows.sort_by_key(|r| r.timestamp);
                            for r in &rows {
                                if r.timestamp < last_ts { continue; }
                                match insert_action(&pool, kind, r).await {
                                    Ok(true) => total_written += 1,
                                    Ok(false) => {} // duplicate, ignored
                                    Err(e) => warn!(err=?e, kind, "action insert failed"),
                                }
                                if r.timestamp > max_ts_seen { max_ts_seen = r.timestamp; }
                            }
                        }
                        Err(e) => warn!(err=?e, kind, "actions fetch failed"),
                    }
                }
                if max_ts_seen > last_ts {
                    last_ts = max_ts_seen;
                    let _ = db::sync::set_cursor(&pool, "lifecycle.actions.last_ts", last_ts).await;
                }
                if total_written > 0 {
                    info!(tick, written = total_written, cursor = last_ts, "actions polled");
                }
                let _ = db::sync::record_heartbeat(&pool, "lifecycle.actions",
                    serde_json::json!({ "tick": tick, "last_ts": last_ts, "written": total_written })).await;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {} // 5min
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2f. retention_task — book_events 7일 보존 정책.
    //          W3: 일 ~45GB 페이스로 쌓이는 WS event 가 디스크 다 잡아먹지 않도록.
    let retention_task = {
        let pool = pool.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                // 시작 시 1분 대기 (다른 task 들 안정화 후)
                if tick == 1 {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
                // 48h 보존 (이전 7d 였으나 디스크 압박 + price_change skip 후로는 짧게 OK)
                let res = sqlx::query(
                    "DELETE FROM book_events WHERE ts < NOW() - INTERVAL '48 hours'"
                ).execute(&pool).await;
                let deleted = res.as_ref().map(|r| r.rows_affected()).unwrap_or(0);
                if let Err(e) = res {
                    warn!(err=?e, "retention cleanup failed");
                } else if deleted > 0 {
                    info!(tick, deleted, "book_events retention cleanup");
                }
                let _ = db::sync::record_heartbeat(&pool, "lifecycle.retention",
                    serde_json::json!({ "tick": tick, "deleted": deleted })).await;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {} // 1h
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 2g. maker_classify_task — fills.is_maker 추론 (2-tier).
    //          관측 fill 은 maker/taker flag 가 없어 추론 필요.
    //          Tier1: WS last_trade_price aggressor side. Tier2: fill 직전 book best_bid.
    let maker_classify_task = {
        let pool = pool.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tick += 1;
                if tick == 1 {
                    // 다른 task (WS, fills) 안정화 대기
                    tokio::time::sleep(std::time::Duration::from_secs(90)).await;
                }
                let (t1, t2) = classify_fills(&pool).await.unwrap_or_else(|e| {
                    warn!(err=?e, "maker classify failed");
                    (0, 0)
                });
                if t1 + t2 > 0 {
                    info!(tick, last_trade = t1, book = t2, "maker classify");
                }
                let _ = db::sync::record_heartbeat(&pool, "lifecycle.maker_classify",
                    serde_json::json!({ "tick": tick, "last_trade": t1, "book": t2 })).await;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(180)) => {} // 3min
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- 3. WS subscriber ----
    let ws_task = {
        let ws_client = ws_client.clone();
        let event_tx = event_tx.clone();
        let shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_market_subscriber(ws_client, reconnect_rx, event_tx, shutdown).await {
                error!(err=?e, "ws subscriber crashed");
            }
        })
    };

    // ---- 4. event sink: batch insert to book_events ----
    let sink_task = {
        let pool = pool.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut batch: Vec<db::book_events::BookEvent> = Vec::with_capacity(256);
            let mut tick: u64 = 0;
            let mut flush_interval = tokio::time::interval(std::time::Duration::from_millis(500));
            flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    Some(ev) = event_rx.recv() => {
                        // price_change 이벤트 skip — 99% 의 ingestion volume 인데 분석에서
                        // best_bid/best_ask 가 한쪽씩만 있어 거의 안 씀.
                        // book + last_trade_price 만 보관 → ~30 GB/일 → ~300 MB/일 으로 100배 절감.
                        if ev.event_type == "price_change" {
                            continue;
                        }
                        let be = db::book_events::BookEvent {
                            ts: ev.received_at,
                            asset_id: ev.asset_id,
                            condition_id: ev.condition_id,
                            event_type: ev.event_type,
                            best_bid: ev.best_bid,
                            best_ask: ev.best_ask,
                            spread: match (ev.best_bid, ev.best_ask) {
                                (Some(b), Some(a)) if a > b => Some(a - b),
                                _ => None,
                            },
                            payload: ev.payload,
                        };
                        batch.push(be);
                        if batch.len() >= 200 {
                            tick += 1;
                            let len = batch.len();
                            if let Err(e) = db::book_events::insert_batch(&pool, &batch).await {
                                warn!(err=?e, "book_events batch insert failed");
                            } else {
                                if tick % 10 == 0 {
                                    info!(tick, batch=len, "book events flushed");
                                }
                            }
                            batch.clear();
                        }
                    }
                    _ = flush_interval.tick() => {
                        if !batch.is_empty() {
                            let len = batch.len();
                            if let Err(e) = db::book_events::insert_batch(&pool, &batch).await {
                                warn!(err=?e, "book_events flush failed");
                            }
                            batch.clear();
                            tick += 1;
                            if tick % 30 == 0 {
                                let _ = db::sync::record_heartbeat(&pool, "lifecycle.book_sink",
                                    serde_json::json!({ "flush_tick": tick, "last_batch": len })).await;
                            }
                        }
                    }
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    // ---- shutdown ----
    tokio::signal::ctrl_c().await.ok();
    info!("shutdown signal");
    let _ = shutdown_tx.send(true);
    let _ = tokio::join!(universe_task, fills_task, positions_task, pnl_snap_task,
        markets_meta_task, actions_task, retention_task, maker_classify_task, ws_task, sink_task);
    info!("stopped");
    Ok(())
}

/// fills.is_maker 추론 — Tier1(last_trade aggressor) + Tier2(fill 직전 book best_bid).
/// 최근 6h window 만 대상 (steady-state 유지용 — 더 오래된 backlog 는
/// scripts/backfill-is-maker.sql 로 처리). 반환: (tier1 건수, tier2 건수).
async fn classify_fills(pool: &sqlx::PgPool) -> Result<(u64, u64)> {
    // Tier 1: WS last_trade_price 의 aggressor side 매칭 (고신뢰).
    // RN1 은 BUY-only → taker=SELL 이면 RN1=maker, taker=BUY 이면 RN1=taker.
    let t1 = sqlx::query(r#"
        WITH lt AS (
          SELECT DISTINCT
            payload->>'transaction_hash' AS txh,
            payload->>'asset_id'         AS aid,
            payload->>'side'             AS taker_side
          FROM book_events
          WHERE event_type = 'last_trade_price'
            AND ts > NOW() - INTERVAL '7 hours'
            AND payload->>'transaction_hash' IS NOT NULL
        ),
        m AS (
          SELECT f.fill_id,
                 bool_and(lt.taker_side = 'SELL') AS all_maker,
                 bool_or (lt.taker_side = 'SELL') AS any_maker
          FROM fills f
          JOIN asset_to_condition a
            ON a.condition_id = f.condition_id AND a.outcome_index = f.outcome_index
          JOIN lt ON lt.txh = f.tx_hash AND lt.aid = a.asset_id
          WHERE f.ts > NOW() - INTERVAL '6 hours'
            AND f.maker_method IS DISTINCT FROM 'last_trade'
          GROUP BY f.fill_id
        )
        UPDATE fills f
        SET is_maker = m.all_maker, maker_method = 'last_trade'
        FROM m
        WHERE f.fill_id = m.fill_id AND m.all_maker = m.any_maker
    "#).execute(pool).await?.rows_affected();

    // Tier 2: fill 직전 15분 내 book best_bid 대비 추론 (중신뢰).
    // last_trade 매칭 기회를 먼저 주기 위해 15분 지난 fill 만 대상.
    let t2 = sqlx::query(r#"
        WITH cand AS (
          SELECT f.fill_id, f.price, f.ts, a.asset_id
          FROM fills f
          JOIN asset_to_condition a
            ON a.condition_id = f.condition_id AND a.outcome_index = f.outcome_index
          WHERE f.maker_method IS NULL
            AND f.ts > NOW() - INTERVAL '6 hours'
            AND f.ts < NOW() - INTERVAL '15 minutes'
        ),
        classified AS (
          SELECT c.fill_id, (c.price <= b.best_bid + 0.005) AS is_maker
          FROM cand c
          CROSS JOIN LATERAL (
            SELECT be.best_bid
            FROM book_events be
            WHERE be.asset_id   = c.asset_id
              AND be.event_type = 'book'
              AND be.best_bid   IS NOT NULL
              AND be.best_ask   IS NOT NULL
              AND be.ts <= c.ts
              AND be.ts >  c.ts - INTERVAL '15 minutes'
            ORDER BY be.ts DESC
            LIMIT 1
          ) b
        )
        UPDATE fills f
        SET is_maker = cl.is_maker, maker_method = 'book'
        FROM classified cl
        WHERE f.fill_id = cl.fill_id
    "#).execute(pool).await?.rows_affected();

    Ok((t1, t2))
}

/// MERGE / REDEEM / SPLIT 액션 idempotent INSERT.
/// 반환: true = 새로 삽입, false = 중복으로 무시 (정상).
async fn insert_action(pool: &sqlx::PgPool, kind: &str, r: &ActivityRow) -> Result<bool> {
    let triggered = chrono::DateTime::<Utc>::from_timestamp(r.timestamp, 0).unwrap_or_else(Utc::now);
    let realized_usdc = if r.usdc_size > 0.0 { Some(r.usdc_size) }
                       else if r.price > 0.0 { Some(r.size * r.price) }
                       else { None };

    // tx_hash 유무에 따라 UPSERT target 다르게 — partial unique index 두 개를 활용.
    if r.transaction_hash.is_some() {
        let res = sqlx::query(r#"
            INSERT INTO position_actions
                (condition_id, action_type, size, tx_hash, status, realized_usdc, triggered_at, confirmed_at)
            VALUES ($1, $2, $3, $4, 'confirmed', $5, $6, $6)
            ON CONFLICT (tx_hash, condition_id, action_type) WHERE tx_hash IS NOT NULL
            DO NOTHING
        "#)
            .bind(&r.condition_id).bind(kind).bind(r.size)
            .bind(&r.transaction_hash)
            .bind(realized_usdc)
            .bind(triggered)
            .execute(pool).await?;
        Ok(res.rows_affected() > 0)
    } else {
        let res = sqlx::query(r#"
            INSERT INTO position_actions
                (condition_id, action_type, size, tx_hash, status, realized_usdc, triggered_at, confirmed_at)
            VALUES ($1, $2, $3, NULL, 'confirmed', $4, $5, $5)
            ON CONFLICT (condition_id, action_type, triggered_at, size) WHERE tx_hash IS NULL
            DO NOTHING
        "#)
            .bind(&r.condition_id).bind(kind).bind(r.size)
            .bind(realized_usdc)
            .bind(triggered)
            .execute(pool).await?;
        Ok(res.rows_affected() > 0)
    }
}

async fn insert_observed_fill(pool: &sqlx::PgPool, r: &ActivityRow) -> Result<bool> {
    let fill = rn1_core::Fill {
        fill_id: None,
        tx_hash: r.transaction_hash.clone(),
        condition_id: r.condition_id.clone(),
        // TRADE 활동은 0/1만 들어옴 (binary outcome). 다른 값은 truncate.
        outcome_index: if r.outcome_index <= 255 { r.outcome_index as u8 } else { 0 },
        outcome_name: r.outcome.clone(),
        side: r.side.clone(),
        size: r.size,
        price: r.price,
        usdc_size: if r.usdc_size > 0.0 { r.usdc_size } else { r.size * r.price },
        fee_paid: 0.0,
        rebate_received: 0.0,
        timestamp: chrono::DateTime::<Utc>::from_timestamp(r.timestamp, 0).unwrap_or_else(Utc::now),
        is_ours: false,
        is_maker: None,
        title: r.title.clone(),
        slug: r.slug.clone(),
        event_slug: r.event_slug.clone(),
        icon: r.icon.clone(),
    };
    db::fills::insert(pool, &fill).await
}

impl Clone for Args {
    fn clone(&self) -> Self {
        Self {
            wallet: self.wallet.clone(),
            lookback_h: self.lookback_h,
            universe_refresh_s: self.universe_refresh_s,
            fills_poll_s: self.fills_poll_s,
            max_assets: self.max_assets,
            database_url: self.database_url.clone(),
        }
    }
}

// silence unused
#[allow(dead_code)]
fn _used(_a: &chrono::DateTime<Utc>, _b: ChronoDur) {}
#[allow(dead_code)]
fn _used2(_a: Arc<u32>) {}
