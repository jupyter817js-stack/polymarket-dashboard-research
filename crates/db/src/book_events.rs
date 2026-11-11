//! Public market WS 의 book / price_change / last_trade_price 이벤트 저장 (M0.5).
//! 원본 JSON 그대로 jsonb 칼럼에 저장 + index 용 컬럼만 추출.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct BookEvent {
    pub ts: DateTime<Utc>,
    pub asset_id: String,
    pub condition_id: Option<String>,
    pub event_type: String, // "book" | "price_change" | "last_trade_price"
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub spread: Option<f64>,
    pub payload: Value,
}

pub async fn insert(pool: &PgPool, e: &BookEvent) -> anyhow::Result<()> {
    sqlx::query(r#"
        INSERT INTO book_events (ts, asset_id, condition_id, event_type,
            best_bid, best_ask, spread, payload)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
    "#)
    .bind(e.ts)
    .bind(&e.asset_id)
    .bind(e.condition_id.as_deref())
    .bind(&e.event_type)
    .bind(e.best_bid)
    .bind(e.best_ask)
    .bind(e.spread)
    .bind(&e.payload)
    .execute(pool).await?;
    Ok(())
}

/// 배치 insert (성능 — 100~500개씩 묶어서 호출).
pub async fn insert_batch(pool: &PgPool, events: &[BookEvent]) -> anyhow::Result<()> {
    if events.is_empty() { return Ok(()); }
    let mut tx = pool.begin().await?;
    for e in events {
        sqlx::query(r#"
            INSERT INTO book_events (ts, asset_id, condition_id, event_type,
                best_bid, best_ask, spread, payload)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
        "#)
        .bind(e.ts)
        .bind(&e.asset_id)
        .bind(e.condition_id.as_deref())
        .bind(&e.event_type)
        .bind(e.best_bid)
        .bind(e.best_ask)
        .bind(e.spread)
        .bind(&e.payload)
        .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
