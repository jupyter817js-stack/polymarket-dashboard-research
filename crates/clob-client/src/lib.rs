//! CLOB v2 REST client — public + L1/L2 authenticated endpoints.
//!
//! Public:        `/book`
//! L1 ClobAuth:   `/auth/api-key`, `/auth/derive-api-key`
//! L2 HMAC:       `/order` (POST/DELETE), `/data/orders` (GET), `/balance-allowance`
//!
//! Live-mode order POST: `orderType="GTC"` (maker-only) or `"FAK"` (taker — used
//! for collapse sweeps and, if `enable_completion_taker`, completion).

use anyhow::{Context, Result};
use reqwest::{Client, Method};
use rn1_core::constants::CLOB_API;
use rn1_signer::{hmac_l2_signature, now_unix_secs, PolySigner};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

#[derive(Clone)]
pub struct ClobClient { http: Client }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderBook {
    #[serde(default)] pub market: Option<String>,
    pub asset_id: String,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    #[serde(default)] pub hash: Option<String>,
    #[serde(default)] pub timestamp: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Level { pub price: String, pub size: String }

impl OrderBook {
    /// best bid = MAX bid price (regardless of array order).
    ///
    /// Polymarket CLOB returns `bids` ascending (low→high) and `asks` DESCENDING
    /// (high→low). Earlier code assumed both ascending — `asks.first()` then
    /// returned the *highest* ask (e.g. $0.99) instead of the true best $0.58,
    /// silently inflating mid prices by 20¢+. Scanning for max/min is sort-agnostic.
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.iter()
            .filter_map(|l| l.price.parse::<f64>().ok())
            .filter(|p| *p > 0.0)
            .fold(None, |acc: Option<f64>, p| Some(acc.map_or(p, |a| a.max(p))))
    }
    /// best ask = MIN ask price (regardless of array order).
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.iter()
            .filter_map(|l| l.price.parse::<f64>().ok())
            .filter(|p| *p > 0.0)
            .fold(None, |acc: Option<f64>, p| Some(acc.map_or(p, |a| a.min(p))))
    }
}

/// V2 CTF Exchange order body — mirrors clob-client-v2 `orderToJsonV2`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClobOrder {
    pub salt: u64,
    pub maker: String,
    pub signer: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub taker: String,
    #[serde(rename = "tokenId")]      pub token_id: String,
    #[serde(rename = "makerAmount")]  pub maker_amount: String,
    #[serde(rename = "takerAmount")]  pub taker_amount: String,
    pub side: String,                            // "BUY" / "SELL"
    #[serde(rename = "signatureType")] pub signature_type: u8,
    pub timestamp: String,
    pub expiration: String,
    pub metadata: String,
    pub builder:  String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClobPostOrderBody {
    #[serde(rename = "deferExec")] pub defer_exec: bool,
    #[serde(rename = "postOnly")]  pub post_only:  bool,
    pub order: ClobOrder,
    pub owner: String,
    #[serde(rename = "orderType")] pub order_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderResponse {
    #[serde(default)] pub success: Option<bool>,
    #[serde(rename = "errorMsg", default)] pub error_msg: Option<String>,
    #[serde(rename = "orderID", default)] pub order_id: Option<String>,
    #[serde(rename = "orderId", default)] pub order_id_alt: Option<String>,
    #[serde(default)] pub status: Option<String>,
    #[serde(rename = "transactionsHashes", default)] pub tx_hashes: Option<Vec<String>>,
}

