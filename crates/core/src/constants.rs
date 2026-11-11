//! Forensic-derived 상수 + 외부 endpoint URL.

pub const RN1_WALLET: &str = "0x2005d16a84ceefa912d4e380cd32e7ff827875ea";
pub const RN1_ALIAS: &str = "RN1 (Scary-Edible)";

/// 2026-04-28 00:00:00 UTC — Polymarket CLOB v2 + pUSD migration.
pub const CLOB_V2_CUTOFF_TS: i64 = 1_777_334_400;

// API endpoints
pub const GAMMA_API: &str        = "https://gamma-api.polymarket.com";
pub const DATA_API: &str         = "https://data-api.polymarket.com";
pub const CLOB_API: &str         = "https://clob.polymarket.com";
pub const CLOB_WS_MARKET: &str   = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
pub const CLOB_WS_USER: &str     = "wss://ws-subscriptions-clob.polymarket.com/ws/user";

// Fee model defaults (sports markets, takerOnly + 25% rebate)
pub const DEFAULT_TAKER_FEE_RATE: f64 = 0.03;
pub const DEFAULT_MAKER_REBATE_RATE: f64 = 0.25;
/// 0.0075 — maker 가 sports 마켓에서 받는 effective rebate / notional.
pub const SPORTS_MAKER_REBATE_PCT: f64 = DEFAULT_TAKER_FEE_RATE * DEFAULT_MAKER_REBATE_RATE;
