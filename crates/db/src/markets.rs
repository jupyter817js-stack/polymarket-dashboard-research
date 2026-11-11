use rn1_core::Market;
use sqlx::PgPool;

/// markets 테이블 upsert. JSON 필드 (outcomes / clob_token_ids / outcome_prices /
/// fee_schedule) 는 serde_json::Value 로 직렬화 후 jsonb 칼럼에 저장.
pub async fn upsert(pool: &PgPool, m: &Market) -> anyhow::Result<()> {
    let outcomes = serde_json::to_value(&m.outcomes)?;
    let clob_token_ids = serde_json::to_value(&m.clob_token_ids)?;
    let outcome_prices = m.outcome_prices.as_ref()
        .map(|p| serde_json::to_value(p)).transpose()?;
    let fee_schedule = m.fee_schedule.as_ref()
        .map(|f| serde_json::to_value(f)).transpose()?;

    sqlx::query(r#"
        INSERT INTO markets (
            condition_id, slug, event_slug, question, outcomes, outcome_prices,
            clob_token_ids, fee_schedule, neg_risk, closed, active,
            end_date, game_start, accepting_orders, uma_status,
            last_trade_price, best_bid, best_ask, volume_num,
            sports_market_type, order_min_size, order_price_min_tick_size,
            rewards_max_spread, icon, refreshed_at
        )
        VALUES (
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24, NOW()
        )
        ON CONFLICT (condition_id) DO UPDATE SET
            slug = EXCLUDED.slug,
            event_slug = COALESCE(EXCLUDED.event_slug, markets.event_slug),
            question = COALESCE(EXCLUDED.question, markets.question),
            outcomes = EXCLUDED.outcomes,
            outcome_prices = COALESCE(EXCLUDED.outcome_prices, markets.outcome_prices),
            clob_token_ids = EXCLUDED.clob_token_ids,
            fee_schedule = COALESCE(EXCLUDED.fee_schedule, markets.fee_schedule),
            neg_risk = EXCLUDED.neg_risk,
            closed = EXCLUDED.closed,
            active = EXCLUDED.active,
            end_date = COALESCE(EXCLUDED.end_date, markets.end_date),
            game_start = COALESCE(EXCLUDED.game_start, markets.game_start),
            accepting_orders = COALESCE(EXCLUDED.accepting_orders, markets.accepting_orders),
            uma_status = COALESCE(EXCLUDED.uma_status, markets.uma_status),
            last_trade_price = COALESCE(EXCLUDED.last_trade_price, markets.last_trade_price),
            best_bid = COALESCE(EXCLUDED.best_bid, markets.best_bid),
            best_ask = COALESCE(EXCLUDED.best_ask, markets.best_ask),
            volume_num = COALESCE(EXCLUDED.volume_num, markets.volume_num),
            sports_market_type = COALESCE(EXCLUDED.sports_market_type, markets.sports_market_type),
            order_min_size = COALESCE(EXCLUDED.order_min_size, markets.order_min_size),
            order_price_min_tick_size = COALESCE(EXCLUDED.order_price_min_tick_size, markets.order_price_min_tick_size),
            rewards_max_spread = COALESCE(EXCLUDED.rewards_max_spread, markets.rewards_max_spread),
            icon = COALESCE(EXCLUDED.icon, markets.icon),
            refreshed_at = NOW()
    "#)
    .bind(&m.condition_id)
    .bind(&m.slug)
    .bind(m.event_slug.as_deref())
    .bind(m.question.as_deref())
    .bind(outcomes)
    .bind(outcome_prices)
    .bind(clob_token_ids)
    .bind(fee_schedule)
    .bind(m.neg_risk)
    .bind(m.closed)
    .bind(m.active)
    .bind(m.end_date)
    .bind(m.game_start_time)
    .bind(m.accepting_orders_ts)
    .bind(m.uma_resolution_status.as_deref())
    .bind(m.last_trade_price)
    .bind(m.best_bid)
    .bind(m.best_ask)
    .bind(m.volume_num)
    .bind(m.sports_market_type.as_deref())
    .bind(m.order_min_size)
    .bind(m.order_price_min_tick_size)
    .bind(m.rewards_max_spread)
    .bind(m.icon.as_deref())
    .execute(pool)
    .await?;

    // asset_to_condition 매핑 동시 유지 — book_events × fills JOIN 의 핵심 키.
    // clob_token_ids[i] = outcome i 의 asset_id (CLOB token).
    for (i, asset_id) in m.clob_token_ids.iter().enumerate() {
        let outcome_name = m.outcomes.get(i).cloned();
        sqlx::query(r#"
            INSERT INTO asset_to_condition (asset_id, condition_id, outcome_index, outcome_name)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (asset_id) DO UPDATE SET
                condition_id  = EXCLUDED.condition_id,
                outcome_index = EXCLUDED.outcome_index,
                outcome_name  = COALESCE(EXCLUDED.outcome_name, asset_to_condition.outcome_name)
        "#)
        .bind(asset_id)
        .bind(&m.condition_id)
        .bind(i as i16)
        .bind(outcome_name)
        .execute(pool).await?;
    }
    Ok(())
}
