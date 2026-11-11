//! Trader CLI / env config — mirrors the rn1-live knobs from
//! `docs/RN1_LIVE_LOGIC_HANDOFF.md` §8.

use anyhow::Result;
use clap::Parser;
use rn1_planner::PlannerConfig;

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Paper, Live }
impl Mode {
    pub fn is_live(self) -> bool { matches!(self, Mode::Live) }
    pub fn as_str(self) -> &'static str { if self.is_live() { "live" } else { "paper" } }
}

#[derive(Parser, Debug, Clone)]
#[command(name = "rn1-trader", version,
    about = "Polymarket pair-completion trader (RN1-style). See docs/RN1_LIVE_LOGIC_HANDOFF.md.")]
pub struct Config {
    #[arg(long, env = "RN1_LIVE_MODE", default_value = "paper")]
    pub mode: Mode,

    // ── Polymarket auth ─────────────────────────────────────────────────
    #[arg(long, env = "POLYMARKET_SIGNER_PK", default_value = "")]
    pub signer_pk: String,
    #[arg(long, env = "POLYMARKET_PROXY_ADDRESS", default_value = "")]
    pub proxy_address: String,
    /// 0=EOA, 1=POLY_PROXY, 2=POLY_GNOSIS_SAFE, 3=POLY_1271. -1 = auto.
    #[arg(long, env = "POLYMARKET_SIGNATURE_TYPE", default_value_t = -1)]
    pub signature_type: i32,
    #[arg(long, env = "POLY_API_KEY",     default_value = "")] pub api_key: String,
    #[arg(long, env = "POLY_API_SECRET",  default_value = "")] pub api_secret: String,
    #[arg(long, env = "POLY_PASSPHRASE",  default_value = "")] pub passphrase: String,

    // ── Strategy core ───────────────────────────────────────────────────
    #[arg(long, env = "RN1_CUMULATIVE_TARGET_MAX", default_value_t = 0.95)]
    pub cumulative_target_max: f64,
    #[arg(long, env = "RN1_CUMULATIVE_AIM",        default_value_t = 0.90)]
    pub cumulative_aim: f64,
    #[arg(long, env = "RN1_ENDGAME_AFTER_MIN",     default_value_t = 75)]
    pub endgame_after_min: i64,

    // ── Sizing ──────────────────────────────────────────────────────────
    #[arg(long, env = "RN1_QUOTE_SHARES",     default_value_t = 6.0)]  pub quote_shares: f64,
    #[arg(long, env = "RN1_MIN_ORDER_USDC",   default_value_t = 1.0)]  pub min_order_usdc: f64,
    #[arg(long, env = "RN1_MIN_ORDER_SHARES", default_value_t = 5.0)]  pub min_order_shares: f64,
    #[arg(long, env = "RN1_LADDER_DEPTH",     default_value_t = 4)]    pub ladder_depth: usize,
    #[arg(long, env = "RN1_LADDER_TICK_CENTS",default_value_t = 1)]    pub ladder_tick_cents: i32,
    #[arg(long, env = "RN1_COMPLETION_MULT",  default_value_t = 2.0)]  pub completion_mult: f64,
    #[arg(long, env = "RN1_DIP_TRIGGER_CENTS",default_value_t = 3)]    pub dip_trigger_cents: i32,
    #[arg(long, env = "RN1_DIP_SCALE_MULT",   default_value_t = 3.0)]  pub dip_scale_mult: f64,
    #[arg(long, env = "RN1_ENABLE_COMPLETION_TAKER", default_value_t = false)]
    pub enable_completion_taker: bool,

