//! Pair-completion planner — the decision core, lifted verbatim from rn1-live.
//!
//! `plan_market` is PURE: given a current pair state (reconciled on-chain
//! position + live books) and affordability, returns orders to ADD and resting
//! order ids to CANCEL for the tick. Identical in paper/live. The single
//! stateful piece is `CollapseDetector`, which the driver owns and ticks every
//! decision cycle.
//!
//! Decision priority (top-down, highest first):
//!   1. SWEEP     — collapse detected on a leg → buy it aggressively (taker)
//!   2. COMPLETE  — naked (unmatched) shares exist → buy the lagging leg
//!   3. ACCUMULATE — ladder both legs at dip prices (alongside Step 2)
//!
//! Step 2 and Step 3 RUN TOGETHER (post 2026-05-24 refactor). Naked positions no
//! longer suppress new-pair accumulation — the bot continues to track real-time
//! dips and place fresh paired ladders. The anchor-leg trap is prevented by the
//! per-rung **weighted post-fill vwap** check (`would_match_vwap_with_rung`)
//! rather than the old blanket "no Step 3 while naked" guard, which froze
//! capital and missed downward dip opportunities.
//!
//! Aim vs cap separation (key alpha): two cumulative-vwap parameters, not one.
//!   `cumulative_aim`        — normal accumulation/completion target (e.g. 0.90)
//!   `cumulative_target_max` — hard ceiling (e.g. 0.95). Never placed above.
//! In endgame, Step 2's complete_cap relaxes from aim up to target.
//!
//! All bug-fixes documented in `docs/RN1_LIVE_LOGIC_HANDOFF.md` §5 are baked in:
//!   - cap_breach requires q[other] > 0 (no churn on new pairs)
//!   - lag_price = min(market_bid, cap) (never idle when bid above cap)
//!   - no outer pair-sum gate in Step 3 (laddered descent handles it)
//!
//! Post-handoff extensions (2026-05-24, paper-mode lessons):
//!   - **BAIL mode** — single-leg trap: when only one leg is filled and its
//!     market price has collapsed below `bail_loser_threshold`, completion bids
//!     on the other side can no longer be reached via maker pricing (cap is
//!     too low vs market). Cancel all resting and stop placing.
//!   - **Stale-maker cancel** — a maker order older than `stale_maker_max_age_s`
//!     whose price is more than `stale_maker_dist_cents` from the current market
//!     bid on its leg is unlikely to ever fill. Cancel to release capital.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Pure config struct — the trader app fills it from env/clap and passes by ref.
/// Defaults match `docs/RN1_LIVE_LOGIC_HANDOFF.md` §8 unless caller overrides.
#[derive(Debug, Clone)]
pub struct PlannerConfig {
    // strategy core
    pub cumulative_target_max: f64,
    pub cumulative_aim: f64,
    // sizing
    pub quote_shares: f64,
    pub min_order_usdc: f64,
    pub min_order_shares: f64,
    pub ladder_depth: usize,
    pub ladder_tick_cents: i32,
    pub completion_mult: f64,
    pub dip_trigger_cents: i32,
    pub dip_scale_mult: f64,
    pub enable_completion_taker: bool,
    // collapse / sweep
    pub collapse_drop_cents: i32,
    pub collapse_window_s: i64,
    pub collapse_lookback_s: i64,
    pub collapse_breakout_cents: i32,
    pub sweep_hold_s: i64,
    pub sweep_target_max: f64,
    pub sweep_taker_slippage_cents: i32,
    pub sweep_size_mult: f64,
    pub max_naked_shares: f64,
    // selection floor for completion price
    pub mid_min: f64,
    // BAIL — single-leg dead-loser trap. When q[other]=0 and bid[c] ≤
    // bail_loser_threshold, the other-side completion bid is unreachable
    // (cap too low vs market). Cancel resting + idle.
    pub bail_loser_threshold: f64,
    pub bail_enabled: bool,
    // Stale-maker auto-cancel. Free capital from makers that have drifted
    // far from the market and aged out.
    pub stale_maker_max_age_s: i64,
    pub stale_maker_dist_cents: i32,
    // MERGE recycling — when per-event cap blocks new rungs, recover capital
    // from matched pairs at $1/pair (handoff §6.5). Paper-mode simulation
    // until the live CTF mergePositions() contract call lands (Phase B).
    pub merge_enabled: bool,
    /// Minimum matched shares required to emit a MergeAction.
    pub min_merge_shares: f64,
    /// Buffer kept in matched (un-merged) so a small position remains paired.
    /// Prevents over-recycling when matched is right at threshold.
    pub merge_buffer_shares: f64,
    /// **Eager** MERGE — recycle whenever matched_pairs ≥ this value, even
    /// without cap pressure. Default 10 → after 10 matched pairs (~$10 of
    /// recoverable capital), bring forward the $1/pair payoff and keep the
    /// ladder running on fresh dips. Disables cap-pressure-only mode.
    pub merge_eager_min_shares: f64,

    // In-play Step 3 suppression — when `game_started=true`, ladder
    // accumulation systematically loads the loser side (the dipping price
    // attracts fills, the rising side never reaches our maker). Pre-2026-05-25
    // the bot lost ~$80 on 9 markets through this anchor-leg trap. Turning
    // this on restricts Step 3 to pre-match only; Steps 1 (Sweep) and 2
    // (Completion) still run during the game.
    pub step3_suppress_in_play: bool,

    // Asymmetric BAIL — fires when the bot holds BOTH legs but one is heavily
    // over-loaded AND its market bid has collapsed. Catches the case the
    // single-leg `loser_leg_trap` misses (e.g. Kecmano-Marozsa: 70.96 loser
    // shares + 1 matched, bid collapsed but q[other]>0).
    /// naked > matched × ratio triggers asymmetric BAIL when bid is low.
    pub bail_asymmetric_ratio: f64,
    /// Bid of the over-loaded leg must be ≤ this for asymmetric BAIL.
    pub bail_asymmetric_threshold: f64,

