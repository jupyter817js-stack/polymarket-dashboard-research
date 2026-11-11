//! Pre-trade risk — affordability + kill switch (RN1_LIVE_LOGIC_HANDOFF §4.2).
//!
//! REPLACES the old lifetime per-leg budget. Affordability is computed every
//! tick from reconciled position + wallet:
//!   event_headroom  = per_event_cap − filled_cost − open_notional
//!   wallet_headroom = min(free − buffer − open_all, daily_gross − filled_today − open_all)
//! Floored at 0. A manual trade simply shrinks headroom; a leg is never frozen.

use anyhow::Result;
use rn1_planner::{Affordability, PairState};
use sqlx::PgPool;

use crate::config::Config;
use crate::wallet::WalletSnapshot;

pub struct RiskGuard<'a> {
    pub cfg: &'a Config,
    pub pool: &'a PgPool,
}

impl<'a> RiskGuard<'a> {
    pub fn new(cfg: &'a Config, pool: &'a PgPool) -> Self { Self { cfg, pool } }

    pub async fn affordability(
        &self,
        ps: &PairState,
        wallet: &WalletSnapshot,
    ) -> Result<Affordability> {
        // Daily realized-loss kill switch — placeholder uses rebate as proxy;
        // real PnL needs MERGE/REDEEM/settlement tracking (handoff §6.5).
        let today = rn1_db::trader::today_summary(self.pool).await.unwrap_or_default();
        if today.realized_pnl < -self.cfg.daily_loss_cap_usdc {
            let _ = rn1_db::trader::set_kill_switch(self.pool, true).await;
        }
        let kill_switch = rn1_db::trader::kill_switch_active(self.pool).await.unwrap_or(false);

        let event_headroom =
            (self.cfg.per_event_cap_usdc - ps.event_cost() - ps.open_notional()).max(0.0);

        let open_all = rn1_db::trader::open_unfilled_notional(self.pool).await.unwrap_or(0.0);
        let filled_today = rn1_db::trader::today_filled_notional(self.pool).await.unwrap_or(0.0);
        let by_daily_gross = self.cfg.daily_gross_cap_usdc - (filled_today + open_all);

        let wallet_headroom = if self.cfg.mode.is_live() {
            let by_wallet = wallet.clob_usdc_available - self.cfg.min_usdc_buffer - open_all;
            by_wallet.min(by_daily_gross).max(0.0)
        } else {
            by_daily_gross.max(0.0)
        };

        Ok(Affordability { event_headroom, wallet_headroom, kill_switch })
    }
}
