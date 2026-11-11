//! Polymarket CLOB v2 public `market` channel WS 클라이언트.
//!
//! - 동적 subscribe (asset_ids 목록 변경 시 자동 재연결)
//! - 자동 backoff reconnect
//! - 이벤트 -> tokio::sync::mpsc 채널로 흘러나가게 함

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use rn1_core::constants::CLOB_WS_MARKET;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, info, warn};

pub mod user;
pub use user::{spawn_user_channel, UserAuth, UserEvent};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event_type")]
#[serde(rename_all = "snake_case")]
pub enum MarketEvent {
    Book {
        market: String,
        asset_id: String,
        timestamp: String,
        bids: Vec<PriceLevel>,
        asks: Vec<PriceLevel>,
        #[serde(default)]
        tick_size: Option<String>,
        #[serde(default)]
        hash: Option<String>,
    },
    PriceChange {
        market: String,
        #[serde(default)]
        asset_id: Option<String>,
        #[serde(default)]
        timestamp: Option<String>,
        #[serde(default)]
        changes: Vec<PriceChange>,
    },
    LastTradePrice {
        market: String,
        asset_id: String,
        timestamp: String,
        price: String,
        size: String,
        #[serde(default)]
        side: Option<String>,
        #[serde(default)]
        fee_rate_bps: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: String,
    pub size: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceChange {
    pub price: String,
    pub size: String,
    pub side: String, // BUY / SELL
    #[serde(default)]
    pub asset_id: Option<String>,
}

/// Raw event delivered to consumer (with extracted best_bid/ask if computable).
#[derive(Clone, Debug)]
pub struct RawMarketEvent {
    pub received_at: DateTime<Utc>,
    pub asset_id: String,
    pub condition_id: Option<String>,
    pub event_type: String,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub payload: Value,
}

#[derive(Clone)]
pub struct MarketWsClient {
    assets: Arc<Mutex<Vec<String>>>,
    reconnect_tx: watch::Sender<u64>,
}

impl MarketWsClient {
    /// 빈 universe 로 시작. `update_assets` 호출하면 즉시 재구독.
    pub fn new() -> (Self, watch::Receiver<u64>) {
        let (tx, rx) = watch::channel(0);
        (Self { assets: Arc::new(Mutex::new(vec![])), reconnect_tx: tx }, rx)
    }

    pub async fn update_assets(&self, new_assets: Vec<String>) {
        let mut a = self.assets.lock().await;
        if *a != new_assets {
            *a = new_assets;
            let v = *self.reconnect_tx.borrow();
            let _ = self.reconnect_tx.send(v + 1);
        }
    }

    pub async fn current_assets(&self) -> Vec<String> {
        self.assets.lock().await.clone()
    }
}

/// 단일 backoff-aware WS subscriber.
/// 이벤트는 `out` 채널로 흐른다. assets 변경 또는 연결 끊김 시 자동 reconnect.
pub async fn run_market_subscriber(
    client: MarketWsClient,
    mut reconnect_rx: watch::Receiver<u64>,
    out: mpsc::Sender<RawMarketEvent>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut backoff_ms = 500u64;
    loop {
        if *shutdown.borrow() { break; }
        let assets = client.current_assets().await;
        if assets.is_empty() {
            tokio::select! {
                _ = reconnect_rx.changed() => continue,
                _ = shutdown.changed() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => continue,
            }
        }

        info!(asset_count = assets.len(), "ws: connecting");
        let conn = connect_async(CLOB_WS_MARKET).await;
        let (mut ws_stream, _) = match conn {
            Ok(p) => p,
            Err(e) => {
                warn!(err=?e, "ws connect failed; backing off {}ms", backoff_ms);
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(30_000);
                continue;
            }
        };

        let sub = serde_json::json!({ "type": "market", "assets_ids": assets });
        if let Err(e) = ws_stream.send(Message::Text(sub.to_string().into())).await {
            warn!(err=?e, "ws subscribe send failed");
            continue;
        }
        info!(asset_count = assets.len(), "ws: subscribed");
        backoff_ms = 500;

        // Polymarket CLOB WS keepalive: send a "PING" text frame every 10s.
        // Without it the idle connection drops every ~3min.
        let mut ping_tick = tokio::time::interval(std::time::Duration::from_secs(10));
        ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ping_tick.tick().await;

        loop {
            tokio::select! {
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(Message::Text(t))) => {
                            if let Err(e) = handle_text(&t, &out).await {
                                debug!(err=?e, "msg parse");
                            }
                        }
                        Some(Ok(Message::Ping(p))) => { let _ = ws_stream.send(Message::Pong(p)).await; }
                        Some(Ok(Message::Close(_))) | None => { warn!("ws closed"); break; }
                        Some(Err(e)) => { warn!(err=?e, "ws recv error"); break; }
                        _ => {}
                    }
                }
                _ = ping_tick.tick() => {
                    if let Err(e) = ws_stream.send(Message::Text("PING".into())).await {
                        warn!(err=?e, "ws ping send failed"); break;
                    }
                }
                _ = reconnect_rx.changed() => { info!("ws: reconnect signal"); break; }
                _ = shutdown.changed() => { let _ = ws_stream.close(None).await; return Ok(()); }
            }
        }
    }
    Ok(())
}

async fn handle_text(text: &str, out: &mpsc::Sender<RawMarketEvent>) -> Result<()> {
    let parsed: Value = serde_json::from_str(text)?;
    let now = Utc::now();
    match parsed {
        Value::Array(arr) => {
            for ev in arr { dispatch_event(ev, now, out).await?; }
        }
        v @ Value::Object(_) => dispatch_event(v, now, out).await?,
        _ => {}
    }
    Ok(())
}

async fn dispatch_event(ev: Value, now: DateTime<Utc>, out: &mpsc::Sender<RawMarketEvent>) -> Result<()> {
    // event_type 추론 (price_change 응답은 `price_changes` 키만 옴)
    let mut event_type = ev.get("event_type").and_then(|x| x.as_str()).map(String::from);
    if event_type.is_none() && ev.get("price_changes").is_some() {
        event_type = Some("price_change".into());
    }
    let event_type = match event_type {
        Some(t) => t,
        None => return Ok(()),
    };

    let asset_id = ev.get("asset_id").and_then(|x| x.as_str())
        .or_else(|| ev.get("price_changes")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.get("asset_id"))
            .and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    if asset_id.is_empty() { return Ok(()); }

    let condition_id = ev.get("market").and_then(|x| x.as_str()).map(String::from);

    // best_bid/ask extract (book event)
    let (mut best_bid, mut best_ask) = (None, None);
    if event_type == "book" {
        if let Some(bids) = ev.get("bids").and_then(|x| x.as_array()) {
            best_bid = bids.iter()
                .filter_map(|b| b.get("price").and_then(|p| p.as_str())
                    .and_then(|s| s.parse::<f64>().ok()))
                .fold(None::<f64>, |acc, x| match acc { Some(a) => Some(a.max(x)), None => Some(x) });
        }
        if let Some(asks) = ev.get("asks").and_then(|x| x.as_array()) {
            best_ask = asks.iter()
                .filter_map(|a| a.get("price").and_then(|p| p.as_str())
                    .and_then(|s| s.parse::<f64>().ok()))
                .fold(None::<f64>, |acc, x| match acc { Some(a) => Some(a.min(x)), None => Some(x) });
        }
    }

    let raw = RawMarketEvent {
        received_at: now,
        asset_id,
        condition_id,
        event_type,
        best_bid,
        best_ask,
        payload: ev,
    };
    let _ = out.send(raw).await;
    Ok(())
}
