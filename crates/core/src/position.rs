//! 보유 포지션 (한 cid, 한 outcome).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub condition_id: String,
    pub outcome_index: u8,
    pub outcome_name: String,
    pub size: f64,
    pub avg_price: Option<f64>,
    pub current_price: Option<f64>,
    pub current_value: f64,
    pub initial_value: f64,
    pub realized_pnl: f64,
    pub cash_pnl: f64,
    pub total_bought: f64,
    pub mergeable: bool,
    pub redeemable: bool,
    pub updated_at: DateTime<Utc>,
    // metadata
    pub title: Option<String>,
    pub slug: Option<String>,
    pub event_slug: Option<String>,
    pub icon: Option<String>,
    pub neg_risk: bool,
    pub end_date: Option<DateTime<Utc>>,
}