    // ── Collapse / sweep ────────────────────────────────────────────────
    // Tuned for in-play volatility (e.g. 5분 내 20¢ collapse 같은 매치 내 급변):
    //   drop 5¢ / window 30s 가 8¢/60s 보다 빠르게 트리거. false positive 약간 ↑.
    //   handoff §3.5 의 "first guess, tune in paper" 권장에 따라 수정.
    #[arg(long, env = "RN1_COLLAPSE_DROP_CENTS",     default_value_t = 5)]   pub collapse_drop_cents: i32,
    #[arg(long, env = "RN1_COLLAPSE_WINDOW_S",       default_value_t = 30)]  pub collapse_window_s: i64,
    #[arg(long, env = "RN1_COLLAPSE_LOOKBACK_S",     default_value_t = 600)] pub collapse_lookback_s: i64,
    #[arg(long, env = "RN1_COLLAPSE_BREAKOUT_CENTS", default_value_t = 3)]   pub collapse_breakout_cents: i32,
    #[arg(long, env = "RN1_SWEEP_HOLD_S",            default_value_t = 180)] pub sweep_hold_s: i64,
    #[arg(long, env = "RN1_SWEEP_TARGET_MAX",        default_value_t = 1.00)] pub sweep_target_max: f64,
    #[arg(long, env = "RN1_SWEEP_TAKER_SLIPPAGE_CENTS", default_value_t = 3)] pub sweep_taker_slippage_cents: i32,
    #[arg(long, env = "RN1_SWEEP_SIZE_MULT",         default_value_t = 5.0)] pub sweep_size_mult: f64,
    #[arg(long, env = "RN1_MAX_NAKED_SHARES",        default_value_t = 12.0)] pub max_naked_shares: f64,

    // ── BAIL / stale-maker hygiene (2026-05-24) ─────────────────────────
    // BAIL = single-leg dead-loser trap. When the held leg's market bid drops
    // below `bail_loser_threshold`, the OTHER leg's completion cap is
    // unreachable via maker pricing — cancel all resting + idle.
    #[arg(long, env = "RN1_BAIL_ENABLED",          default_value_t = true)] pub bail_enabled: bool,
    #[arg(long, env = "RN1_BAIL_LOSER_THRESHOLD",  default_value_t = 0.05)] pub bail_loser_threshold: f64,
    // Stale-maker: makers older than max_age AND further than dist_cents from
    // current market bid on their leg are cancelled (capital release).
    #[arg(long, env = "RN1_STALE_MAKER_MAX_AGE_S", default_value_t = 4 * 3600)] pub stale_maker_max_age_s: i64,
    #[arg(long, env = "RN1_STALE_MAKER_DIST_CENTS",default_value_t = 10)]       pub stale_maker_dist_cents: i32,

    // ── MERGE recycle (paper-mode §6.5) ─────────────────────────────────
    // When per-event cap is tight, merge matched pairs into USDC at $1/pair
    // so the dip-ladder can keep placing. Always net-positive: realized_pnl
    // = shares × (1 − matched_vwap_at_merge). Live CTF mergePositions() is
    // deferred to Phase B (Polygon signer + gas).
    #[arg(long, env = "RN1_MERGE_ENABLED",          default_value_t = true)] pub merge_enabled: bool,
    #[arg(long, env = "RN1_MIN_MERGE_SHARES",       default_value_t = 5.0)]  pub min_merge_shares: f64,
    #[arg(long, env = "RN1_MERGE_BUFFER_SHARES",    default_value_t = 1.0)]  pub merge_buffer_shares: f64,
    /// Eager MERGE — recycle whenever matched ≥ N (no cap pressure needed).
    /// 2026-05-25: 26h run showed cap-pressure-only triggered only 3 merges.
    /// Eager fires continuously, keeps the ladder flush with USDC.
    /// 2026-05-25 (C): default lowered 10 → 5 for higher capital turnover.
    #[arg(long, env = "RN1_MERGE_EAGER_MIN_SHARES", default_value_t = 5.0)] pub merge_eager_min_shares: f64,

    // ── In-play Step 3 suppression (2026-05-25) ─────────────────────────
    // Dip-ladder during live game systematically loaded the loser side
    // (Kecmano-Marozsa 70.96 loser shares, Ruiz-Tirante 96.00 loser shares).
    // Pre-match only by default.
    #[arg(long, env = "RN1_STEP3_SUPPRESS_IN_PLAY", default_value_t = true)] pub step3_suppress_in_play: bool,