    // ── Pre-match emergency close ───────────────────────────────────────
    // When game_start is within `emergency_close_secs` AND we have single-leg
    // naked exposure, sweep the lagging leg at ask to lock in matched pairs
    // before in-play volatility. Prevents the Rinderknech-Rodionov /
    // Kecmano-Marozsa pattern: ladder filled one side pre-match, market moved
    // away, and we entered in-play with single-leg directional exposure.
    pub emergency_close_enabled: bool,
    pub emergency_close_secs: i64,
    /// Extra ¢ on top of ask for the emergency taker sweep (slippage buffer).
    pub emergency_close_slippage_cents: i32,

    // ── In-play loser-side guards (2026-05-25 Busta-Lehecka post-mortem) ──
    // Even with Step 3 in-play suppression, Step 2 (Completion) and Step 1
    // (Sweep) can still pile into the dropping leg → loser-side accumulation
    // (e.g. Lehecka 13:1 imbalance). Block both when the leg's bid OR
    // outcome_prob is below thresholds.
    /// If `bid[lag] < step2_in_play_min_bid`, no Completion bid post-game.
    pub step2_in_play_min_bid: f64,
    /// If `outcome_prob[c] ≤ dead_leg_prob_threshold`, leg `c` is "dead"
    /// (Polymarket-implied loser) and Completion+Sweep skip it.
    pub dead_leg_prob_threshold: f64,
    /// Asymmetric BAIL second-tier: `naked > matched × ratio` AND
    /// `bid[ahead] ≤ threshold` triggers BAIL even at higher bids.
    pub bail_strong_ratio: f64,
    pub bail_strong_threshold: f64,

