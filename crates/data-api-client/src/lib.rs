//! Polymarket public REST clients:
//! - Gamma API (`/markets`)
//! - Data API (`/activity`, `/positions`)
//!
//! 모두 인증 불필요. UA 헤더 + gzip + 재시도 backoff.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rn1_core::*;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub mod gamma;
pub mod activity;
pub mod positions;

const UA: &str = "rn1-bot/0.1 (+research)";

#[derive(Clone)]
pub struct DataApi {
    pub client: Client,
}

impl DataApi {
    pub fn new() -> Result<Self> {
        let client = ClientBuilder::new()
            .user_agent(UA)
            .gzip(true)
            .timeout(Duration::from_secs(20))
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .context("reqwest build")?;
        Ok(Self { client })
    }
}

/// /activity row (TRADE / REDEEM / MERGE / SPLIT 등 공통)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityRow {
    #[serde(rename = "proxyWallet")]
    pub proxy_wallet: String,
    #[serde(default)]
    pub side: String,
    pub asset: String,
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    pub size: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(rename = "usdcSize", default)]
    pub usdc_size: f64,
    pub timestamp: i64,
    #[serde(rename = "transactionHash", default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, rename = "eventSlug")]
    pub event_slug: Option<String>,
    // MERGE/REDEEM 응답에서 outcomeIndex = 999 sentinel 이 들어옴 — u16 으로 받음.
    // TRADE 는 0 또는 1.
    #[serde(rename = "outcomeIndex", default)]
    pub outcome_index: u16,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: String, // TRADE / REDEEM / MERGE / SPLIT
}

/// /positions row
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PositionRow {
    #[serde(rename = "proxyWallet")]
    pub proxy_wallet: String,
    pub asset: String,
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    pub size: f64,
    #[serde(rename = "avgPrice", default)]
    pub avg_price: Option<f64>,
    #[serde(rename = "currentValue", default)]
    pub current_value: f64,
    #[serde(rename = "initialValue", default)]
    pub initial_value: f64,
    #[serde(rename = "cashPnl", default)]
    pub cash_pnl: f64,
    #[serde(rename = "realizedPnl", default)]
    pub realized_pnl: f64,
    #[serde(rename = "percentPnl", default)]
    pub percent_pnl: Option<f64>,
    #[serde(rename = "totalBought", default)]
    pub total_bought: f64,
    #[serde(rename = "curPrice", default)]
    pub cur_price: Option<f64>,
    pub redeemable: bool,
    pub mergeable: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(rename = "eventSlug", default)]
    pub event_slug: Option<String>,
    pub outcome: String,
    #[serde(rename = "outcomeIndex")]
    pub outcome_index: u8,
    #[serde(rename = "oppositeOutcome", default)]
    pub opposite_outcome: Option<String>,
    #[serde(rename = "oppositeAsset", default)]
    pub opposite_asset: Option<String>,
    #[serde(rename = "endDate", default)]
    pub end_date: Option<String>,
    #[serde(rename = "negativeRisk", default)]
    pub negative_risk: bool,
}

impl PositionRow {
    pub fn into_position(self, outcome_name_override: Option<String>) -> Position {
        Position {
            condition_id: self.condition_id,
            outcome_index: self.outcome_index,
            outcome_name: outcome_name_override.unwrap_or(self.outcome),
            size: self.size,
            avg_price: self.avg_price,
            current_price: self.cur_price,
            current_value: self.current_value,
            initial_value: self.initial_value,
            realized_pnl: self.realized_pnl,
            cash_pnl: self.cash_pnl,
            total_bought: self.total_bought,
            mergeable: self.mergeable,
            redeemable: self.redeemable,
            updated_at: Utc::now(),
            title: self.title,
            slug: self.slug,
            event_slug: self.event_slug,
            icon: self.icon,
            neg_risk: self.negative_risk,
            end_date: self.end_date.as_deref().and_then(parse_iso),
        }
    }
}

/// Polymarket Gamma 의 ISO 형식 변종 모두 흡수.
/// 관찰된 케이스:
///  · "2026-05-18T15:40:00Z"           (RFC3339)
///  · "2026-05-18T15:40:00.123Z"        (with millis)
///  · "2026-05-18 15:40:00+00"          (space 분리, +00 short offset) — gameStartTime
///  · "2026-05-18 15:40:00+00:00"       (with colon)
///  · "2026-05-18"                       (date only)
pub fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    // T-separated RFC3339
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    // Polymarket 의 space-separated 변종 (gameStartTime 등). %#z 는 "+00" / "+0000" / "+00:00" 모두 흡수.
    if let Ok(d) = DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%#z") {
        return Some(d.with_timezone(&Utc));
    }
    // 더 짧은 변종 (밀리초 포함)
    if let Ok(d) = DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%#z") {
        return Some(d.with_timezone(&Utc));
    }
    if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ") {
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(d, Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
        }
    }
    None
}
