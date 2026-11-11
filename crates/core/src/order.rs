//! 주문 (우리 자체) — M4 부터 사용. RN1 의 주문은 관찰 불가.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderSide { Buy, Sell }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Signed,
    Posted,
    Resting,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    pub order_id: i64,
    pub clob_order_hash: Option<String>,
    pub condition_id: String,
    pub outcome_index: u8,
    pub asset_id: String,
    pub side: OrderSide,
    pub price: f64,
    pub size: f64,
    pub signed_at: DateTime<Utc>,
    pub posted_at: Option<DateTime<Utc>>,
    pub filled_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub status: OrderStatus,
    pub rejection_reason: Option<String>,
}
