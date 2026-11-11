-- pnl_snapshots 에 scope 컬럼 추가 — 'all' (전체 positions 합) vs
-- 'recent' (최근 24h activity 마켓 한정).
-- 동시점에 두 scope 행이 공존하므로 PK 를 (ts, scope) 로.

ALTER TABLE pnl_snapshots
  ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'all';

-- PK 갱신
ALTER TABLE pnl_snapshots
  DROP CONSTRAINT IF EXISTS pnl_snapshots_pkey;

ALTER TABLE pnl_snapshots
  ADD PRIMARY KEY (ts, scope);

CREATE INDEX IF NOT EXISTS pnl_snapshots_scope_ts_idx
  ON pnl_snapshots(scope, ts);
