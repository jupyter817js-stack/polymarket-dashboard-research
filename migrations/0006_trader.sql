-- Trader (apps/trader) — orders gain plan-tag, kind, size_usdc fills & a
-- token_id back-reference; we also add a small per-day trader-state table for
-- realized PnL, gross-deployed, kill-switch (see RN1_LIVE_LOGIC_HANDOFF §4.2).

ALTER TABLE orders
  ADD COLUMN IF NOT EXISTS token_id          TEXT,
  ADD COLUMN IF NOT EXISTS size_usdc         DOUBLE PRECISION NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS fill_size_shares  DOUBLE PRECISION NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS fill_usdc         DOUBLE PRECISION NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS rebate_usdc       DOUBLE PRECISION NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS mid_at_place      DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS kind              TEXT NOT NULL DEFAULT 'maker'
    CHECK (kind IN ('maker','taker','reconcile')),
  ADD COLUMN IF NOT EXISTS tag               TEXT NOT NULL DEFAULT 'new_pair'
    CHECK (tag IN ('new_pair','completion','sweep','reconcile')),
  ADD COLUMN IF NOT EXISTS cancelled_reason  TEXT;

-- Trader status (status='resting' is the post-confirmed maker-on-book state).
-- We rely on the 0001_init.sql CHECK to permit it; nothing more to add there.

CREATE INDEX IF NOT EXISTS orders_clob_hash_idx     ON orders(clob_order_hash) WHERE clob_order_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS orders_cid_outcome_open_idx ON orders(condition_id, outcome_index)
  WHERE status IN ('signed','posted','resting','partially_filled');

-- Daily trader aggregate — realized PnL / gross deployed / fills counter /
-- kill-switch flag. One row per UTC date. Used by the affordability gate.
CREATE TABLE IF NOT EXISTS trader_daily (
  date            DATE        PRIMARY KEY,
  gross_filled    DOUBLE PRECISION NOT NULL DEFAULT 0,
  realized_pnl    DOUBLE PRECISION NOT NULL DEFAULT 0,
  rebate          DOUBLE PRECISION NOT NULL DEFAULT 0,
  fills_count     INTEGER     NOT NULL DEFAULT 0,
  kill_switch     BOOLEAN     NOT NULL DEFAULT FALSE,
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Bot status / strategy snapshot — single-row table the trader writes to and
-- the api reads. tick / wsmarket / wsuser / candidates / last_cycle_ms / mode.
CREATE TABLE IF NOT EXISTS trader_status (
  id                  SMALLINT     PRIMARY KEY DEFAULT 1
                                   CHECK (id = 1),
  mode                TEXT         NOT NULL DEFAULT 'paper',
  running             BOOLEAN      NOT NULL DEFAULT FALSE,
  focus_slug          TEXT,
  current_tick        BIGINT       NOT NULL DEFAULT 0,
  last_cycle_at       TIMESTAMPTZ,
  last_cycle_ms       BIGINT       NOT NULL DEFAULT 0,
  candidates          INTEGER      NOT NULL DEFAULT 0,
  ws_market_connected BOOLEAN      NOT NULL DEFAULT FALSE,
  ws_user_connected   BOOLEAN      NOT NULL DEFAULT FALSE,
  kill_switch         BOOLEAN      NOT NULL DEFAULT FALSE,
  last_error          TEXT,
  wallet              JSONB,
  config_snapshot     JSONB,
  strategy            JSONB,
  updated_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

INSERT INTO trader_status (id) VALUES (1) ON CONFLICT (id) DO NOTHING;
