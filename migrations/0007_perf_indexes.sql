-- Performance indexes — fills table lookups that drove /api/portfolio/positions
-- and /api/portfolio/summary up to 600-700 ms p99.
--
-- 1) (condition_id, outcome_index, is_ours, ts DESC) covers the LATERAL JOIN
--    in /api/portfolio/positions for last_fill_price / first_fill_ts per leg.
-- 2) is_ours-false 24h window scan with condition_id projection — used by
--    /api/portfolio/summary scope=recent subquery AND the positions handler's
--    recent filter.

CREATE INDEX IF NOT EXISTS fills_cid_outcome_ours_ts_idx
    ON fills (condition_id, outcome_index, is_ours, ts DESC);

-- INCLUDE makes the DISTINCT condition_id sub-query an index-only scan.
CREATE INDEX IF NOT EXISTS fills_ours_ts_cid_idx
    ON fills (is_ours, ts DESC) INCLUDE (condition_id)
    WHERE is_ours = false;

-- Bot-only (is_ours=true) leg lookup for /api/live/positions GROUP BY.
CREATE INDEX IF NOT EXISTS fills_ours_true_cid_outcome_ts_idx
    ON fills (condition_id, outcome_index, ts DESC)
    WHERE is_ours = true;

-- ANALYZE so planner sees the new indexes immediately.
ANALYZE fills;
