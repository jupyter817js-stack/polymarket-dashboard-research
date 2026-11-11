use rn1_core::Position;
use sqlx::PgPool;

pub async fn upsert(pool: &PgPool, p: &Position) -> anyhow::Result<()> {
    sqlx::query(r#"
        INSERT INTO positions (
            condition_id, outcome_index, outcome_name, size, avg_price,
            current_price, current_value, initial_value, realized_pnl, cash_pnl,
            total_bought, mergeable, redeemable, title, slug, event_slug, icon, neg_risk, end_date, updated_at
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19, NOW())
        ON CONFLICT (condition_id, outcome_index) DO UPDATE SET
            size = EXCLUDED.size,
            avg_price = EXCLUDED.avg_price,
            current_price = EXCLUDED.current_price,
            current_value = EXCLUDED.current_value,
            initial_value = EXCLUDED.initial_value,
            realized_pnl = EXCLUDED.realized_pnl,
            cash_pnl = EXCLUDED.cash_pnl,
            total_bought = EXCLUDED.total_bought,
            mergeable = EXCLUDED.mergeable,
            redeemable = EXCLUDED.redeemable,
            title = COALESCE(EXCLUDED.title, positions.title),
            slug = COALESCE(EXCLUDED.slug, positions.slug),
            event_slug = COALESCE(EXCLUDED.event_slug, positions.event_slug),
            icon = COALESCE(EXCLUDED.icon, positions.icon),
            neg_risk = EXCLUDED.neg_risk,
            end_date = COALESCE(EXCLUDED.end_date, positions.end_date),
            updated_at = NOW()
    "#)
    .bind(&p.condition_id)
    .bind(p.outcome_index as i16)
    .bind(&p.outcome_name)
    .bind(p.size)
    .bind(p.avg_price)
    .bind(p.current_price)
    .bind(p.current_value)
    .bind(p.initial_value)
    .bind(p.realized_pnl)
    .bind(p.cash_pnl)
    .bind(p.total_bought)
    .bind(p.mergeable)
    .bind(p.redeemable)
    .bind(p.title.as_deref())
    .bind(p.slug.as_deref())
    .bind(p.event_slug.as_deref())
    .bind(p.icon.as_deref())
    .bind(p.neg_risk)
    .bind(p.end_date)
    .execute(pool).await?;
    Ok(())
}

/// 모든 보유 포지션 (size > 0)
pub async fn all_active(pool: &PgPool) -> anyhow::Result<Vec<Position>> {
    use sqlx::Row;
    let rows = sqlx::query(r#"
        SELECT condition_id, outcome_index, outcome_name, size, avg_price,
            current_price, current_value, initial_value, realized_pnl, cash_pnl,
            total_bought, mergeable, redeemable, title, slug, event_slug, icon,
            neg_risk, end_date, updated_at
        FROM positions
        WHERE size > 0 OR redeemable = true OR mergeable = true
        ORDER BY current_value DESC
    "#).fetch_all(pool).await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(Position {
            condition_id: r.get("condition_id"),
            outcome_index: r.get::<i16, _>("outcome_index") as u8,
            outcome_name: r.get("outcome_name"),
            size: r.get("size"),
            avg_price: r.get("avg_price"),
            current_price: r.get("current_price"),
            current_value: r.get("current_value"),
            initial_value: r.get("initial_value"),
            realized_pnl: r.get("realized_pnl"),
            cash_pnl: r.get("cash_pnl"),
            total_bought: r.get("total_bought"),
            mergeable: r.get("mergeable"),
            redeemable: r.get("redeemable"),
            title: r.get("title"),
            slug: r.get("slug"),
            event_slug: r.get("event_slug"),
            icon: r.get("icon"),
            neg_risk: r.get("neg_risk"),
            end_date: r.get("end_date"),
            updated_at: r.get("updated_at"),
        });
    }
    Ok(out)
}
