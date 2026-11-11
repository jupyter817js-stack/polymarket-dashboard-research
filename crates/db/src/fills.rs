use rn1_core::Fill;
use sqlx::PgPool;

/// 새 fill 삽입. tx_hash + asset(token_id) + side + size 의 조합으로 dedupe.
/// 실제로 새 row 가 들어가면 `true`, conflict(이미 있음) 면 `false`.
pub async fn insert(pool: &PgPool, f: &Fill) -> anyhow::Result<bool> {
    let res = sqlx::query(r#"
        INSERT INTO fills (
            tx_hash, condition_id, outcome_index, outcome_name, side,
            size, price, usdc_size, fee_paid, rebate_received,
            ts, is_ours, is_maker, title, slug, event_slug, icon
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
        ON CONFLICT (tx_hash, condition_id, outcome_index, side, size, price, ts)
        DO NOTHING
    "#)
    .bind(f.tx_hash.as_deref())
    .bind(&f.condition_id)
    .bind(f.outcome_index as i16)
    .bind(f.outcome_name.as_deref())
    .bind(&f.side)
    .bind(f.size)
    .bind(f.price)
    .bind(f.usdc_size)
    .bind(f.fee_paid)
    .bind(f.rebate_received)
    .bind(f.timestamp)
    .bind(f.is_ours)
    .bind(f.is_maker)
    .bind(f.title.as_deref())
    .bind(f.slug.as_deref())
    .bind(f.event_slug.as_deref())
    .bind(f.icon.as_deref())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 최신 RN1 fill timestamp (cursor).
pub async fn last_rn1_ts(pool: &PgPool) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>> {
    let row: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(r#"
        SELECT ts FROM fills WHERE is_ours = false ORDER BY ts DESC LIMIT 1
    "#)
    .fetch_optional(pool).await?;
    Ok(row.map(|(t,)| t))
}
