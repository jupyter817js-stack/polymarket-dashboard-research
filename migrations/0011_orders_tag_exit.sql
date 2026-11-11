-- 0011_orders_tag_exit.sql — allow 'exit' tag for S2 SELL maker orders.
-- The 2026-05-25 time-arb layer emits PlanTag::Exit when a naked leg's price
-- rebounds ≥ vwap × 1.75; trader.insert_order writes this tag.

ALTER TABLE orders DROP CONSTRAINT IF EXISTS orders_tag_check;
ALTER TABLE orders ADD CONSTRAINT orders_tag_check
  CHECK (tag = ANY (ARRAY['new_pair'::text, 'completion'::text,
                          'sweep'::text, 'reconcile'::text,
                          'exit'::text]));
