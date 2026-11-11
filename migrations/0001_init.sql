-- rn1bot v0.1 schema — markets / fills / positions / orders + M0.5 book_events
-- 모든 timestamp 는 timestamptz UTC.

-- ==========================================================
-- markets — Gamma 캐시 + 시장 메타
-- ==========================================================
CREATE TABLE IF NOT EXISTS markets (
  condition_id              CHAR(66)     PRIMARY KEY,
  slug                      TEXT         NOT NULL,
  event_slug                TEXT,
  question                  TEXT,
  outcomes                  JSONB        NOT NULL,
  outcome_prices            JSONB,
  clob_token_ids            JSONB        NOT NULL,
  fee_schedule              JSONB,
  neg_risk                  BOOLEAN      NOT NULL DEFAULT FALSE,
  closed                    BOOLEAN      NOT NULL DEFAULT FALSE,
  active                    BOOLEAN      NOT NULL DEFAULT FALSE,
  end_date                  TIMESTAMPTZ,
  game_start                TIMESTAMPTZ,
  accepting_orders          TIMESTAMPTZ,
  uma_status                TEXT,
  last_trade_price          DOUBLE PRECISION,
  best_bid                  DOUBLE PRECISION,
  best_ask                  DOUBLE PRECISION,
  volume_num                DOUBLE PRECISION,
  sports_market_type        TEXT,
  order_min_size            DOUBLE PRECISION,
  order_price_min_tick_size DOUBLE PRECISION,
  rewards_max_spread        DOUBLE PRECISION,
  icon                      TEXT,
  refreshed_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS markets_event_slug_idx   ON markets(event_slug);
CREATE INDEX IF NOT EXISTS markets_closed_idx       ON markets(closed, active);
CREATE INDEX IF NOT EXISTS markets_game_start_idx   ON markets(game_start);
CREATE INDEX IF NOT EXISTS markets_neg_risk_idx     ON markets(neg_risk);

-- Asset(clob_token_id) -> condition mapping (lookup 가속)
CREATE TABLE IF NOT EXISTS asset_to_condition (
  asset_id      TEXT     PRIMARY KEY,
  condition_id  CHAR(66) NOT NULL REFERENCES markets(condition_id) ON DELETE CASCADE,
  outcome_index SMALLINT NOT NULL,
  outcome_name  TEXT
);
CREATE INDEX IF NOT EXISTS atc_cid_idx ON asset_to_condition(condition_id);

-- ==========================================================
-- fills — 자체 + 외부(RN1) 관찰 체결
-- ==========================================================
CREATE TABLE IF NOT EXISTS fills (
  fill_id          BIGSERIAL    PRIMARY KEY,
  tx_hash          CHAR(66),
  condition_id     CHAR(66)     NOT NULL,
  outcome_index    SMALLINT     NOT NULL,
  outcome_name     TEXT,
  side             TEXT         NOT NULL CHECK (side IN ('BUY','SELL')),
  size             DOUBLE PRECISION NOT NULL,
  price            DOUBLE PRECISION NOT NULL,
  usdc_size        DOUBLE PRECISION NOT NULL,
  fee_paid         DOUBLE PRECISION NOT NULL DEFAULT 0,
  rebate_received  DOUBLE PRECISION NOT NULL DEFAULT 0,
  ts               TIMESTAMPTZ  NOT NULL,
  is_ours          BOOLEAN      NOT NULL DEFAULT FALSE,
  is_maker         BOOLEAN,
  title            TEXT,
  slug             TEXT,
  event_slug       TEXT,
  icon             TEXT,
  UNIQUE (tx_hash, condition_id, outcome_index, side, size, price, ts)
);
CREATE INDEX IF NOT EXISTS fills_cid_ts_idx     ON fills(condition_id, ts DESC);
CREATE INDEX IF NOT EXISTS fills_ts_desc_idx    ON fills(ts DESC);
CREATE INDEX IF NOT EXISTS fills_is_ours_ts_idx ON fills(is_ours, ts DESC);
CREATE INDEX IF NOT EXISTS fills_event_slug_idx ON fills(event_slug);

-- ==========================================================
-- positions — 현재 보유 snapshot (RN1 + 우리)
-- 우리/외부 구분은 별도 wallet 컬럼 X — 본 봇 인스턴스는 한 wallet 만 추적.
-- ==========================================================
CREATE TABLE IF NOT EXISTS positions (
  condition_id   CHAR(66)         NOT NULL,
  outcome_index  SMALLINT         NOT NULL,
  outcome_name   TEXT             NOT NULL,
  size           DOUBLE PRECISION NOT NULL,
  avg_price      DOUBLE PRECISION,
  current_price  DOUBLE PRECISION,
  current_value  DOUBLE PRECISION NOT NULL DEFAULT 0,
  initial_value  DOUBLE PRECISION NOT NULL DEFAULT 0,
  realized_pnl   DOUBLE PRECISION NOT NULL DEFAULT 0,
  cash_pnl       DOUBLE PRECISION NOT NULL DEFAULT 0,
  total_bought   DOUBLE PRECISION NOT NULL DEFAULT 0,
  mergeable      BOOLEAN          NOT NULL DEFAULT FALSE,
  redeemable     BOOLEAN          NOT NULL DEFAULT FALSE,
  title          TEXT,
  slug           TEXT,
  event_slug     TEXT,
  icon           TEXT,
  neg_risk       BOOLEAN          NOT NULL DEFAULT FALSE,
  end_date       TIMESTAMPTZ,
  updated_at     TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
  PRIMARY KEY (condition_id, outcome_index)
);
CREATE INDEX IF NOT EXISTS positions_event_slug_idx ON positions(event_slug);
CREATE INDEX IF NOT EXISTS positions_size_idx       ON positions(size) WHERE size > 0;
CREATE INDEX IF NOT EXISTS positions_redeemable_idx ON positions(redeemable) WHERE redeemable = true;

-- ==========================================================
-- orders — 우리 자체 (M4+)
-- ==========================================================
CREATE TABLE IF NOT EXISTS orders (
  order_id         BIGSERIAL    PRIMARY KEY,
  clob_order_hash  TEXT         UNIQUE,
  condition_id     CHAR(66)     NOT NULL,
  outcome_index    SMALLINT     NOT NULL,
  asset_id         TEXT         NOT NULL,
  side             TEXT         NOT NULL CHECK (side IN ('BUY','SELL')),
  price            DOUBLE PRECISION NOT NULL,
  size             DOUBLE PRECISION NOT NULL,
  signed_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
  posted_at        TIMESTAMPTZ,
  filled_at        TIMESTAMPTZ,
  cancelled_at     TIMESTAMPTZ,
  status           TEXT         NOT NULL CHECK (status IN ('signed','posted','resting','partially_filled','filled','cancelled','rejected','expired')),
  rejection_reason TEXT
);
CREATE INDEX IF NOT EXISTS orders_status_idx ON orders(status);
CREATE INDEX IF NOT EXISTS orders_cid_outcome_status_idx ON orders(condition_id, outcome_index, status);

-- ==========================================================
-- position_actions — MERGE / REDEEM tx 로그
-- ==========================================================
CREATE TABLE IF NOT EXISTS position_actions (
  action_id     BIGSERIAL    PRIMARY KEY,
  condition_id  CHAR(66)     NOT NULL,
  action_type   TEXT         NOT NULL CHECK (action_type IN ('MERGE','REDEEM','SPLIT')),
  size          DOUBLE PRECISION,
  tx_hash       CHAR(66),
  status        TEXT         NOT NULL CHECK (status IN ('pending','confirmed','failed')),
  realized_usdc DOUBLE PRECISION,
  triggered_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
  confirmed_at  TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS pa_cid_idx ON position_actions(condition_id);

-- ==========================================================
-- pnl_snapshots — 5min 주기 timeseries
-- ==========================================================
CREATE TABLE IF NOT EXISTS pnl_snapshots (
  ts            TIMESTAMPTZ      NOT NULL PRIMARY KEY,
  gross_buy     DOUBLE PRECISION,
  gross_sell    DOUBLE PRECISION,
  realized_pnl  DOUBLE PRECISION,
  unrealized    DOUBLE PRECISION,
  merge_locked  DOUBLE PRECISION,
  rebate        DOUBLE PRECISION,
  trade_count   INTEGER,
  active_orders INTEGER
);

-- ==========================================================
-- system_events / heartbeats / sync_cursors / config
-- ==========================================================
CREATE TABLE IF NOT EXISTS system_events (
  event_id  BIGSERIAL    PRIMARY KEY,
  ts        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
  level     TEXT         NOT NULL CHECK (level IN ('debug','info','warn','error','fatal')),
  component TEXT         NOT NULL,
  message   TEXT         NOT NULL,
  context   JSONB
);
CREATE INDEX IF NOT EXISTS se_ts_desc_idx ON system_events(ts DESC);
CREATE INDEX IF NOT EXISTS se_level_ts_idx ON system_events(level, ts DESC) WHERE level IN ('error','fatal');

CREATE TABLE IF NOT EXISTS system_heartbeats (
  component  TEXT         PRIMARY KEY,
  ts         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
  payload    JSONB
);

CREATE TABLE IF NOT EXISTS sync_cursors (
  key         TEXT         PRIMARY KEY,
  value_int   BIGINT,
  value_text  TEXT,
  updated_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS config (
  key        TEXT         PRIMARY KEY,
  value      JSONB        NOT NULL,
  updated_at TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
  updated_by TEXT
);

-- ==========================================================
-- M0.5: book_events — public market WS 시계열
-- ==========================================================
CREATE TABLE IF NOT EXISTS book_events (
  event_id      BIGSERIAL    PRIMARY KEY,
  ts            TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
  asset_id      TEXT         NOT NULL,
  condition_id  CHAR(66),
  event_type    TEXT         NOT NULL CHECK (event_type IN ('book','price_change','last_trade_price')),
  best_bid      DOUBLE PRECISION,
  best_ask      DOUBLE PRECISION,
  spread        DOUBLE PRECISION,
  payload       JSONB        NOT NULL
);
CREATE INDEX IF NOT EXISTS be_asset_ts_idx  ON book_events(asset_id, ts);
CREATE INDEX IF NOT EXISTS be_cid_ts_idx    ON book_events(condition_id, ts);
CREATE INDEX IF NOT EXISTS be_ts_desc_idx   ON book_events(ts DESC);

-- ==========================================================
-- view: rn1 portfolio (frontend 조회용)
-- positions + events 단위 group by
-- ==========================================================
CREATE OR REPLACE VIEW v_rn1_event_tree AS
SELECT
  p.event_slug,
  COUNT(DISTINCT p.condition_id) AS market_count,
  COUNT(*) AS leg_count,
  SUM(p.current_value) AS total_value,
  SUM(p.cash_pnl) AS total_cash_pnl,
  SUM(p.realized_pnl) AS total_realized_pnl,
  BOOL_OR(p.redeemable) AS any_redeemable,
  BOOL_OR(p.mergeable) AS any_mergeable,
  MAX(p.updated_at) AS last_updated,
  MIN(p.end_date) AS earliest_end,
  MAX(p.icon) AS sample_icon,
  MAX(p.title) AS sample_title
FROM positions p
WHERE p.event_slug IS NOT NULL AND (p.size > 0 OR p.redeemable OR p.mergeable)
GROUP BY p.event_slug;
