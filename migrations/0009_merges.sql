-- 0009_merges.sql — paper-mode MERGE simulation.
--
-- Polymarket CTF (Conditional Token Framework) lets a holder of matched
-- outcome-0 + outcome-1 shares MERGE them back into USDC at $1/pair (no
-- slippage, gas only). RN1's only "sell": never trades a leg back to the
-- book — always recycles matched pairs via MERGE. See handoff §6.5.
--
-- This migration adds the paper-mode bookkeeping so the planner can model
-- capital recycling. Live CTF mergePositions() contract calls are deferred
-- to Phase B (Polygon signer + gas) — see handoff §11.
--
-- Each row represents one MERGE event:
--   shares           pairs merged (subtracted from BOTH legs)
--   usdc_recovered   = shares × $1.00 (always)
--   matched_vwap     leg[0].vwap + leg[1].vwap snapshot at merge time
--   realized_pnl     = shares × (1.0 − matched_vwap_at_merge)
--   ts               wall-clock timestamp

CREATE TABLE IF NOT EXISTS merges (
    id              BIGSERIAL PRIMARY KEY,
    condition_id    CHAR(66)        NOT NULL,
    shares          DOUBLE PRECISION NOT NULL,
    usdc_recovered  DOUBLE PRECISION NOT NULL,
    matched_vwap    DOUBLE PRECISION NOT NULL,
    realized_pnl    DOUBLE PRECISION NOT NULL,
    ts              TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS merges_cid_idx ON merges(condition_id);
CREATE INDEX IF NOT EXISTS merges_ts_idx  ON merges(ts DESC);
