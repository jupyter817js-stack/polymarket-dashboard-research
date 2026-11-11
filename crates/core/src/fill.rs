//! 체결. 자체 체결 + RN1 (외부) 관찰 체결 둘 다.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    pub fill_id: Option<i64>,
    pub tx_hash: Option<String>,
    pub condition_id: String,
    pub outcome_index: u8,
    pub outcome_name: Option<String>,
    pub side: String, // BUY / SELL
    pub size: f64,
    pub price: f64,
    pub usdc_size: f64,
    pub fee_paid: f64,
    pub rebate_received: f64,
    pub timestamp: DateTime<Utc>,
    /// Our own trade vs observed (RN1 etc.)
    pub is_ours: bool,
    /// Whether RN1/we were maker
    pub is_maker: Option<bool>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub event_slug: Option<String>,
    pub icon: Option<String>,
}
