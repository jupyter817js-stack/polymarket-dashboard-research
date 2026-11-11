-- 0010_redemptions.sql — paper-mode settlement redemption simulation.
--
-- When a Polymarket market settles (closed=true + outcome_prices set),
-- each held share is redeemed for `outcome_prices[idx]` USDC. For paper
-- mode we record this as a synthetic redemption that:
--   - reduces net position to 0 (market_leg_history subtracts it)
--   - credits realized_pnl = shares × outcome_price − cost_basis
--   - drops the market from active_market_cids
--
-- Live mode will eventually call CTF redeemPositions() and reconcile
-- on-chain. For now (paper) this is bookkeeping only.

CREATE TABLE IF NOT EXISTS redemptions (
    id              BIGSERIAL PRIMARY KEY,
    condition_id    CHAR(66)        NOT NULL,
    outcome_index   SMALLINT        NOT NULL,
    shares          DOUBLE PRECISION NOT NULL,
    settle_price    DOUBLE PRECISION NOT NULL,
    usdc_received   DOUBLE PRECISION NOT NULL,
    cost_basis      DOUBLE PRECISION NOT NULL,
    realized_pnl    DOUBLE PRECISION NOT NULL,
    ts              TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    UNIQUE(condition_id, outcome_index)   -- one redemption per leg
);

CREATE INDEX IF NOT EXISTS redemptions_cid_idx ON redemptions(condition_id);
CREATE INDEX IF NOT EXISTS redemptions_ts_idx  ON redemptions(ts DESC);
