-- 0003: position_actions UNIQUE 제약 + book_events retention 인덱스
-- W1 (MERGE/REDEEM collector) 가 idempotent 하게 INSERT 하려면 dedupe key 필요.
-- W3 (book_events 7d 보존) 의 DELETE 가 효율적으로 돌게 ts 인덱스도 확인.

-- 같은 (tx_hash, condition_id, action_type) 의 중복 row 방지.
-- tx_hash 가 NULL 인 케이스도 있을 수 있어 partial unique index 로.
CREATE UNIQUE INDEX IF NOT EXISTS pa_unique_tx_action_idx
  ON position_actions (tx_hash, condition_id, action_type)
  WHERE tx_hash IS NOT NULL;

-- tx_hash 없는 (예: 일부 polymarket /activity 응답) 경우의 dedupe:
-- triggered_at 까지 포함하여 unique 보장.
CREATE UNIQUE INDEX IF NOT EXISTS pa_unique_notx_idx
  ON position_actions (condition_id, action_type, triggered_at, size)
  WHERE tx_hash IS NULL;

-- 활동 타입별 빠른 조회.
CREATE INDEX IF NOT EXISTS pa_type_triggered_idx
  ON position_actions (action_type, triggered_at DESC);

-- book_events 의 7-day retention 정리를 위해 ts 단독 인덱스 보강
-- (be_ts_desc_idx 가 이미 DESC 로 있으므로 OK, 추가 보장).
-- 본 마이그레이션에선 인덱스만 점검.

-- 점검 SQL:
--   SELECT relname, pg_size_pretty(pg_total_relation_size(oid))
--   FROM pg_class WHERE relname LIKE 'book_events%' OR relname LIKE 'position_actions%';
