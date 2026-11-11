//! Gamma `/markets` 클라이언트.

use anyhow::Result;
use rn1_core::{constants::GAMMA_API, Market, FeeSchedule};
use serde::Deserialize;
use crate::{DataApi, parse_iso};

#[derive(Deserialize)]
struct RawMarket {
    #[serde(rename = "conditionId")] condition_id: String,
    slug: String,
    #[serde(default, rename = "eventSlug")] event_slug: Option<String>,
    #[serde(default)] question: Option<String>,
    /// outcomes 는 ["A","B"] 문자열 또는 JSON 문자열로 옴
    #[serde(default)] outcomes: Option<serde_json::Value>,
    #[serde(default, rename = "outcomePrices")] outcome_prices: Option<serde_json::Value>,
    #[serde(default, rename = "clobTokenIds")] clob_token_ids: Option<serde_json::Value>,
    #[serde(default, rename = "feeSchedule")] fee_schedule: Option<FeeSchedule>,
    #[serde(default, rename = "negRisk")] neg_risk: bool,
    #[serde(default)] closed: bool,
    #[serde(default)] active: bool,
    #[serde(default, rename = "endDate")] end_date: Option<String>,
    #[serde(default, rename = "gameStartTime")] game_start_time: Option<String>,
    #[serde(default, rename = "acceptingOrdersTimestamp")] accepting_orders: Option<String>,
    #[serde(default, rename = "umaResolutionStatus")] uma_resolution_status: Option<String>,
    #[serde(default, rename = "lastTradePrice")] last_trade_price: Option<f64>,
    #[serde(default, rename = "bestBid")] best_bid: Option<f64>,
    #[serde(default, rename = "bestAsk")] best_ask: Option<f64>,
    #[serde(default, rename = "volumeNum")] volume_num: Option<f64>,
    #[serde(default, rename = "sportsMarketType")] sports_market_type: Option<String>,
    #[serde(default, rename = "orderMinSize")] order_min_size: Option<f64>,
    #[serde(default, rename = "orderPriceMinTickSize")] order_price_min_tick_size: Option<f64>,
    #[serde(default, rename = "rewardsMaxSpread")] rewards_max_spread: Option<f64>,
    #[serde(default)] icon: Option<String>,
}

fn parse_string_array(v: serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::Array(a) => a.into_iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        serde_json::Value::String(s) => serde_json::from_str::<Vec<String>>(&s).unwrap_or_default(),
        _ => vec![],
    }
}

fn parse_f64_array(v: serde_json::Value) -> Vec<f64> {
    match v {
        serde_json::Value::Array(a) => a.into_iter().filter_map(|x| {
            x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        }).collect(),
        serde_json::Value::String(s) => {
            let arr: Result<Vec<serde_json::Value>, _> = serde_json::from_str(&s);
            arr.map(|a| a.into_iter().filter_map(|x| {
                x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))
            }).collect()).unwrap_or_default()
        }
        _ => vec![],
    }
}

impl RawMarket {
    fn into_market(self) -> Market {
        let outcomes = self.outcomes.map(parse_string_array).unwrap_or_default();
        let outcome_prices = self.outcome_prices.map(parse_f64_array);
        let clob_token_ids = self.clob_token_ids.map(parse_string_array).unwrap_or_default();
        Market {
            condition_id: self.condition_id,
            slug: self.slug,
            event_slug: self.event_slug,
            question: self.question,
            outcomes,
            outcome_prices,
            clob_token_ids,
            neg_risk: self.neg_risk,
            closed: self.closed,
            active: self.active,
            end_date: self.end_date.as_deref().and_then(parse_iso),
            game_start_time: self.game_start_time.as_deref().and_then(parse_iso),
            accepting_orders_ts: self.accepting_orders.as_deref().and_then(parse_iso),
            uma_resolution_status: self.uma_resolution_status,
            last_trade_price: self.last_trade_price,
            best_bid: self.best_bid,
            best_ask: self.best_ask,
            volume_num: self.volume_num,
            fee_schedule: self.fee_schedule,
            sports_market_type: self.sports_market_type,
            order_min_size: self.order_min_size,
            order_price_min_tick_size: self.order_price_min_tick_size,
            rewards_max_spread: self.rewards_max_spread,
            icon: self.icon,
        }
    }
}

impl DataApi {
    /// 단일 conditionId 의 시장 메타를 가져온다. closed=true 와 closed=false 둘 다 시도.
    pub async fn fetch_market(&self, condition_id: &str) -> Result<Option<Market>> {
        for closed_flag in ["true", "false"] {
            let url = format!("{GAMMA_API}/markets?condition_ids={}&closed={}&limit=5",
                condition_id, closed_flag);
            let resp = self.client.get(&url).send().await?;
            if !resp.status().is_success() { continue; }
            let arr: Vec<RawMarket> = resp.json().await.unwrap_or_default();
            if let Some(raw) = arr.into_iter().next() {
                return Ok(Some(raw.into_market()));
            }
        }
        Ok(None)
    }
}
