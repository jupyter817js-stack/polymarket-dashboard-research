//! 시장 + outcome + 수수료 스케줄.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Market {
    pub condition_id: String,
    pub slug: String,
    pub event_slug: Option<String>,
    pub question: Option<String>,
    pub outcomes: Vec<String>,
    pub clob_token_ids: Vec<String>,
    pub outcome_prices: Option<Vec<f64>>, // resolved 시 [1,0] 등
    pub neg_risk: bool,
    pub closed: bool,
    pub active: bool,
    pub end_date: Option<DateTime<Utc>>,
    pub game_start_time: Option<DateTime<Utc>>,
    pub accepting_orders_ts: Option<DateTime<Utc>>,
    pub uma_resolution_status: Option<String>,
    pub last_trade_price: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub volume_num: Option<f64>,
    pub fee_schedule: Option<FeeSchedule>,
    pub sports_market_type: Option<String>,
    pub order_min_size: Option<f64>,
    pub order_price_min_tick_size: Option<f64>,
    pub rewards_max_spread: Option<f64>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeeSchedule {
    #[serde(default)]
    pub rate: f64,
    #[serde(default, rename = "rebateRate")]
    pub rebate_rate: f64,
    #[serde(default, rename = "takerOnly")]
    pub taker_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub index: u8,
    pub name: String,
    pub token_id: Option<String>,
    pub resolved_price: Option<f64>,
}
