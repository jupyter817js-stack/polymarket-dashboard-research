-- Per-asset best_bid/best_ask cache — outcome 별 정확한 현재가가 필요.
-- markets 테이블의 best_bid/best_ask 는 한 row 당 하나 (= outcome 0 기준) 라
-- 양 outcome 모두 같은 가격이 표시되는 버그가 났다. asset_to_condition 은
-- 이미 asset_id 별 row 가 있으니 거기에 quote 컬럼을 붙인다.

ALTER TABLE asset_to_condition
  ADD COLUMN IF NOT EXISTS best_bid    DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS best_ask    DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS last_trade  DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS quoted_at   TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS atc_quoted_at_idx ON asset_to_condition(quoted_at DESC)
  WHERE quoted_at IS NOT NULL;