    // ── Asymmetric BAIL — over-loaded loser side ────────────────────────
    #[arg(long, env = "RN1_BAIL_ASYMMETRIC_RATIO",     default_value_t = 0.5)]  pub bail_asymmetric_ratio: f64,
    #[arg(long, env = "RN1_BAIL_ASYMMETRIC_THRESHOLD", default_value_t = 0.10)] pub bail_asymmetric_threshold: f64,

    // ── Pre-match emergency close (2026-05-25) ──────────────────────────
    // Sweep lagging leg when game_start is imminent and we hold single-leg
    // naked exposure. Prevents Rinderknech-Rodionov / Kecmano-Marozsa.
    #[arg(long, env = "RN1_EMERGENCY_CLOSE_ENABLED",         default_value_t = true)] pub emergency_close_enabled: bool,
    /// 2026-05-25 (E): default expanded 300s → 1800s (5 min → 30 min).
    #[arg(long, env = "RN1_EMERGENCY_CLOSE_SECS",            default_value_t = 1800)] pub emergency_close_secs: i64,
    #[arg(long, env = "RN1_EMERGENCY_CLOSE_SLIPPAGE_CENTS",  default_value_t = 2)]    pub emergency_close_slippage_cents: i32,

    // ── In-play loser-side guards (2026-05-25 A/B/D) ────────────────────
    #[arg(long, env = "RN1_STEP2_IN_PLAY_MIN_BID",       default_value_t = 0.30)] pub step2_in_play_min_bid: f64,
    #[arg(long, env = "RN1_DEAD_LEG_PROB_THRESHOLD",     default_value_t = 0.20)] pub dead_leg_prob_threshold: f64,
    #[arg(long, env = "RN1_BAIL_STRONG_RATIO",           default_value_t = 2.0)]  pub bail_strong_ratio: f64,
    /// 2026-05-25: 0.50 → 0.30 after Svitolina-Bondar false positive.
    #[arg(long, env = "RN1_BAIL_STRONG_THRESHOLD",       default_value_t = 0.30)] pub bail_strong_threshold: f64,

    // ── S2 Exit Layer (2026-05-25 time-arb on naked legs) ────────────────
    #[arg(long, env = "RN1_EXIT_ENABLED",            default_value_t = true)] pub exit_enabled: bool,
    /// 2026-05-26: 1.75 → 1.5 (Shimizu 0.89 target never filled).
    #[arg(long, env = "RN1_EXIT_VWAP_MULTIPLIER",    default_value_t = 1.5)]  pub exit_vwap_multiplier: f64,
    /// 2026-05-26: 0.50 → 0.40 (looser floor for fillability).
    #[arg(long, env = "RN1_EXIT_MIN_PRICE",          default_value_t = 0.40)] pub exit_min_price: f64,
    #[arg(long, env = "RN1_MIN_SELL_SHARES",         default_value_t = 5.0)]  pub min_sell_shares: f64,

    // ── Risk caps ───────────────────────────────────────────────────────
    #[arg(long, env = "RN1_PER_EVENT_CAP_USDC",  default_value_t = 50.0)]  pub per_event_cap_usdc: f64,
    #[arg(long, env = "RN1_DAILY_LOSS_CAP_USDC", default_value_t = 40.0)]  pub daily_loss_cap_usdc: f64,
    #[arg(long, env = "RN1_DAILY_GROSS_CAP_USDC",default_value_t = 120.0)] pub daily_gross_cap_usdc: f64,
    #[arg(long, env = "RN1_MIN_USDC_BUFFER",     default_value_t = 100.0)] pub min_usdc_buffer: f64,

