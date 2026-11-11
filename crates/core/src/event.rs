//! 이벤트 단위 그룹 (frontend tree view 의 상위 노드).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventGroup {
    pub event_slug: String,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub category: Option<String>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub submarket_count: u32,
    pub is_multi_neg: bool,
    pub trade_pnl: f64,
    pub gross_notional: f64,
    pub merge_locked: f64,
    pub conditions: Vec<String>, // condition_ids
}
