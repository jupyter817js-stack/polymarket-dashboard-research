-- M1.0 Shadow runner — quote 계산 + 로그만 (실제 post 안함).
-- 후속 phase M1.1 부터는 동일 schema 에 actual order_id / clob_hash 연결.

CREATE TABLE IF NOT EXISTS shadow_quotes (
  quote_id          BIGSERIAL PRIMARY KEY,
  placed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  cancelled_at      TIMESTAMPTZ,
  filled_at         TIMESTAMPTZ,                  -- RN1 fill 이 우리 quote price 매칭 시
  condition_id      CHAR(66)  NOT NULL,
  outcome_index     SMALLINT  NOT NULL,
  outcome_name      TEXT,
  side              TEXT      NOT NULL DEFAULT 'BUY' CHECK (side = 'BUY'),
  -- 1¢ 단위 정수 — would-fill 매칭에 사용 (e.g., 45 = $0.45)
  price_cents       SMALLINT  NOT NULL,
  size_usdc         DOUBLE PRECISION NOT NULL,    -- $30 default
  is_first_leg      BOOLEAN   NOT NULL DEFAULT TRUE,
  -- market context at placement
  mid_at_place      DOUBLE PRECISION,
  spread_at_place   DOUBLE PRECISION,
  game_eta_sec      BIGINT,
  -- result
  fill_size         DOUBLE PRECISION NOT NULL DEFAULT 0,
  est_rebate_usdc   DOUBLE PRECISION NOT NULL DEFAULT 0,
  cancelled_reason  TEXT,
  -- meta (denormalized for fast query)
  market_title      TEXT,
  market_slug       TEXT,
  event_slug        TEXT,
  sport             TEXT
);

-- open quote 매칭 (cid, outcome, price) 위한 partial index
CREATE INDEX IF NOT EXISTS shadow_quotes_open_idx
  ON shadow_quotes (condition_id, outcome_index, price_cents)
  WHERE cancelled_at IS NULL AND filled_at IS NULL;

CREATE INDEX IF NOT EXISTS shadow_quotes_placed_at_idx
  ON shadow_quotes (placed_at DESC);

CREATE INDEX IF NOT EXISTS shadow_quotes_filled_idx
  ON shadow_quotes (filled_at DESC) WHERE filled_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS shadow_quotes_cid_idx
  ON shadow_quotes (condition_id);

-- KPI 일별 집계 view
CREATE OR REPLACE VIEW v_shadow_daily AS
SELECT
  date_trunc('day', placed_at)::date AS day,
  COUNT(*) AS quotes_placed,
  COUNT(*) FILTER (WHERE filled_at IS NOT NULL) AS quotes_filled,
  COUNT(*) FILTER (WHERE cancelled_at IS NOT NULL AND filled_at IS NULL) AS quotes_cancelled,
  COUNT(DISTINCT condition_id) AS markets_touched,
  SUM(fill_size)::float8 AS gross_filled_usdc,
  SUM(est_rebate_usdc)::float8 AS rebate_est_usdc,
  COUNT(*) FILTER (WHERE is_first_leg AND filled_at IS NOT NULL) AS first_leg_filled,
  COUNT(*) FILTER (WHERE NOT is_first_leg AND filled_at IS NOT NULL) AS second_leg_filled
FROM shadow_quotes
GROUP BY day
ORDER BY day DESC;