    // ── Universe ────────────────────────────────────────────────────────
    #[arg(long, env = "RN1_UNIVERSE",         default_value = "atp,wta")] pub universe: String,
    #[arg(long, env = "RN1_MAX_OPEN_MARKETS", default_value_t = 1)]       pub max_open_markets: usize,
    #[arg(long, env = "RN1_MID_MIN",          default_value_t = 0.05)]    pub mid_min: f64,
    #[arg(long, env = "RN1_MID_MAX",          default_value_t = 0.95)]    pub mid_max: f64,
    #[arg(long, env = "RN1_MIN_EVENT_VOLUME", default_value_t = 25000.0)] pub min_event_volume: f64,
    #[arg(long, env = "RN1_EXCLUDE_SLUG_SUBSTRINGS", default_value = "doubles,itf-")]
    pub exclude_slug_substrings: String,
    /// Restrict trader to a single market slug (Phase A). Empty = autonomous scan.
    #[arg(long, env = "RN1_FOCUS_SLUG", default_value = "")] pub focus_slug: String,

    // ── Driver timing ───────────────────────────────────────────────────
    // 500ms (was 1500) — in-play 급변 시 더 빠른 반응. CLOB book() 호출 부담
    // 약간 ↑ 이지만 의미 있는 burst 만 통과 (decision_tick 가 자체 throttle).
    #[arg(long, env = "RN1_BOOK_DEBOUNCE_MS", default_value_t = 500)] pub book_debounce_ms: u64,
    #[arg(long, env = "RN1_POLL_BACKSTOP_S",  default_value_t = 10)]   pub poll_backstop_s: u64,
    #[arg(long, env = "RN1_GAMMA_REFRESH_S",  default_value_t = 180)]  pub gamma_refresh_s: u64,

    // ── Wallet / RPC ────────────────────────────────────────────────────
    #[arg(long, env = "POLYGON_RPC_URL", default_value = "https://polygon.drpc.org")]
    pub polygon_rpc_url: String,
    #[arg(long, env = "RN1_WALLET_REFRESH_S", default_value_t = 60)] pub wallet_refresh_s: u64,

