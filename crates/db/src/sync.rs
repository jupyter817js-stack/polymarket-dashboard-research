//! Sync cursor / job 상태 추적.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// 마지막 RN1 fill ingestion cursor.
pub async fn set_cursor(pool: &PgPool, key: &str, value: i64) -> anyhow::Result<()> {
    sqlx::query(r#"
        INSERT INTO sync_cursors(key, value_int, updated_at)
        VALUES ($1, $2, NOW())
        ON CONFLICT (key) DO UPDATE SET value_int = EXCLUDED.value_int, updated_at = NOW()
    "#).bind(key).bind(value).execute(pool).await?;
    Ok(())
}

pub async fn get_cursor(pool: &PgPool, key: &str) -> anyhow::Result<Option<i64>> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(r#"
        SELECT value_int FROM sync_cursors WHERE key = $1
    "#).bind(key).fetch_optional(pool).await?;
    Ok(row.and_then(|(v,)| v))
}

pub async fn record_heartbeat(pool: &PgPool, component: &str, payload: serde_json::Value) -> anyhow::Result<()> {
    sqlx::query(r#"
        INSERT INTO system_heartbeats(component, ts, payload)
        VALUES ($1, NOW(), $2)
        ON CONFLICT (component) DO UPDATE SET ts = NOW(), payload = EXCLUDED.payload
    "#).bind(component).bind(payload).execute(pool).await?;
    Ok(())
}

pub async fn latest_heartbeats(pool: &PgPool) -> anyhow::Result<Vec<(String, DateTime<Utc>, serde_json::Value)>> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT component, ts, payload FROM system_heartbeats ORDER BY component")
        .fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}