impl OrderResponse {
    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref().or(self.order_id_alt.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyResponse {
    #[serde(rename = "apiKey")] pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

#[derive(Debug, Clone)]
pub struct L2Auth<'a> {
    pub address: &'a str,
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub passphrase: &'a str,
}

impl ClobClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            // Match clob-client-v2 User-Agent so any UA-keyed server logic accepts us.
            .user_agent("@polymarket/clob-client")
            .gzip(true)
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self { http })
    }

    // ── public ──────────────────────────────────────────────────────────
    pub async fn book(&self, token_id: &str) -> Result<OrderBook> {
        let url = format!("{CLOB_API}/book?token_id={token_id}");
        let r = self.http.get(&url).send().await.context("clob /book")?;
        if !r.status().is_success() { anyhow::bail!("clob /book http {}", r.status()); }
        Ok(r.json().await.context("clob /book decode")?)
    }

    // ── L1: API-key derive / fetch ──────────────────────────────────────
    pub async fn derive_api_key(&self, signer: &PolySigner) -> Result<ApiKeyResponse> {
        let ts = now_unix_secs();
        let sig = signer.sign_clob_auth(ts, 0)?;
        let url = format!("{CLOB_API}/auth/api-key");
        let r = self.http.post(&url)
            .header("POLY_ADDRESS", signer.address_lower())
            .header("POLY_SIGNATURE", sig)
            .header("POLY_TIMESTAMP", ts.to_string())
            .header("POLY_NONCE", "0")
            .send().await.context("derive_api_key POST")?;
        let status = r.status();
        let body = r.text().await.context("read body")?;
        if !status.is_success() { anyhow::bail!("derive_api_key http {}: {body}", status); }
        serde_json::from_str(&body).with_context(|| format!("decode api-key resp: {body}"))
    }

    pub async fn get_api_key(&self, signer: &PolySigner) -> Result<Option<ApiKeyResponse>> {
        let ts = now_unix_secs();
        let sig = signer.sign_clob_auth(ts, 0)?;
        let url = format!("{CLOB_API}/auth/derive-api-key");
        let r = self.http.get(&url)
            .header("POLY_ADDRESS", signer.address_lower())
            .header("POLY_SIGNATURE", sig)
            .header("POLY_TIMESTAMP", ts.to_string())
            .header("POLY_NONCE", "0")
            .send().await.context("get_api_key GET")?;
        if r.status().as_u16() == 404 { return Ok(None); }
        let status = r.status();
        let body = r.text().await.context("read body")?;
        if !status.is_success() { anyhow::bail!("get_api_key http {}: {body}", status); }
        Ok(Some(serde_json::from_str(&body).with_context(|| format!("decode: {body}"))?))
    }

    // ── L2: order POST / DELETE / GET ──────────────────────────────────
    pub async fn post_order(&self, auth: &L2Auth<'_>, body: &ClobPostOrderBody) -> Result<OrderResponse> {
        let path = "/order";
        let body_json = serde_json::to_string(body).context("serialize order body")?;
        tracing::debug!(body=%body_json, "POST /order");
        let ts = now_unix_secs();
        let sig = hmac_l2_signature(auth.api_secret, ts, "POST", path, &body_json)?;
        let url = format!("{CLOB_API}{path}");
        let r = self.http.post(&url)
            .header("POLY_ADDRESS", auth.address)
            .header("POLY_API_KEY", auth.api_key)
            .header("POLY_PASSPHRASE", auth.passphrase)
            .header("POLY_TIMESTAMP", ts.to_string())
            .header("POLY_SIGNATURE", sig)
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .header("Connection", "keep-alive")
            .body(body_json).send().await.context("post_order POST")?;
        let status = r.status();
        let resp_body = r.text().await.context("read body")?;
        if !status.is_success() { anyhow::bail!("post_order http {}: {resp_body}", status); }
        serde_json::from_str(&resp_body).with_context(|| format!("decode post_order: {resp_body}"))
    }

    /// GET /data/orders — returns the raw response body (one element vec for compatibility).
    pub async fn get_open_orders(&self, auth: &L2Auth<'_>, asset_id: Option<&str>) -> Result<Vec<String>> {
        let hmac_path = "/data/orders";
        let url = match asset_id {
            Some(a) => format!("{CLOB_API}{hmac_path}?market={a}"),
            None    => format!("{CLOB_API}{hmac_path}"),
        };
        let ts = now_unix_secs();
        let sig = hmac_l2_signature(auth.api_secret, ts, "GET", hmac_path, "")?;
        let r = self.http.get(&url)
            .header("POLY_ADDRESS", auth.address)
            .header("POLY_API_KEY", auth.api_key)
            .header("POLY_PASSPHRASE", auth.passphrase)
            .header("POLY_TIMESTAMP", ts.to_string())
            .header("POLY_SIGNATURE", sig)
            .send().await.context("get_open_orders GET")?;
        let status = r.status();
        let body = r.text().await.context("read body")?;
        if !status.is_success() { anyhow::bail!("get_open_orders http {}: {body}", status); }
        Ok(vec![body])
    }

    pub async fn cancel_order(&self, auth: &L2Auth<'_>, order_id: &str) -> Result<()> {
        let path = "/order";
        let body_json = serde_json::to_string(&json!({"orderID": order_id})).unwrap();
        let ts = now_unix_secs();
        let sig = hmac_l2_signature(auth.api_secret, ts, "DELETE", path, &body_json)?;
        let url = format!("{CLOB_API}{path}");
        let r = self.http.request(Method::DELETE, &url)
            .header("POLY_ADDRESS", auth.address)
            .header("POLY_API_KEY", auth.api_key)
            .header("POLY_PASSPHRASE", auth.passphrase)
            .header("POLY_TIMESTAMP", ts.to_string())
            .header("POLY_SIGNATURE", sig)
            .header("Content-Type", "application/json")
            .body(body_json).send().await.context("cancel_order DELETE")?;
        let status = r.status();
        if !status.is_success() {
            let body = r.text().await.unwrap_or_default();
            anyhow::bail!("cancel_order http {}: {body}", status);
        }
        Ok(())
    }

    /// `/balance-allowance` (L2). Returns the deposited USDC available for trading.
    /// HMAC is computed over the BARE path, but the request URL appends
    /// `?asset_type=COLLATERAL&signature_type=N`.
    pub async fn balance_allowance(&self, auth: &L2Auth<'_>, signature_type: u8) -> Result<f64> {
        let hmac_path = "/balance-allowance";
        let url = format!(
            "{CLOB_API}{hmac_path}?asset_type=COLLATERAL&signature_type={}",
            signature_type
        );
        let ts = now_unix_secs();
        let sig = hmac_l2_signature(auth.api_secret, ts, "GET", hmac_path, "")?;
        let r = self.http.get(&url)
            .header("POLY_ADDRESS", auth.address)
            .header("POLY_API_KEY", auth.api_key)
            .header("POLY_PASSPHRASE", auth.passphrase)
            .header("POLY_TIMESTAMP", ts.to_string())
            .header("POLY_SIGNATURE", sig)
            .send().await.context("clob /balance-allowance")?;
        let status = r.status();
        let body = r.text().await.context("read body")?;
        if !status.is_success() { anyhow::bail!("clob /balance-allowance http {}: {body}", status); }
        let v: serde_json::Value = serde_json::from_str(&body).context("decode")?;
        let bal_str = v.get("balance").and_then(|x| x.as_str()).unwrap_or("0");
        let n: u128 = bal_str.parse().context("parse balance int")?;
        Ok(n as f64 / 1_000_000.0)
    }

    pub fn exchange_for(neg_risk: bool) -> alloy_primitives::Address {
        rn1_signer::exchange_address_for(neg_risk)
    }
}