    // ── Misc ────────────────────────────────────────────────────────────
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Smoke test: place ONE deep-OTM order, verify, cancel, exit. Requires live mode.
    #[arg(long, env = "RN1_SMOKE_TEST", default_value_t = false)] pub smoke_test: bool,
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();
        let c = Self::parse();
        c.validate()?;
        Ok(c)
    }

    pub fn effective_signature_type(&self) -> u8 {
        if self.signature_type >= 0 { return self.signature_type as u8; }
        if self.proxy_address.is_empty() { 0 } else { 3 }
    }

    pub fn maker_address_override(&self) -> Option<String> {
        if self.proxy_address.is_empty() { None } else { Some(self.proxy_address.clone()) }
    }

    pub fn universe_prefixes(&self) -> Vec<String> {
        self.universe.split(',').map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty()).collect()
    }

    pub fn exclude_substrings(&self) -> Vec<String> {
        self.exclude_slug_substrings.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty()).collect()
    }

    pub fn to_planner(&self) -> PlannerConfig {
        PlannerConfig {
            cumulative_target_max: self.cumulative_target_max,
            cumulative_aim: self.cumulative_aim,
            quote_shares: self.quote_shares,
            min_order_usdc: self.min_order_usdc,
            min_order_shares: self.min_order_shares,
            ladder_depth: self.ladder_depth,
            ladder_tick_cents: self.ladder_tick_cents,
            completion_mult: self.completion_mult,
            dip_trigger_cents: self.dip_trigger_cents,
            dip_scale_mult: self.dip_scale_mult,
            enable_completion_taker: self.enable_completion_taker,
            collapse_drop_cents: self.collapse_drop_cents,
            collapse_window_s: self.collapse_window_s,
            collapse_lookback_s: self.collapse_lookback_s,
            collapse_breakout_cents: self.collapse_breakout_cents,
            sweep_hold_s: self.sweep_hold_s,
            sweep_target_max: self.sweep_target_max,
            sweep_taker_slippage_cents: self.sweep_taker_slippage_cents,
            sweep_size_mult: self.sweep_size_mult,
            max_naked_shares: self.max_naked_shares,
            mid_min: self.mid_min,
            bail_enabled: self.bail_enabled,
            bail_loser_threshold: self.bail_loser_threshold,
            stale_maker_max_age_s: self.stale_maker_max_age_s,
            stale_maker_dist_cents: self.stale_maker_dist_cents,
            merge_enabled: self.merge_enabled,
            min_merge_shares: self.min_merge_shares,
            merge_buffer_shares: self.merge_buffer_shares,
            merge_eager_min_shares: self.merge_eager_min_shares,
            step3_suppress_in_play: self.step3_suppress_in_play,
            bail_asymmetric_ratio: self.bail_asymmetric_ratio,
            bail_asymmetric_threshold: self.bail_asymmetric_threshold,
            emergency_close_enabled: self.emergency_close_enabled,
            emergency_close_secs: self.emergency_close_secs,
            emergency_close_slippage_cents: self.emergency_close_slippage_cents,
            step2_in_play_min_bid: self.step2_in_play_min_bid,
            dead_leg_prob_threshold: self.dead_leg_prob_threshold,
            bail_strong_ratio: self.bail_strong_ratio,
            bail_strong_threshold: self.bail_strong_threshold,
            exit_enabled: self.exit_enabled,
            exit_vwap_multiplier: self.exit_vwap_multiplier,
            exit_min_price: self.exit_min_price,
            min_sell_shares: self.min_sell_shares,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.mode.is_live() && self.signer_pk.is_empty() {
            anyhow::bail!("live mode requires POLYMARKET_SIGNER_PK to be set");
        }
        if !self.proxy_address.is_empty()
            && (!self.proxy_address.starts_with("0x") || self.proxy_address.len() != 42)
        {
            anyhow::bail!("POLYMARKET_PROXY_ADDRESS must be 0x-prefixed 42-char hex");
        }
        let sigtype = self.effective_signature_type();
        if !(0..=3).contains(&sigtype) {
            anyhow::bail!("POLYMARKET_SIGNATURE_TYPE must be 0..=3");
        }
        if sigtype != 0 && self.proxy_address.is_empty() {
            anyhow::bail!("signatureType={} requires POLYMARKET_PROXY_ADDRESS", sigtype);
        }
        if self.mid_min >= self.mid_max {
            anyhow::bail!("RN1_MID_MIN must be < RN1_MID_MAX");
        }
        if !(0.5..=1.0).contains(&self.cumulative_target_max) {
            anyhow::bail!("RN1_CUMULATIVE_TARGET_MAX must be in [0.5, 1.0]");
        }
        if !(0.5..=self.cumulative_target_max).contains(&self.cumulative_aim) {
            anyhow::bail!("RN1_CUMULATIVE_AIM must be in [0.5, RN1_CUMULATIVE_TARGET_MAX]");
        }
        if !(0.5..=1.1).contains(&self.sweep_target_max) {
            anyhow::bail!("RN1_SWEEP_TARGET_MAX must be in [0.5, 1.1]");
        }
        if self.per_event_cap_usdc < 2.0 * self.min_order_usdc {
            anyhow::bail!("RN1_PER_EVENT_CAP_USDC must be ≥ 2× RN1_MIN_ORDER_USDC");
        }
        if !(0.0..=0.5).contains(&self.bail_loser_threshold) {
            anyhow::bail!("RN1_BAIL_LOSER_THRESHOLD must be in [0.0, 0.5]");
        }
        if self.stale_maker_max_age_s < 0 {
            anyhow::bail!("RN1_STALE_MAKER_MAX_AGE_S must be ≥ 0");
        }
        if !(0..=100).contains(&self.stale_maker_dist_cents) {
            anyhow::bail!("RN1_STALE_MAKER_DIST_CENTS must be in [0, 100]");
        }
        Ok(())
    }
}
