//! User channel — authenticated WS that streams our own trades/fills.
//!
//! Subscribes with the L2 api_key/secret/passphrase and emits `UserEvent::Fill`
//! to the driver. Sends a "PING" text frame every 10s (Polymarket CLOB WS
//! keepalive requirement — without it the idle connection drops every ~3min).

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone)]
pub enum UserEvent {
    Fill {
        order_id: String,
        size_shares: f64,
        price: f64,
        usdc_size: f64,
        dedup_key: String,
    },
    Reconnected,
    Disconnected { reason: String },
}

#[derive(Clone)]
pub struct UserAuth {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
}

/// Spawn the user channel subscriber. Auto-reconnects with exponential backoff
/// (clamped to 30s).
pub fn spawn_user_channel(
    url: String,
    auth: UserAuth,
    market_ids: Vec<String>,
    tx: mpsc::UnboundedSender<UserEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff_s = 1u64;
        loop {
            tracing::info!(channel="user", n=market_ids.len(), "connecting WS");
            match run_user_session(&url, &auth, &market_ids, &tx).await {
                Ok(()) => {
                    let _ = tx.send(UserEvent::Disconnected { reason: "clean close".into() });
                    backoff_s = 1;
                }
                Err(e) => {
                    tracing::warn!(channel="user", err=?e, "session ended");
                    let _ = tx.send(UserEvent::Disconnected { reason: e.to_string() });
                }
            }
            tokio::time::sleep(Duration::from_secs(backoff_s)).await;
            backoff_s = (backoff_s * 2).min(30);
        }
    })
}

async fn run_user_session(
    url: &str,
    auth: &UserAuth,
    market_ids: &[String],
    tx: &mpsc::UnboundedSender<UserEvent>,
) -> Result<()> {
    let (mut socket, _resp) = connect_async(url).await.context("connect_async user")?;
    let auth_body = json!({
        "apiKey": auth.api_key,
        "secret": auth.api_secret,
        "passphrase": auth.passphrase,
    });
    let sub = json!({
        "type": "USER",
        "markets": market_ids,
        "auth": auth_body,
        "timestamp": chrono::Utc::now().timestamp().to_string(),
    });
    socket.send(Message::Text(sub.to_string().into())).await.context("send sub user")?;
    let _ = tx.send(UserEvent::Reconnected);

    let mut ping_tick = tokio::time::interval(Duration::from_secs(10));
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ping_tick.tick().await;

    loop {
        tokio::select! {
            msg = socket.next() => {
                let msg = match msg {
                    Some(m) => m.context("read user ws")?,
                    None => return Ok(()),
                };
                match msg {
                    Message::Text(t) => handle_user_text(&t, tx),
                    Message::Ping(p) => { let _ = socket.send(Message::Pong(p)).await; }
                    Message::Close(_) => return Ok(()),
                    _ => {}
                }
            }
            _ = ping_tick.tick() => {
                if let Err(e) = socket.send(Message::Text("PING".into())).await {
                    return Err(anyhow::anyhow!("user ws ping send failed: {e}"));
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct UserTradeMsg {
    event_type: Option<String>,
    #[serde(default)] id: Option<String>,
    #[serde(default)] match_time: Option<serde_json::Value>,
    #[serde(default)] transaction_hash: Option<String>,
    #[serde(default)] maker_order_id: Option<String>,
    #[serde(default)] taker_order_id: Option<String>,
    #[serde(default)] size: Option<String>,
    #[serde(default)] price: Option<String>,
}

fn handle_user_text(text: &str, tx: &mpsc::UnboundedSender<UserEvent>) {
    let parsed: serde_json::Value = match serde_json::from_str(text) { Ok(v) => v, Err(_) => return };
    let items: Vec<UserTradeMsg> = match parsed {
        serde_json::Value::Array(arr) => arr.into_iter()
            .filter_map(|v| serde_json::from_value(v).ok()).collect(),
        v => match serde_json::from_value::<UserTradeMsg>(v) {
            Ok(m) => vec![m],
            Err(_) => return,
        }
    };
    for m in items {
        if m.event_type.as_deref() != Some("trade") { continue; }
        let order_id = match m.maker_order_id.clone().or(m.taker_order_id.clone()) {
            Some(o) => o, None => continue,
        };
        let size_shares: f64 = m.size.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let price: f64 = m.price.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        if size_shares == 0.0 || price == 0.0 { continue; }
        let usdc = size_shares * price;
        let dedup_key = match (m.id.as_deref(), m.transaction_hash.as_deref()) {
            (Some(id), _) if !id.is_empty() => format!("id:{id}"),
            (_, Some(tx)) if !tx.is_empty() => format!("tx:{tx}"),
            _ => {
                let ts = m.match_time.as_ref().map(|v| v.to_string()).unwrap_or_default();
                format!("syn:{order_id}:{ts}:{size_shares}:{price}")
            }
        };
        let _ = tx.send(UserEvent::Fill { order_id, size_shares, price, usdc_size: usdc, dedup_key });
    }
}