    // ── S2 Time-Arb Exit Layer (2026-05-25) ─────────────────────────────
    // Backtest on 3-week ATP/WTA showed +22.5% avg PnL/trade at 80% win
    // rate when SELLing naked legs whose price rebounded ≥ 1.75× entry vwap.
    // This is an EXIT on top of RN1 BUY-only — fires only on naked shares
    // (matched portion stays for MERGE alpha).
    pub exit_enabled: bool,
    /// Sell when leg's ask ≥ vwap × this multiplier. 1.75 default = 75%
    /// profit target.
    pub exit_vwap_multiplier: f64,
    /// Absolute floor — only SELL when target_price ≥ this (e.g. 0.50).
    /// Prevents tiny-target sells like buying 0.05 → selling 0.10.
    pub exit_min_price: f64,
    /// Minimum naked shares to trigger a SELL.
    pub min_sell_shares: f64,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            cumulative_target_max: 0.95,
            cumulative_aim: 0.90,
            quote_shares: 6.0,
            min_order_usdc: 1.0,
            min_order_shares: 5.0,
            ladder_depth: 4,
            ladder_tick_cents: 1,
            completion_mult: 2.0,
            dip_trigger_cents: 3,
            dip_scale_mult: 3.0,
            enable_completion_taker: false,
            collapse_drop_cents: 8,
            collapse_window_s: 60,
            collapse_lookback_s: 600,
            collapse_breakout_cents: 5,
            sweep_hold_s: 120,
            sweep_target_max: 1.00,
            sweep_taker_slippage_cents: 3,
            sweep_size_mult: 5.0,
            max_naked_shares: 12.0,
            mid_min: 0.05,
            bail_loser_threshold: 0.05,
            bail_enabled: true,
            stale_maker_max_age_s: 4 * 3600,
            stale_maker_dist_cents: 10,
            merge_enabled: true,
            min_merge_shares: 5.0,
            merge_buffer_shares: 1.0,
            // 2026-05-25 (C): more aggressive — fire at matched ≥ 5 not 10.
            // Smaller pairs realized faster keeps capital turnover high; even
            // small marginal pairs (vwap ~0.95) free up ~$0.05 per pair.
            merge_eager_min_shares: 5.0,
            step3_suppress_in_play: true,
            bail_asymmetric_ratio: 0.5,
            bail_asymmetric_threshold: 0.10,
            emergency_close_enabled: true,
            // 2026-05-25 (E): 5 min → 30 min. Pre-match dip imbalances often
            // appear an hour out; 5 min was too late. 30 min catches Rinderknech-
            // Rodionov-class cases comfortably without sweeping good positions.
            emergency_close_secs: 1800,
            emergency_close_slippage_cents: 2,
            // 2026-05-25 (A): Step 2 + Sweep in-play guards.
            step2_in_play_min_bid: 0.30,
            // 2026-05-25 (D): Polymarket-implied loser threshold.
            dead_leg_prob_threshold: 0.20,
            // 2026-05-25 (B): strong asymmetric BAIL.
            bail_strong_ratio: 2.0,
            // 2026-05-25: tightened 0.50 → 0.30. With 0.50, Svitolina-Bondar
            // (Bondar 12 shares @ vwap 0.25, market bid 0.33) was triggering
            // BAIL even though Bondar wasn't losing. 0.30 = "clearly losing".
            bail_strong_threshold: 0.30,
            exit_enabled: true,
            // 2026-05-26: tuned 1.75 → 1.5, 0.50 → 0.40 after live observation
            // (885 churn on Shimizu market — target 0.89 was above realistic
            // market depth, 0 fills all day). Lower target = higher fill
            // probability at the cost of margin per trade.
            exit_vwap_multiplier: 1.5,
            exit_min_price: 0.40,
            min_sell_shares: 5.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode { Idle, Accumulate, Complete, Sweep, Bail }

impl PlanMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanMode::Idle => "idle",
            PlanMode::Accumulate => "accumulate",
            PlanMode::Complete => "complete",
            PlanMode::Sweep => "sweep",
            PlanMode::Bail => "bail",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind { Maker, Taker }

impl OrderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OrderKind::Maker => "maker",
            OrderKind::Taker => "taker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanTag { NewPair, Completion, Sweep, Exit }

impl PlanTag {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanTag::NewPair => "new_pair",
            PlanTag::Completion => "completion",
            PlanTag::Sweep => "sweep",
            PlanTag::Exit => "exit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side { Buy, Sell }
impl Side {
    pub fn as_str(self) -> &'static str {
        match self { Side::Buy => "BUY", Side::Sell => "SELL" }
    }
}

#[derive(Debug, Clone)]
pub struct DesiredOrder {
    pub leg: u8,
    /// `Side::Buy` for all RN1 entries (new_pair, completion, sweep). `Sell`
    /// only for `PlanTag::Exit` orders that liquidate naked holdings at
    /// a peak (S2 time-arb layer, 2026-05-25).
    pub side: Side,
    pub price: f64,
    pub size_shares: f64,
    pub kind: OrderKind,
    pub tag: PlanTag,
}

/// A maker bid OR sell offer currently resting on the CLOB (or paper book).
#[derive(Debug, Clone)]
pub struct RestingOrder {
    pub qid: i64,
    pub leg: u8,
    /// `Side::Buy` (RN1 ladder/completion/sweep) vs `Side::Sell` (S2 exit).
    /// Step 1 sweep, Step 2 completion, and cap_breach must NOT cancel SELL
    /// orders — they are exit makers and shouldn't be churned by the BUY
    /// flow. 2026-05-25 incident: 885 cancel-replace churns on a single SELL.
    pub side: Side,
    pub price: f64,
    pub remaining_shares: f64,
    pub kind: OrderKind,
    /// Seconds since the order was signed. Used by stale-maker auto-cancel.
    pub age_s: i64,
}

/// The planner's complete view of one market for one decision tick.
#[derive(Debug, Clone)]
pub struct PairState {
    pub condition_id: String,
    pub token_id: [String; 2],
    pub game_started: bool,
    /// Past `endgame_after_min` minutes since game_start — relax completion to
    /// the hard cap. The driver decides this from market metadata.
    pub endgame: bool,
    /// Seconds until `game_start`. Negative if already started, `None` if the
    /// market has no scheduled start time. Used by the pre-match emergency
    /// close rule (Step 1.5) to sweep single-leg exposure before in-play.
    pub secs_to_game_start: Option<i64>,
    /// Polymarket's settlement/probability snapshot for each leg, e.g.
    /// `[0.965, 0.035]`. Populated from `markets.outcome_prices` even pre-
    /// settlement (Polymarket updates this in real-time). Used to mark a
    /// leg as "dead": when `outcome_prob[i] ≤ dead_leg_threshold`, the leg
    /// is heavily losing and Completion/Sweep must NOT pile into it
    /// (anchor-leg trap prevention).
    pub outcome_prob: Option<[f64; 2]>,
    pub q: [f64; 2],
    pub vwap: [f64; 2],
    pub bid: [f64; 2],
    pub ask: [f64; 2],
    pub collapse: [bool; 2],
    pub resting: Vec<RestingOrder>,
}

impl PairState {
    pub fn matched(&self) -> f64 { self.q[0].min(self.q[1]) }
    pub fn naked_qty(&self) -> f64 { (self.q[0] - self.q[1]).abs() }
    pub fn ahead_leg(&self) -> Option<u8> {
        if self.q[0] > self.q[1] + 1e-6 { Some(0) }
        else if self.q[1] > self.q[0] + 1e-6 { Some(1) }
        else { None }
    }
    pub fn lagging_leg(&self) -> Option<u8> { self.ahead_leg().map(|a| 1 - a) }
    pub fn event_cost(&self) -> f64 {
        self.q[0] * self.vwap[0] + self.q[1] * self.vwap[1]
    }
    pub fn open_notional(&self) -> f64 {
        self.resting.iter().map(|r| r.remaining_shares * r.price).sum()
    }
    pub fn matched_vwap(&self) -> f64 {
        if self.matched() <= 0.0 { 0.0 } else { self.vwap[0] + self.vwap[1] }
    }
    fn resting_shares_on(&self, leg: u8) -> f64 {
        // BUY-side only — completion-bid sizing uses this to subtract
        // already-resting buy quantity. SELL exits are unrelated.
        self.resting.iter()
            .filter(|r| r.leg == leg && matches!(r.side, Side::Buy))
            .map(|r| r.remaining_shares).sum()
    }
}

#[derive(Debug, Clone)]
pub struct Affordability {
    pub event_headroom: f64,
    pub wallet_headroom: f64,
    pub kill_switch: bool,
}
impl Affordability {
    pub fn cap(&self) -> f64 {
        self.event_headroom.min(self.wallet_headroom).max(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct MergeAction {
    /// Pairs to merge (subtracted from both legs equally).
    pub shares: f64,
    /// Snapshot of `vwap[0] + vwap[1]` at merge time — used downstream to
    /// compute `realized_pnl = shares × (1.0 − matched_vwap)`.
    pub matched_vwap: f64,
}

#[derive(Debug, Clone)]
pub struct MarketPlan {
    pub add: Vec<DesiredOrder>,
    pub cancel: Vec<i64>,
    pub merges: Vec<MergeAction>,
    pub mode: PlanMode,
}

impl MarketPlan {
    fn idle() -> Self { Self { add: vec![], cancel: vec![], merges: vec![], mode: PlanMode::Idle } }
}

fn round_cent(p: f64) -> f64 { (p * 100.0).round() / 100.0 }
fn round_sh(s: f64) -> f64 { (s * 100.0).round() / 100.0 }

/// A resting order on `leg` at `price` is cap-breaching when, **assuming it
/// fully fills**, the projected weighted matched_vwap (using POST-fill leg
/// vwap on `leg` + current vwap on `other`) would exceed `target`.
///
/// History:
///   v1: `price + vwap[other] > target` (single-leg compare). Caused 27,768
///       churn cancels in the 2026-05-24 run because Step 3 placement uses
///       weighted-avg (allows rungs that pull leg vwap DOWN) while cap_breach
///       was checking the rung's solo price (always high vs other's full
///       vwap) → place / cancel infinite loop.
///   v2 (this version): match Step 3's weighted-vwap rule exactly. A rung
///       only cancels if its inclusion would breach target ON AVERAGE — so
///       deeply-dipped rungs that drag the leg's vwap below target are kept.
fn cap_breach(ps: &PairState, r: &RestingOrder, target: f64) -> bool {
    // SELL exits reduce position; they never push matched_vwap up. Skip.
    if matches!(r.side, Side::Sell) { return false; }
    let other = (1 - r.leg) as usize;
    if ps.q[other] <= 0.0 { return false; }
    let leg = r.leg as usize;
    let new_q = ps.q[leg] + r.remaining_shares;
    let new_vwap = if new_q > 0.0 {
        (ps.q[leg] * ps.vwap[leg] + r.remaining_shares * r.price) / new_q
    } else {
        r.price
    };
    round_cent(new_vwap + ps.vwap[other]) > target + 1e-9
}

/// Returns the trapped-leg index when one of two failure modes is active:
///
/// **Case A — single-leg trap (single-leg loser):**
/// Only one leg has shares AND its market bid is below `bail_loser_threshold`
/// (≤ 5¢ default). The OTHER side's completion bid lives at `aim − vwap_held`,
/// which never reaches the live market → no fill possible. Cancel all resting.
///
/// **Case B — asymmetric trap (over-loaded loser side):**
/// Both legs have shares but the **ahead** leg (more shares) has collapsed
/// past `bail_asymmetric_threshold` (≤ 10¢ default) AND naked > matched ×
/// `bail_asymmetric_ratio` (0.5 default = naked is half of matched). This is
/// the Kecmano-Marozsa / Ruiz-Tirante failure: dip-ladder accumulated 70+
/// shares of the loser side. Stop digging deeper.
fn loser_leg_trap(ps: &PairState, cfg: &PlannerConfig) -> Option<u8> {
    if !cfg.bail_enabled { return None; }
    // Case A: single-leg trap.
    let single_held: Option<u8> = if ps.q[0] > 0.0 && ps.q[1] <= 0.0 {
        Some(0)
    } else if ps.q[1] > 0.0 && ps.q[0] <= 0.0 {
        Some(1)
    } else {
        None
    };
    if let Some(h) = single_held {
        if ps.bid[h as usize] > 0.0 && ps.bid[h as usize] <= cfg.bail_loser_threshold {
            return Some(h);
        }
    }
    // Case B: asymmetric trap (over-loaded loser leg + collapsed bid).
    if let Some(ahead) = ps.ahead_leg() {
        let ai = ahead as usize;
        let naked = ps.naked_qty();
        let matched = ps.matched();
        // Require matched > 0 so we don't double-fire with single-leg case.
        if matched > 0.0
            && naked > matched * cfg.bail_asymmetric_ratio
            && ps.bid[ai] > 0.0
            && ps.bid[ai] <= cfg.bail_asymmetric_threshold
        {
            return Some(ahead);
        }
        // Case C (2026-05-25 Busta-Lehecka): STRONG asymmetric — naked vastly
        // exceeds matched. Fires even at mid bids (≤ 0.50) because once
        // imbalance hits 2:1+, the loser side's downside dominates.
        if matched > 0.0
            && naked > matched * cfg.bail_strong_ratio
            && ps.bid[ai] > 0.0
            && ps.bid[ai] <= cfg.bail_strong_threshold
        {
            return Some(ahead);
        }
    }
    None
}

/// Polymarket-implied dead leg: when settlement probability falls below
/// `dead_leg_prob_threshold` (default 20%), the leg is structurally losing
/// and we must NOT pile more shares onto it via Completion or Sweep.
/// Returns true if leg `c` is dead per outcome_prob (or false if no data).
fn is_dead_leg(ps: &PairState, cfg: &PlannerConfig, c: usize) -> bool {
    match ps.outcome_prob {
        Some(p) => p[c] <= cfg.dead_leg_prob_threshold,
        None => false,
    }
}

/// Maker BUY that has aged past `stale_maker_max_age_s` AND whose price is
/// more than `stale_maker_dist_cents` from the leg's current market bid.
/// Such an order is parked too far from the book to ever fill — cancel.
///
/// SELL exits get their own freshness handling (exit_planner re-emits per
/// tick if target still valid). This guard only applies to BUY makers.
fn stale_maker(r: &RestingOrder, ps: &PairState, cfg: &PlannerConfig) -> bool {
    if matches!(r.side, Side::Sell) { return false; }
    if !matches!(r.kind, OrderKind::Maker) { return false; }
    if cfg.stale_maker_max_age_s <= 0 { return false; }
    if r.age_s < cfg.stale_maker_max_age_s { return false; }
    let dist_thr = (cfg.stale_maker_dist_cents.max(0) as f64) / 100.0;
    let mkt_bid = ps.bid[r.leg as usize];
    if mkt_bid <= 0.0 { return false; }
    (mkt_bid - r.price).abs() > dist_thr + 1e-9
}

/// Core decision function. Pure — no IO.
pub fn plan_market(cfg: &PlannerConfig, ps: &PairState, afford: &Affordability) -> MarketPlan {
    let target = cfg.cumulative_target_max;
    let aim = cfg.cumulative_aim;
    let base = if cfg.quote_shares > 0.0 { cfg.quote_shares } else { 6.0 };
    let tick = (cfg.ladder_tick_cents.max(1) as f64) / 100.0;
    let afford_cap = afford.cap();

    // ── Step 0 — guards ─────────────────────────────────────────────────
    if afford.kill_switch {
        // Cancel BUY-side only — leave SELL exits intact for liquidation.
        return MarketPlan {
            add: vec![],
            cancel: ps.resting.iter()
                .filter(|r| matches!(r.side, Side::Buy))
                .map(|r| r.qid).collect(),
            merges: vec![],
            mode: PlanMode::Idle,
        };
    }
    if ps.bid[0] <= 0.0 || ps.bid[1] <= 0.0 {
        return MarketPlan::idle();
    }

    // BAIL — over-loaded loser side OR single-leg trap. Stops further BUY
    // accumulation but allows the S2 SELL exit layer to still fire (price
    // rebound on naked side = chance to unwind without buying more).
    let bail_active = loser_leg_trap(ps, cfg).is_some();
    if bail_active {
        // Cancel BUY-side resting only. Existing SELL exit makers stay (they
        // are our liquidation path — must not churn them).
        let mut bail_cancel: Vec<i64> = ps.resting.iter()
            .filter(|r| matches!(r.side, Side::Buy))
            .map(|r| r.qid).collect();
        // Try S2 exits even during bail — they help liquidate the trap.
        let mut bail_add: Vec<DesiredOrder> = vec![];
        if cfg.exit_enabled {
            let matched_eff = ps.q[0].min(ps.q[1]);
            for leg in 0..2usize {
                let leg_q = ps.q[leg];
                if leg_q <= matched_eff { continue; }
                let naked_in_leg = leg_q - matched_eff;
                if naked_in_leg < cfg.min_sell_shares { continue; }
                if ps.vwap[leg] <= 0.0 { continue; }
                let target = round_cent(
                    (ps.vwap[leg] * cfg.exit_vwap_multiplier).max(cfg.exit_min_price));
                if ps.ask[leg] < target - 1e-9 { continue; }
                // Skip if SELL already resting at this price level.
                let already = ps.resting.iter().any(|r|
                    matches!(r.side, Side::Sell)
                    && r.leg as usize == leg
                    && (r.price * 100.0).round() as i64 == (target * 100.0).round() as i64);
                if already { continue; }
                let size = round_sh(naked_in_leg);
                if size < cfg.min_sell_shares { continue; }
                bail_add.push(DesiredOrder {
                    leg: leg as u8, side: Side::Sell,
                    price: target, size_shares: size,
                    kind: OrderKind::Maker, tag: PlanTag::Exit,
                });
            }
        }
        return MarketPlan {
            add: bail_add,
            cancel: bail_cancel,
            merges: vec![],
            mode: PlanMode::Bail,
        };
    }

    let mut cancel: Vec<i64> = ps.resting.iter()
        .filter(|r| cap_breach(ps, r, target) || stale_maker(r, ps, cfg))
        .map(|r| r.qid)
        .collect();

    let can_afford = afford_cap >= cfg.min_order_usdc;

    // ── Step 1 — SWEEP (collapse) ───────────────────────────────────────
    if ps.game_started {
        for c in 0usize..2 {
            if !ps.collapse[c] { continue; }
            let o = 1 - c;
            // Only sweep if buying c reduces naked AND c is the cheap side.
            if ps.q[c] > ps.q[o] + 1e-6 { continue; }
            if ps.bid[c] > ps.bid[o] + 1e-6 { continue; }
            // 2026-05-25 (D): don't sweep into a Polymarket-implied dead leg.
            // Lehecka-class trap: leg keeps "collapsing" because it's losing.
            // Buying more is throwing money away.
            if is_dead_leg(ps, cfg, c) { continue; }
            // Cancel BUY-side resting on OTHER leg — a fill there would
            // outrun the sweep. SELL exits are unrelated to BUY flow → keep.
            for r in &ps.resting {
                if r.leg as usize == o
                    && matches!(r.side, Side::Buy)
                    && !cancel.contains(&r.qid)
                {
                    cancel.push(r.qid);
                }
            }
            if !can_afford {
                return MarketPlan { add: vec![], cancel, merges: vec![], mode: PlanMode::Sweep };
            }
            let other_ref = if ps.q[o] > 0.0 { ps.vwap[o] } else { ps.bid[o] };
            let price_cap = round_cent(cfg.sweep_target_max - other_ref);
            let slip = (cfg.sweep_taker_slippage_cents.max(0) as f64) / 100.0;
            let limit = round_cent((ps.ask[c] + slip).min(price_cap));
            if limit < ps.bid[c] - 1e-9 || limit <= 0.0 {
                return MarketPlan { add: vec![], cancel, merges: vec![], mode: PlanMode::Sweep };
            }
            let to_match = (ps.q[o] - ps.q[c]).max(0.0);
            let mut size = (base * cfg.sweep_size_mult).min(to_match + cfg.max_naked_shares);
            size = size.min(afford_cap / limit);
            size = round_sh(size);
            if size >= cfg.min_order_shares && size * limit >= cfg.min_order_usdc {
                return MarketPlan {
                    add: vec![DesiredOrder { leg: c as u8, side: Side::Buy, price: limit, size_shares: size,
                                             kind: OrderKind::Taker, tag: PlanTag::Sweep }],
                    cancel,
                    merges: vec![],
                    mode: PlanMode::Sweep,
                };
            }
            return MarketPlan { add: vec![], cancel, merges: vec![], mode: PlanMode::Sweep };
        }
    }

    // ── Step 1.5 — PRE-MATCH EMERGENCY CLOSE ────────────────────────────
    // Single-leg naked + game_start imminent → taker-sweep the lagging leg
    // at ask to pair up BEFORE in-play. Without this, the bot enters live
    // matches with directional exposure (Rinderknech-Rodionov pattern:
    // ladder filled one side pre-match, the other never caught the dip).
    // Sweep cost = $0.03/share taker fee — small price vs. anchor-leg-trap
    // tail loss seen in the 2026-05-24 run (-$80 / 9 markets).
    if cfg.emergency_close_enabled
        && !ps.game_started
        && ps.secs_to_game_start.map(|s| s > 0 && s <= cfg.emergency_close_secs).unwrap_or(false)
        && ps.naked_qty() >= cfg.min_order_shares
    {
        if let (Some(_ahead), Some(lag)) = (ps.ahead_leg(), ps.lagging_leg()) {
            let lag_idx = lag as usize;
            let need = ps.naked_qty() - ps.resting_shares_on(lag);
            if ps.ask[lag_idx] > 0.0 && need >= cfg.min_order_shares && afford_cap > 0.0 {
                let slip = (cfg.emergency_close_slippage_cents.max(0) as f64) / 100.0;
                let limit = round_cent(ps.ask[lag_idx] + slip);
                let mut size = need.min(afford_cap / limit.max(0.01));
                size = round_sh(size);
                if size >= cfg.min_order_shares && size * limit >= cfg.min_order_usdc {
                    // Cancel BUY-side resting on lag leg only (sweep replaces
                    // them at ask). SELL exits on lag leg are unrelated.
                    for r in &ps.resting {
                        if r.leg == lag
                            && matches!(r.side, Side::Buy)
                            && !cancel.contains(&r.qid)
                        {
                            cancel.push(r.qid);
                        }
                    }
                    return MarketPlan {
                        add: vec![DesiredOrder {
                            leg: lag, side: Side::Buy, price: limit, size_shares: size,
                            kind: OrderKind::Taker, tag: PlanTag::Sweep,
                        }],
                        cancel,
                        merges: vec![],
                        mode: PlanMode::Sweep,
                    };
                }
            }
        }
    }

    let mut add: Vec<DesiredOrder> = vec![];
    let mut remaining = afford_cap;
    let mut completion_active = false;

    // ── Step 2 — COMPLETION (naked → matched) ───────────────────────────
    // Does NOT early-return any more; Step 3 also runs in the same tick so
    // fresh dip-pairs can be acquired while we wait for the completion bid.
    //
    // 2026-05-25 in-play guards (Busta-Lehecka post-mortem):
    //   - dead leg (outcome_prob ≤ threshold) → skip (loser side stop-loss)
    //   - in-play AND bid[lag] < step2_in_play_min_bid → skip
    if ps.naked_qty() >= cfg.min_order_shares {
        if let (Some(ahead), Some(lag)) = (ps.ahead_leg(), ps.lagging_leg()) {
            let lag_idx = lag as usize;
            let lag_is_dead = is_dead_leg(ps, cfg, lag_idx);
            let lag_in_play_too_low = ps.game_started
                && ps.bid[lag_idx] < cfg.step2_in_play_min_bid;
            // (2026-05-27) Removed: "cancel ALL ahead-leg BUYs as housekeeping".
            // It collided with Step 3's per-tier paired ladder which legitimately
            // places dip-bids on the ahead leg too (when pair-sum ≤ aim). The
            // result was a place→cancel→place oscillation every 4s (Fokina-Tirante
            // & Fonseca-Prizmic, 2026-05-27 07:04+). Real housekeeping (stale or
            // cap-breach-y orders) is already covered by the cap_breach + stale_maker
            // filter earlier in this function (lines 572-575).
            if !lag_is_dead && !lag_in_play_too_low {
                let ahead_ref = if ps.q[ahead as usize] > 0.0 {
                    ps.vwap[ahead as usize]
                } else {
                    ps.bid[ahead as usize]
                };
                let complete_cap = if ps.endgame { target } else { aim };
                let price_cap = round_cent(complete_cap - ahead_ref);
                // Completion taker (when enabled): must cross the ASK to fill,
                // not sit at the BID. Use ask + slippage as the limit. If even
                // (ask + slip) exceeds price_cap (i.e., the spread is too wide
                // for the cap), skip — set lag_price=0.0 which falls through
                // the mid_min guard. Without this, planner emits taker orders
                // at bid-based prices that never cross in real CLOB while
                // paper-mode (formerly) recorded them as fills at the limit —
                // an enormous fake-PnL bug (Smith-Svajda 100×fake, 2026-05-27).
                let lag_price = if cfg.enable_completion_taker {
                    let slip = (cfg.sweep_taker_slippage_cents.max(0) as f64) / 100.0;
                    let candidate = round_cent(ps.ask[lag_idx] + slip);
                    if candidate > price_cap + 1e-9 { 0.0 } else { candidate }
                } else {
                    round_cent(ps.bid[lag_idx].min(price_cap))
                };
                if can_afford && lag_price >= cfg.mid_min {
                    let need = ps.naked_qty() - ps.resting_shares_on(lag);
                    if need >= cfg.min_order_shares {
                        let kind = if cfg.enable_completion_taker {
                            OrderKind::Taker
                        } else {
                            OrderKind::Maker
                        };
                        let mut size = (base * cfg.completion_mult).min(need);
                        size = size.max(cfg.min_order_usdc / lag_price.max(0.01));
                        size = size.min(remaining / lag_price);
                        size = round_sh(size);
                        if size >= cfg.min_order_shares && size * lag_price >= cfg.min_order_usdc {
                            // Weighted-vwap post-fill check (matches cap_breach
                            // formula exactly). Without this, ENDGAME mode prices
                            // up to (target − vwap[ahead]) ignoring existing lag
                            // shares — the resulting order gets cancelled by
                            // cap_breach next tick, causing endless place/cancel
                            // churn (Kopriva-Moutet 28-order, 240+ cancel pattern,
                            // 2026-05-26). Skip placement if post-fill weighted
                            // vwap would breach target.
                            let new_q_lag = ps.q[lag_idx] + size;
                            let new_vwap_lag = if new_q_lag > 0.0 {
                                (ps.q[lag_idx] * ps.vwap[lag_idx] + size * lag_price) / new_q_lag
                            } else {
                                lag_price
                            };
                            let would_breach = ps.q[ahead as usize] > 0.0
                                && round_cent(new_vwap_lag + ps.vwap[ahead as usize])
                                    > target + 1e-9;
                            if !would_breach {
                                add.push(DesiredOrder { leg: lag, side: Side::Buy,
                                                        price: lag_price, size_shares: size,
                                                        kind, tag: PlanTag::Completion });
                                remaining -= size * lag_price;
                                completion_active = true;
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Step 2.5 — MERGE recycle (handoff §6.5) ─────────────────────────
    // Recycle matched pairs into USDC at $1/pair (paper-mode CTF
    // mergePositions simulation). Always net-positive because matched_vwap
    // ≤ aim < $1 by construction.
    //
    // Two triggers (OR):
    //   (a) **cap pressure** — `remaining < min_order_usdc * 2` (legacy)
    //   (b) **eager** — `matched_pairs ≥ merge_eager_min_shares` (default 10)
    // The eager trigger frees capital BEFORE cap pressure builds up, so the
    // ladder doesn't stall and matched_vwap doesn't drift toward target.
    // Previous build: 3 merges total in 26h, $4.28 realized. Eager: ~one
    // merge per 10 matched pairs as the bot accumulates → continuous recycle.
    let mut merges: Vec<MergeAction> = vec![];
    let mut eff_q = ps.q;
    let matched_pairs = ps.matched();
    let cap_pressure = remaining < cfg.min_order_usdc * 2.0;
    let eager_fire   = matched_pairs >= cfg.merge_eager_min_shares;
    if cfg.merge_enabled
        && matched_pairs >= cfg.min_merge_shares
        && (cap_pressure || eager_fire)
    {
        let shares_to_merge = round_sh(matched_pairs - cfg.merge_buffer_shares);
        if shares_to_merge >= cfg.min_merge_shares {
            let mvwap = round_cent(ps.vwap[0] + ps.vwap[1]);
            if mvwap > 0.0 && mvwap < 1.0 + 1e-9 {
                merges.push(MergeAction { shares: shares_to_merge, matched_vwap: mvwap });
                eff_q[0] -= shares_to_merge;
                eff_q[1] -= shares_to_merge;
                remaining += shares_to_merge * mvwap;
            }
        }
    }

    // ── Step 3 — DIP-LADDER ─────────────────────────────────────────────
    // Per-rung guard: COMBINED post-fill matched_vwap ≤ target_max blocks
    // anchor-leg trap rungs that would push average above ceiling.
    //
    // **In-play suppression (2026-05-25)**: when game has started, the
    // dip-ladder systematically loads the loser side because the dropping
    // price attracts fills while the rising side never reaches the maker.
    // 26h paper run lost ~$80 across 9 markets through this. Pre-match only.
    //
    // ── Step 2.6 — S2 EXIT LAYER (time-arb on naked legs) ───────────────
    // For each leg holding NAKED shares (q[leg] > matched), if its market ASK
    // has rebounded ≥ vwap × exit_vwap_multiplier (default 1.75), place a
    // SELL maker at the target price. Captures intra-match momentum WITHOUT
    // touching the matched portion (which still gets MERGE'd → $1/pair).
    //
    // Backtest (3-week ATP/WTA, 2026-05-25): 80% win rate, +22.5% avg/trade.
    // Uses eff_q (post-MERGE q) so we don't try to sell what was just merged.
    if cfg.exit_enabled {
        let matched_eff = eff_q[0].min(eff_q[1]);
        for leg in 0..2usize {
            let leg_q = eff_q[leg];
            let leg_vwap = ps.vwap[leg];
            if leg_q <= matched_eff { continue; }            // not naked
            let naked_in_leg = leg_q - matched_eff;
            if naked_in_leg < cfg.min_sell_shares { continue; }
            if leg_vwap <= 0.0 { continue; }
            // Target = max(vwap × multiplier, exit_min_price). Floor protects
            // against tiny-target sells (e.g. bought 0.05 → would sell 0.09).
            let target = round_cent((leg_vwap * cfg.exit_vwap_multiplier).max(cfg.exit_min_price));
            // Only place if market ASK is at or above target (otherwise no
            // one will buy from us at this price yet).
            if ps.ask[leg] < target - 1e-9 { continue; }
            // Skip if we already have a SELL resting at this price level.
            let already = ps.resting.iter().any(|r|
                r.leg as usize == leg
                && (r.price * 100.0).round() as i64 == (target * 100.0).round() as i64);
            if already { continue; }
            let size = round_sh(naked_in_leg);
            if size < cfg.min_sell_shares { continue; }
            add.push(DesiredOrder {
                leg: leg as u8, side: Side::Sell,
                price: target, size_shares: size,
                kind: OrderKind::Maker, tag: PlanTag::Exit,
            });
        }
    }

    // ── Step 3 — PER-TIER PAIRED LADDER (playbook spec, 2026-05-27 revert)
    // Reverted from the temporary RN1-style independent ladder — playbook
    // RN1_TENNIS_PLAYBOOK.md is authoritative (9d 398 events, every rule
    // data-backed). Per-tier paired with cap ≤ aim (0.90), fixed quote_shares
    // sizing, depth 5. The "wide independent ladder" experiment placed rungs
    // at combined = 1.00 which the 9d cVWAP→ROI bucket shows is the −12.6%
    // ROI zone — exactly what the spec is designed to avoid.
    //
    // Mechanic per playbook §4: top rung at best_bid (median 0¢ offset),
    // tiers −1¢..−4¢ below. Each tier checks pair-sum ≤ aim AND post-fill
    // matched_vwap ≤ target (would_match_vwap_with_rung_q).
    let in_play_block = cfg.step3_suppress_in_play && ps.game_started;
    if !in_play_block && remaining >= cfg.min_order_usdc {
        let depth = cfg.ladder_depth.max(1);
        let occupied = |leg: u8, cents: i64, add: &[DesiredOrder]| -> bool {
            ps.resting.iter().any(|r|
                matches!(r.side, Side::Buy)
                && r.leg == leg
                && (r.price * 100.0).round() as i64 == cents)
            || add.iter().any(|d| d.leg == leg && (d.price * 100.0).round() as i64 == cents
                                   && matches!(d.tag, PlanTag::NewPair))
        };
        let leg_new_rung_total = |leg: u8, add: &[DesiredOrder]| -> usize {
            ps.resting.iter()
                .filter(|r| matches!(r.side, Side::Buy) && r.leg == leg).count()
            + add.iter().filter(|d| d.leg == leg && matches!(d.tag, PlanTag::NewPair)).count()
        };
        let dip = (cfg.dip_trigger_cents.max(0) as f64) / 100.0;
        let max_tiers = depth * 10 + 10;
        for t in 0..max_tiers {
            if leg_new_rung_total(0, &add) >= depth && leg_new_rung_total(1, &add) >= depth { break; }
            let p0 = round_cent(ps.bid[0] - (t as f64) * tick);
            let p1 = round_cent(ps.bid[1] - (t as f64) * tick);
            if p0 < cfg.mid_min || p1 < cfg.mid_min { break; }
            // Per-tier paired cap: pair_sum ≤ aim (RN1 dip-vwap-aim discipline).
            if round_cent(p0 + p1) > aim + 1e-9 { continue; }
            let dipped = (eff_q[0] > 0.0 && ps.vwap[0] - p0 >= dip)
                      || (eff_q[1] > 0.0 && ps.vwap[1] - p1 >= dip);
            let mut size = if dipped { base * cfg.dip_scale_mult } else { base };
            size = size.max(cfg.min_order_usdc / p0.min(p1).max(0.01));
            size = round_sh(size);
            if size < cfg.min_order_shares { break; }
            // Matched-vwap check: post-fill combined vwap ≤ target (= hard cap).
            if !would_match_vwap_with_rung_q(eff_q, ps.vwap, p0, p1, size, target) {
                continue;
            }
            let pair_cost = size * p0 + size * p1;
            if pair_cost > remaining { break; }
            let c0 = (p0 * 100.0).round() as i64;
            let c1 = (p1 * 100.0).round() as i64;
            let mut placed_any = false;
            if leg_new_rung_total(0, &add) < depth && !occupied(0, c0, &add) {
                add.push(DesiredOrder { leg: 0, side: Side::Buy, price: p0, size_shares: size,
                                        kind: OrderKind::Maker, tag: PlanTag::NewPair });
                placed_any = true;
            }
            if leg_new_rung_total(1, &add) < depth && !occupied(1, c1, &add) {
                add.push(DesiredOrder { leg: 1, side: Side::Buy, price: p1, size_shares: size,
                                        kind: OrderKind::Maker, tag: PlanTag::NewPair });
                placed_any = true;
            }
            if placed_any { remaining -= pair_cost; }
        }
    }

    let mode = if add.is_empty() && cancel.is_empty() {
        PlanMode::Idle
    } else if completion_active {
        PlanMode::Complete
    } else {
        PlanMode::Accumulate
    };
    MarketPlan { add, cancel, merges, mode }
}

/// Forecast the matched_vwap if a new rung at (p0, p1, size) gets filled.
/// Returns true if it stays ≤ target ceiling. We compare against TARGET
/// (the hard cap) rather than aim because:
///   - aim is the new-rung pair-sum gate (already enforced upstream)
///   - matched_vwap rolls together ALL fills on each leg, so a single
///     deeply-dipped rung can drag the average down toward safety
///   - we only forbid the rung if its inclusion would breach the absolute
///     ceiling; otherwise dip-accumulation is permitted
fn would_match_vwap_with_rung_q(
    q: [f64; 2], vwap: [f64; 2], p0: f64, p1: f64, size: f64, target: f64,
) -> bool {
    let post_q0 = q[0] + size;
    let post_q1 = q[1] + size;
    if post_q0 <= 0.0 || post_q1 <= 0.0 { return true; }
    let post_vwap0 = (q[0] * vwap[0] + size * p0) / post_q0;
    let post_vwap1 = (q[1] * vwap[1] + size * p1) / post_q1;
    round_cent(post_vwap0 + post_vwap1) <= target + 1e-9
}

// ── Collapse detector ───────────────────────────────────────────────────

/// Per-leg rolling bid history → collapse flags. Stateful; owned by the driver.
pub struct CollapseDetector {
    hist: [VecDeque<(Instant, f64)>; 2],
    last_trigger: [Option<Instant>; 2],
}

impl CollapseDetector {
    pub fn new() -> Self {
        Self { hist: [VecDeque::new(), VecDeque::new()], last_trigger: [None, None] }
    }

    pub fn update(&mut self, cfg: &PlannerConfig, bid: [f64; 2]) -> [bool; 2] {
        let now = Instant::now();
        let lookback = Duration::from_secs(cfg.collapse_lookback_s.max(1) as u64);
        let window = Duration::from_secs(cfg.collapse_window_s.max(1) as u64);
        let hold = Duration::from_secs(cfg.sweep_hold_s.max(1) as u64);
        let drop_thr = (cfg.collapse_drop_cents.max(1) as f64) / 100.0;
        let breakout = (cfg.collapse_breakout_cents.max(0) as f64) / 100.0;
        let mut out = [false, false];
        for leg in 0..2 {
            if bid[leg] > 0.0 {
                self.hist[leg].push_back((now, bid[leg]));
            }
            while let Some(&(t, _)) = self.hist[leg].front() {
                if now.duration_since(t) > lookback { self.hist[leg].pop_front(); } else { break; }
            }
            let cur = bid[leg];
            let recent_max = self.hist[leg].iter()
                .filter(|(t, _)| now.duration_since(*t) <= window)
                .map(|(_, b)| *b)
                .fold(0.0f64, f64::max);
            let trig_a = cur > 0.0 && recent_max - cur >= drop_thr;
            let prior_min = self.hist[leg].iter()
                .filter(|(t, _)| now.duration_since(*t) > window)
                .map(|(_, b)| *b)
                .fold(f64::INFINITY, f64::min);
            let trig_b = cur > 0.0 && prior_min.is_finite() && cur + breakout < prior_min;

            if trig_a || trig_b {
                self.last_trigger[leg] = Some(now);
            }
            out[leg] = self.last_trigger[leg]
                .map(|t| now.duration_since(t) <= hold)
                .unwrap_or(false);
        }
        out
    }
}

impl Default for CollapseDetector {
    fn default() -> Self { Self::new() }
}
