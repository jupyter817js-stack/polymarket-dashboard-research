//! Wallet snapshot — on-chain USDC + CLOB-deposited USDC + positions value.
//! Pulled from Polygon RPC + Polymarket data-api + (live) CLOB balance-allowance.
//! Mirrors rn1-live/src/wallet.rs.

use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use rn1_clob::{ClobClient, L2Auth};

pub const USDC_E_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
pub const USDC_NATIVE_ADDRESS: &str = "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359";
pub const DATA_API: &str = "https://data-api.polymarket.com";

#[derive(Debug, Clone, Default, Serialize)]
pub struct WalletSnapshot {
    pub address: String,
    pub usdc_e_balance: f64,
    pub usdc_native_balance: f64,
    pub usdc_total: f64,
    pub clob_usdc_available: f64,
    pub pol_balance: f64,
    pub positions_count: usize,
    pub positions_value_usdc: f64,
    pub total_value_usdc: f64,
    pub fetched_at: String,
    pub fetch_error: Option<String>,
}

pub async fn fetch_snapshot(
    http: &Client,
    address: &str,
    rpc_url: &str,
    clob: Option<&ClobClient>,
    l2_auth: Option<&L2Auth<'_>>,
    signature_type: u8,
) -> WalletSnapshot {
    let mut snap = WalletSnapshot {
        address: address.to_lowercase(),
        fetched_at: Utc::now().to_rfc3339(),
        ..Default::default()
    };
    if !address.starts_with("0x") || address.len() != 42 {
        snap.fetch_error = Some("invalid address".into());
        return snap;
    }
    match fetch_erc20_balance(http, rpc_url, USDC_E_ADDRESS, address).await {
        Ok(b) => snap.usdc_e_balance = b,
        Err(e) => tracing::warn!(err=?e, "USDC.e balance fetch failed"),
    }
    match fetch_erc20_balance(http, rpc_url, USDC_NATIVE_ADDRESS, address).await {
        Ok(b) => snap.usdc_native_balance = b,
        Err(e) => tracing::warn!(err=?e, "USDC native balance fetch failed"),
    }
    match fetch_native_balance(http, rpc_url, address).await {
        Ok(b) => snap.pol_balance = b,
        Err(e) => tracing::warn!(err=?e, "POL balance fetch failed"),
    }
    if let (Some(c), Some(a)) = (clob, l2_auth) {
        match c.balance_allowance(a, signature_type).await {
            Ok(b) => snap.clob_usdc_available = b,
            Err(e) => tracing::warn!(err=?e, "CLOB /balance-allowance fetch failed"),
        }
    }
    match fetch_positions_summary(http, address).await {
        Ok((n, v)) => { snap.positions_count = n; snap.positions_value_usdc = v; }
        Err(e) => tracing::warn!(err=?e, "positions summary fetch failed"),
    }
    snap.usdc_total = snap.usdc_e_balance + snap.usdc_native_balance;
    snap.total_value_usdc =
        snap.usdc_total + snap.clob_usdc_available + snap.positions_value_usdc;
    snap
}

#[derive(Deserialize)] struct RpcResp { result: Option<String>, error: Option<RpcError> }
#[derive(Deserialize)] struct RpcError { message: String }

async fn rpc_call(http: &Client, rpc_url: &str, method: &str, params: serde_json::Value) -> Result<String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp = http.post(rpc_url).json(&body).send().await.context("rpc send")?;
    if !resp.status().is_success() { anyhow::bail!("rpc http {}", resp.status()); }
    let r: RpcResp = resp.json().await.context("rpc decode")?;
    if let Some(e) = r.error { anyhow::bail!("rpc error: {}", e.message); }
    r.result.ok_or_else(|| anyhow::anyhow!("rpc no result"))
}

async fn fetch_erc20_balance(http: &Client, rpc_url: &str, token: &str, owner: &str) -> Result<f64> {
    let addr_padded = format!("000000000000000000000000{}", owner.trim_start_matches("0x"));
    let data = format!("0x70a08231{}", addr_padded);
    let hex_resp = rpc_call(http, rpc_url, "eth_call", json!([
        { "to": token, "data": data }, "latest"
    ])).await?;
    let hex_s = hex_resp.trim_start_matches("0x");
    if hex_s.is_empty() || hex_s.chars().all(|c| c == '0') { return Ok(0.0); }
    let n = u128::from_str_radix(hex_s, 16).context("parse hex balance")?;
    Ok(n as f64 / 1_000_000.0)
}

async fn fetch_native_balance(http: &Client, rpc_url: &str, owner: &str) -> Result<f64> {
    let hex_resp = rpc_call(http, rpc_url, "eth_getBalance", json!([owner, "latest"])).await?;
    let hex_s = hex_resp.trim_start_matches("0x");
    if hex_s.is_empty() || hex_s.chars().all(|c| c == '0') { return Ok(0.0); }
    let n = u128::from_str_radix(hex_s, 16).context("parse hex pol balance")?;
    Ok(n as f64 / 1e18)
}

#[derive(Deserialize)] struct PosSum {
    #[serde(rename = "currentValue", default)] current_value: Option<f64>,
}

async fn fetch_positions_summary(http: &Client, address: &str) -> Result<(usize, f64)> {
    let url = format!("{DATA_API}/positions?user={address}&sizeThreshold=0.01");
    let resp = http.get(&url).send().await.context("data-api positions")?;
    if !resp.status().is_success() { anyhow::bail!("data-api positions http {}", resp.status()); }
    let positions: Vec<PosSum> = resp.json().await.context("positions decode")?;
    let total: f64 = positions.iter().filter_map(|p| p.current_value).sum();
    Ok((positions.len(), total))
}

#[derive(Deserialize, Debug, Clone)]
pub struct PositionDetail {
    #[serde(rename = "conditionId", default)] pub condition_id: String,
    #[serde(default)] pub asset: String,
    #[serde(rename = "outcomeIndex", default)] pub outcome_index: u8,
    #[serde(default)] pub size: f64,
    #[serde(rename = "avgPrice", default)] pub avg_price: f64,
}

pub async fn fetch_positions_detailed(http: &Client, address: &str) -> Result<Vec<PositionDetail>> {
    let url = format!("{DATA_API}/positions?user={address}&sizeThreshold=0.01&limit=500");
    let resp = http.get(&url).send().await.context("data-api positions detailed")?;
    if !resp.status().is_success() {
        anyhow::bail!("data-api positions http {}", resp.status());
    }
    let positions: Vec<PositionDetail> = resp.json().await.context("positions detailed decode")?;
    Ok(positions)
}
