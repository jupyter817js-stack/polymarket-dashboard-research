/** Backend REST clients. Server-side 와 client-side 둘 다에서 호출 가능. */

const BASE = typeof window === "undefined"
  ? process.env.RN1_API_BASE ?? "http://localhost:8000"
  : "";  // browser 는 next rewrite 사용

async function get<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(`${BASE}${path}`, { cache: "no-store", ...init });
  if (!r.ok) throw new Error(`${path} HTTP ${r.status}`);
  return r.json() as Promise<T>;
}

// ===== shared types =====
export interface PositionEntry {
  condition_id: string; outcome_index: number; outcome_name: string;
  size: number; avg_price: number | null; current_price: number | null;
  current_value: number; initial_value: number;
  realized_pnl: number; cash_pnl: number; total_bought: number;
  mergeable: boolean; redeemable: boolean;
  title: string | null; slug: string | null; event_slug: string | null;
  icon: string | null; neg_risk: boolean;
  end_date: string | null; updated_at: string;
  // 마지막 관측 fill — 토스트의 실제 fill 가격 표시 (current_price 는 시장 mid 라 inaccurate)
  last_fill_price: number | null;
  last_fill_ts:    string | null;
  // 첫 fill ts — 양면 마켓에서 어느 outcome 이 먼저 시작됐는지 (FIRST leg)
  first_fill_ts:   string | null;
  /** Seconds since `current_price` was last refreshed by the trader's quote
   *  cache. Display warning if > ~60s — 2026-05-25 incident, stale cache
   *  showed false +profit. */
  quote_age_s:     number | null;
}
export interface MarketNode {
  condition_id: string; title: string | null; slug: string | null;
  neg_risk: boolean; end_date: string | null;
  current_value: number; cash_pnl: number; realized_pnl: number;
  outcomes: PositionEntry[];
  // 같은 event 안에서 market family 식별
  category: "moneyline" | "ou-totals" | "spread" | "first-inning" | "alt-spread" | "other" | string;
  family_param: string | null;
}
export interface EventNode {
  event_slug: string;
  sample_title: string | null;
  sample_icon: string | null;
  market_count: number; leg_count: number;
  total_value: number; total_cash_pnl: number; total_realized_pnl: number;
  any_redeemable: boolean; any_mergeable: boolean;
  earliest_end: string | null; last_updated: string;
  /** Most-recent fill ts across all legs in this event. Distinct from
   *  `last_updated` (which advances on every quote refresh). Used by the
   *  "최근 체결 ↓" sort. May be null if no fills yet. */
  latest_fill_ts: string | null;
  markets: MarketNode[];
}
export interface PortfolioTree {
  events: EventNode[];
  orphans: PositionEntry[];
  total_value: number;
  total_cash_pnl: number;
  total_realized_pnl: number;
  event_count: number;
  page?: number;
  page_size?: number;
  total_pages?: number;
  filter?: string;
  sort?: string;
  q?: string | null;
}

export type PositionsSort =
  | "recent" | "oldest"
  | "fill"   | "fill_asc"
  | "value" | "value_asc"
  | "pnl" | "pnl_asc"
  | "realized"
  | "legs" | "markets" | "name"
  | "end" | "end_desc";

export interface PositionsQuery {
  sort?: PositionsSort;
  filter?: "recent" | "active" | "redeemable" | "mergeable" | "all";
  q?: string;
  page?: number;
  page_size?: number;
}

export type HistorySort = "newest" | "oldest" | "size" | "usdc" | "price" | "low_price";
export interface HistoryQuery {
  sort?: HistorySort;
  side?: "ALL" | "BUY" | "SELL";
  q?: string;
  page?: number;
  page_size?: number;
}
export interface HistoryResp {
  fills: FillEntry[];
  page: number;
  page_size: number;
  total_count: number;
  total_pages: number;
  sort: string;
  side: string;
  q: string | null;
}

export interface PortfolioSummary {
  wallet: string;
  alias: string;
  scope: string;
  positions_value: number;
  position_legs: number;
  biggest_win: number | null;
  biggest_win_title: string | null;
  biggest_win_slug: string | null;
  biggest_win_event: string | null;
  predictions: number;
  predictions_24h: number;
  gross_24h: number;
  joined_at: string;
  now: string;
}

export interface PnlPoint {
  ts: string;
  realized: number;
  unrealized: number;
  net: number;
  gross_buy: number;
  value: number; // = net (main series)
}
export interface PnlSeries {
  tf: string;
  scope?: string;
  interval: string;
  bucket: string;
  metric?: string;
  source?: string;
  point_count?: number;
  points: PnlPoint[];
  start_net?: number;
  end_net?: number;
  delta_net?: number;
  // 호환 (구 필드명)
  start_cum: number;
  end_cum: number;
  delta_cum: number;
}

export interface MarketDetail {
  condition_id: string;
  title: string | null;
  event_slug: string | null;
  slug: string | null;
  icon: string | null;
  neg_risk: boolean;
  uma_status: string | null;
  game_start: string | null;
  game_eta_sec: number | null;
  game_state: "upcoming" | "live" | "post" | "unknown";
  end_date: string | null;
  best_bid: number | null;
  best_ask: number | null;
  mid: number | null;
  spread: number | null;
  spread_bps: number | null;
  last_trade_price: number | null;
  volume_num: number | null;
  order_min_size: number | null;
  tick_size: number | null;
  rewards_max_spread: number | null;
  rn1_current_value: number;
  rn1_total_size: number;
  rn1_avg_entry: number | null;
  slippage_pct: number | null;
  fills_24h: number;
  gross_24h: number;
  avg_fill_size: number | null;
  price_min_24h: number | null;
  price_max_24h: number | null;
  price_range_24h: number | null;
  last_fill_ts: string | null;
}

export type MarketsDetailSort =
  | "activity" | "fills_asc"
  | "spread" | "value" | "volume"
  | "game" | "slippage_abs" | "price_range";

export interface MarketsDetailQuery {
  sort?: MarketsDetailSort;
  state?: "all" | "upcoming" | "live" | "post";
  q?: string;
  page?: number;
  page_size?: number;
}

export interface MarketsDetailResp {
  now: string;
  market_count: number;
  page: number;
  page_size: number;
  total_pages: number;
  sort: string;
  state_filter: string;
  q: string | null;
  markets: MarketDetail[];
}

export interface FillEntry {
  ts: string;
  condition_id: string; outcome_index: number; outcome_name: string | null;
  side: string; size: number; price: number; usdc_size: number;
  title: string | null; event_slug: string | null; icon: string | null;
  tx_hash: string | null; is_ours: boolean;
}

export interface PriceLevel {
  outcome_index: number;
  outcome_name: string;
  side: "BUY" | "SELL" | string;
  price_cents: number;
  price: number;
  fill_count: number;
  total_size: number;
  total_usdc: number;
  first_ts: string;
  last_ts: string;
}
export interface BreakdownFill {
  ts: string;
  outcome_index: number;
  outcome_name: string | null;
  side: string;
  size: number;
  price: number;
  usdc_size: number;
  tx_hash: string | null;
}
export interface BreakdownOutcome {
  outcome_index: number;
  outcome_name: string;
  size: number;
  avg_price: number | null;
  current_price: number | null;
  current_value: number;
  total_bought: number;
  cash_pnl: number;
  realized_pnl: number;
  redeemable: boolean;
  mergeable: boolean;
}
export interface PositionBreakdown {
  condition_id: string;
  outcomes: BreakdownOutcome[];
  price_levels: PriceLevel[];
  fills: BreakdownFill[];
}

export interface AnalysisResp {
  wallet: string; alias: string;
  scope?: string;
  scope_label?: string;
  live: {
    position_legs: number; current_value: number;
    cash_pnl: number; realized_pnl: number;
    fills_observed: number; gross_notional_observed: number; buy_count: number;
  };
  categories: { category: string; fills: number; notional: number }[];
  forensic_v0_1: Record<string, number | string>;
  strategy_signature: {
    entry: string; exit: string;
    alpha_sources: string[]; non_used: string; universe: string;
    timing_p50: { entry_lag_minutes: number; last_trade_minus_gameStart_min: number };
  };
}

export interface Heartbeat {
  component: string; ts: string; payload: Record<string, unknown> | null;
}

// ===== Maker behavior 분석 =====
export type EntryPhase = "pre-game-1h+" | "pre-game" | "live-1h" | "live-3h" | "post-game" | "no-meta";
export interface MakerMarket {
  condition_id: string;
  title: string | null;
  event_slug: string | null;
  icon: string | null;
  uma_status: string | null;
  game_start: string | null;
  game_eta_sec: number | null;
  end_date: string | null;
  first_fill: string;
  last_fill: string;
  fills: number;
  gross_usdc: number;
  shares: number;
  avg_price: number;
  min_price: number;
  max_price: number;
  price_range: number;
  price_stddev: number;
  avg_fill_size: number;
  avg_fill_usdc: number;
  active_minutes: number;
  inter_fill_sec: number;
  sides_traded: number;
  two_sided: boolean;
  first_vs_game_min: number | null;
  last_vs_game_min: number | null;
  entry_phase: EntryPhase;
}
export type MakerSort = "gross" | "fills" | "entry" | "spread" | "range" | "update";
export interface MakerBehaviorQuery {
  hours?: number;
  sort?: MakerSort;
  page?: number;
  page_size?: number;
}
export interface MakerBehaviorResp {
  window_hours: number;
  now: string;
  total_markets: number;
  page: number;
  page_size: number;
  total_pages: number;
  sort: string;
  markets: MakerMarket[];
  aggregate: {
    two_sided_pct: number;
    avg_inter_fill_sec: number;
    avg_price_stddev: number;
    avg_price_range: number;
    median_entry_min_vs_game: number | null;
    phase_counts: Record<string, number>;
  };
}

// ===== MERGE / REDEEM / SPLIT actions =====
export interface ActionRow {
  action_id: number;
  condition_id: string;
  action_type: "MERGE" | "REDEEM" | "SPLIT";
  size: number | null;
  tx_hash: string | null;
  realized_usdc: number | null;
  triggered_at: string;
  status: string;
  title: string | null;
  event_slug: string | null;
  outcome_name: string | null;
}
export interface ActionsRecentResp {
  actions: ActionRow[];
  summary: {
    merge_count: number; redeem_count: number; split_count: number;
    merge_usdc: number; redeem_usdc: number; split_usdc: number;
  };
}

// ===== Mid-price tracking =====
export interface MidPriceResp {
  window_hours: number;
  sample: number;
  now: string;
  total_fills_scanned: number;
  fills_with_book: number;
  fills_no_book: number;
  classification: {
    maker_bid: number; taker_ask: number; inside_spread: number; outside: number;
    maker_pct: number; taker_pct: number; inside_pct: number;
  };
  gross_usdc: {
    maker: number; taker: number; inside: number; total_with_book: number;
  };
  avg_distance_from_mid: number;
  estimated_rebate_usdc: number;
  rebate_assumption_bps: number;
  distance_distribution: { cents_from_mid: number; count: number }[];
  top_markets_by_gross: {
    condition_id: string; title: string | null; fills: number;
    maker_fills: number; maker_pct: number; gross_usdc: number;
  }[];
}

// ===== Per-sport breakdown =====
export interface SportRow {
  sport: string;
  markets: number;
  fills: number;
  gross_usdc: number;
  avg_gross_per_market: number;
  avg_fills_per_market: number;
  avg_active_min: number;
  avg_inter_fill_sec: number;
  avg_price_stddev: number;
  avg_price_range: number;
  avg_price_mean: number;
  two_sided_count: number;
  two_sided_pct: number;
  avg_first_vs_game_min: number | null;
  markets_with_meta: number;
}
export interface PerSportResp {
  window_hours: number;
  now: string;
  total_markets: number;
  total_gross_usdc: number;
  sports: SportRow[];
}

// ===== Lifecycle patterns aggregate =====
export interface LifecyclePatternsResp {
  window_hours: number;
  total_markets: number;
  first_leg_distribution: {
    outcome_0: number;
    outcome_1: number;
    single_leg_only: number;
    two_sided: number;
    two_sided_pct: number;
  };
  first_leg_classification: Record<string, number>;  // favorite / underdog / even
  time_to_second_buckets: { label: string; count: number }[];
  balance_buckets: { label: string; count: number }[];
  stats: {
    median_time_to_second_min: number | null;
    median_balance_ratio: number | null;
    n_with_t2s: number;
    n_with_balance: number;
  };
  scatter: { cid: string; title: string | null; t2s_min: number; balance: number; gross: number }[];
  representatives: {
    condition_id: string; title: string | null; event_slug: string | null;
    first_leg: number | null; first_leg_price: number | null;
    time_to_second_min: number | null; balance_ratio: number | null;
    total_usdc: number; total_fills: number; sides_traded: number;
  }[];
}

// ===== First-leg context =====
export interface FirstLegContextResp {
  window_hours: number;
  total_markets: number;
  first_leg_side_selection: {
    cheaper_side: number; expensive_side: number; equal: number; single_leg_only: number;
    cheaper_pct: number;
  };
  first_leg_price_distribution: { label: string; count: number }[];
  second_leg_gap_median_min: number | null;
  markets: {
    condition_id: string; title: string | null; event_slug: string | null; icon: string | null;
    first_leg: number; other_leg: number | null;
    first_ts: string; other_ts: string | null;
    self_first_price: number; other_first_price: number | null;
    implied_sum: number | null;
    gap_min: number | null;
    first_leg_cheaper: boolean | null;
    secs_from_game_min: number | null;
  }[];
}

// ===== Quote timeline (single market book × fills) =====
export interface QuoteTimelineResp {
  condition_id: string;
  assets: { asset_id: string; outcome_index: number }[];
  book_events: {
    ts: string;
    outcome_index: number;
    event_type: string;
    best_bid: number | null;
    best_ask: number | null;
  }[];
  fills: {
    ts: string;
    outcome_index: number;
    price: number;
    size: number;
    usdc_size: number;
  }[];
  book_event_count: number;
  fill_count: number;
}

// ===== Market lifecycle replay (single market 마이크로 분석) =====
export interface LifecycleFill {
  ts: string;
  secs_from_game_min: number | null;
  outcome_index: number;
  outcome_name: string | null;
  side: string;
  price: number;
  size: number;
  usdc_size: number;
  tx_hash: string | null;
  best_bid: number | null;
  best_ask: number | null;
  mid: number | null;
  dist_from_mid: number | null;
  classification: "maker" | "taker" | "inside" | "no-book";
  cum_shares_outcome: number;
  cum_usdc_outcome: number;
  cum_shares_0: number;
  cum_shares_1: number;
  balance_ratio: number;
}
export interface LifecycleLeg {
  outcome_index: number;
  fills: number;
  shares: number;
  usdc: number;
  avg_price: number;
  first_ts: string;
}
export interface LifecycleResp {
  meta: {
    condition_id: string;
    question: string | null;
    slug: string;
    event_slug: string | null;
    outcomes: string[] | null;
    icon: string | null;
    game_start: string | null;
    end_date: string | null;
    uma_status: string | null;
    best_bid: number | null;
    best_ask: number | null;
    volume_num: number | null;
    neg_risk: boolean;
  };
  fills: LifecycleFill[];
  legs: LifecycleLeg[];
  total_fills: number;
  active_minutes: number | null;
  lifecycle: {
    first_leg: number | null;
    time_to_second_leg_min: number | null;
    two_sided: boolean;
    final_balance_ratio: number | null;
  };
}

// ===== Realized P&L verification =====
export interface RealizedPnlDay {
  day: string;
  merge_payout: number;
  merge_n: number;
  redeem_payout: number;
  redeem_n: number;
  split_payout: number;
  split_n: number;
  gross_buy: number;
  n_fills: number;
  rebate_est: number;
  day_net_est: number;
}
export interface RealizedPnlResp {
  window_days: number;
  observed_days: number;
  actions_days: number;
  fills_days: number;
  now: string;
  daily: RealizedPnlDay[];
  totals: {
    merge_payout: number; merge_count: number;
    redeem_payout: number; redeem_count: number;
    split_payout: number; split_count: number;
    all_payouts: number;
    gross_buy: number;
    n_fills: number;
    rebate_estimate: number;
    current_holdings_mtm: number;
    merge_locked_est: number;
    merge_margin_est_pct: number;
  };
  daily_averages: {
    merge_payout: number;
    redeem_payout: number;
    all_payouts: number;
    rebate: number;
    gross_buy: number;
    projected_net: number;
  };
  forensic_baseline: {
    window_days: number;
    merge_locked: number;
    residual_pnl: number;
    maker_rebate: number;
    trade_pnl: number;
    net: number;
    daily_avg: number;
    gross_notional: number;
  };
  verification: {
    gross_buy_vs_baseline_pct: number;
    rebate_vs_baseline_pct: number;
    projected_net_vs_baseline_pct: number;
    rebate_assumption_bps: number;
    maker_share_assumption_pct: number;
    merge_margin_assumption_pct: number;
    caveat: string;
  };
}

// ===== Coverage diagnostics =====
export interface GapMarket {
  condition_id: string;
  outcome_index: number;
  outcome_name: string;
  title: string | null;
  event_slug: string | null;
  api_total_bought: number;
  observed_shares: number;
  observed_fills: number;
  gap_shares: number;
  coverage_pct: number;
  first_observed: string | null;
  last_observed: string | null;
}
export interface CoverageResp {
  now: string;
  total_positions: number;
  covered_markets: number;
  zero_observed: number;
  overall_coverage_pct: number;
  total_api_bought: number;
  total_observed_shares: number;
  gap_markets: GapMarket[];
}

// ===== fetchers =====
function qs(p: Record<string, string | number | undefined>): string {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(p)) {
    if (v !== undefined && v !== "" && v !== null) u.set(k, String(v));
  }
  const s = u.toString();
  return s ? `?${s}` : "";
}

export const api = {
  summary: (scope: "recent" | "all" = "recent") => get<PortfolioSummary>(`/api/portfolio/summary?scope=${scope}`),
  pnlSeries: (tf = "1D", scope: "recent" | "all" = "recent") =>
    get<PnlSeries>(`/api/portfolio/pnl-series?tf=${tf}&scope=${scope}`),
  marketsDetail: (opts: MarketsDetailQuery = {}) => get<MarketsDetailResp>(
    "/api/portfolio/markets-detail" + qs(opts as Record<string, string | number | undefined>)
  ),
  positions: (opts: PositionsQuery = {}) => get<PortfolioTree>(
    "/api/portfolio/positions" + qs({ view: "tree", ...opts })
  ),
  positionsFlat: () => get<{ positions: PositionEntry[] }>("/api/portfolio/positions?view=flat"),
  positionBreakdown: (cid: string) => get<PositionBreakdown>(`/api/portfolio/positions/${cid}/breakdown`),
  openOrders: () => get<{ orders: unknown[]; note?: string }>("/api/portfolio/orders"),
  history: (opts: HistoryQuery = {}) => get<HistoryResp>(
    "/api/portfolio/history" + qs(opts as Record<string, string | number | undefined>)
  ),
  analysis: (scope: "recent" | "all" = "recent") => get<AnalysisResp>(`/api/portfolio/analysis?scope=${scope}`),
  makerBehavior: (opts: MakerBehaviorQuery = {}) => get<MakerBehaviorResp>(
    "/api/analysis/maker-behavior" + qs(opts as Record<string, string | number | undefined>)
  ),
  actionsRecent: () => get<ActionsRecentResp>("/api/analysis/actions-recent"),
  midPrice:      (hours = 24, sample = 5000) =>
    get<MidPriceResp>(`/api/analysis/mid-price-tracking?hours=${hours}&sample=${sample}`),
  perSport:      (hours = 24) => get<PerSportResp>(`/api/analysis/per-sport?hours=${hours}`),
  marketLifecycle: (cid: string) => get<LifecycleResp>(`/api/analysis/market-lifecycle/${cid}`),
  lifecyclePatterns: (hours = 24) => get<LifecyclePatternsResp>(`/api/analysis/lifecycle-patterns?hours=${hours}`),
  firstLegContext:   (hours = 24) => get<FirstLegContextResp>(`/api/analysis/first-leg-context?hours=${hours}`),
  quoteTimeline:     (cid: string) => get<QuoteTimelineResp>(`/api/analysis/quote-timeline/${cid}`),
  realizedPnl:       (days = 30) => get<RealizedPnlResp>(`/api/analysis/realized-pnl?days=${days}`),
  coverage:      () => get<CoverageResp>("/api/diagnostics/coverage"),
  liveSummary: () => get<TraderState>("/api/live/summary"),
  traderState:  () => get<TraderState>("/api/live/state"),
  traderOpenOrders:    () => get<TraderOpenOrdersResp>("/api/live/orders"),
  traderClosedOrders:  () => get<TraderClosedOrdersResp>("/api/live/orders/recent"),
  livePositions:       (opts: PositionsQuery = {}) => get<PortfolioTree>(
    "/api/live/positions" + qs({ view: "tree", ...opts })
  ),
  livePositionBreakdown: (cid: string) => get<PositionBreakdown>(
    `/api/live/positions/${cid}/breakdown`
  ),
  livePortfolioSummary: (scope: "recent" | "all" = "recent") =>
    get<LivePortfolioSummary>(`/api/live/portfolio-summary?scope=${scope}`),
  livePnlSeries:        (tf: string = "1D", scope: "recent" | "all" = "recent") =>
    get<PnlSeries>(`/api/live/pnl-series?tf=${tf}&scope=${scope}`),
  liveMarketsUpcoming:  (opts: { universe?: string; hours_back?: number; hours_ahead?: number; min_vol?: number } = {}) =>
    get<LiveUpcomingMarketsResp>("/api/live/markets-upcoming" + qs(opts as Record<string, string | number | undefined>)),
  systemStatus: () => get<{ heartbeats: Heartbeat[]; now: string }>("/api/system/status"),
};

// ===== Live upcoming markets (ATP/WTA schedule) =====
export interface LiveUpcomingMarket {
  condition_id: string;
  slug: string;
  event_slug: string | null;
  question: string | null;
  neg_risk: boolean;
  game_start: string | null;
  end_date: string | null;
  secs_to_start: number | null;
  game_state: "upcoming" | "live" | "post" | "unknown" | string;
  volume_num: number | null;
  best_bid: number | null;
  best_ask: number | null;
  mid: number | null;
  spread: number | null;
  last_trade_price: number | null;
  icon: string | null;
  accepting_orders: string | null;
  uma_status: string | null;
  has_bot_fills: boolean;
}
export interface LiveUpcomingMarketsResp {
  now: string;
  universe: string[];
  hours_back: number;
  hours_ahead: number;
  min_vol: number;
  market_count: number;
  markets: LiveUpcomingMarket[];
}

// ===== Live portfolio summary (bot KPI header) =====
export interface LivePortfolioSummary {
  alias: string;
  wallet: string;
  scope: "recent" | "all" | string;
  positions_value: number;
  position_legs: number;
  biggest_win: number | null;
  biggest_win_title: string | null;
  biggest_win_slug:  string | null;
  biggest_win_event: string | null;
  predictions: number;
  predictions_24h: number;
  gross_24h: number;
  joined_at: string;
  now: string;
}

// ===== Trader (rn1-trader) live state =====
export interface WalletSnapshot {
  address: string;
  usdc_e_balance: number;
  usdc_native_balance: number;
  usdc_total: number;
  clob_usdc_available: number;
  pol_balance: number;
  positions_count: number;
  positions_value_usdc: number;
  total_value_usdc: number;
  fetched_at: string;
  fetch_error: string | null;
}

export interface TraderConfigSnapshot {
  quote_shares: number;
  per_event_cap_usdc: number;
  daily_loss_cap_usdc: number;
  daily_gross_cap_usdc: number;
  cumulative_aim: number;
  cumulative_target_max: number;
  mid_min: number;
  mid_max: number;
  ladder_depth: number;
  completion_mult: number;
  endgame_after_min: number;
}

export interface StrategyView {
  condition_id: string;
  focus_slug: string;
  event_slug: string | null;
  question: string | null;
  leg0_name: string;
  leg0_shares: number;
  leg0_vwap: number;
  leg1_name: string;
  leg1_shares: number;
  leg1_vwap: number;
  cumulative_vwap: number;
  cumulative_target: number;
  matched_pairs: number;
  naked_shares: number;
  naked_leg_name: string;
}

export interface TraderBot {
  mode: "paper" | "live" | string;
  running: boolean;
  focus_slug: string | null;
  current_tick: number;
  last_cycle_at: string | null;
  last_cycle_ms: number;
  candidates: number;
  ws_market_connected: boolean;
  ws_user_connected: boolean;
  kill_switch: boolean;
  last_error: string | null;
  wallet: WalletSnapshot | null;
  config: TraderConfigSnapshot | null;
  strategy: StrategyView | null;
  updated_at: string;
  note?: string;
}

export interface TraderDaily {
  date: string;
  gross_filled: number;
  realized_pnl: number;
  rebate: number;
  fills_count: number;
  kill_switch: boolean;
}

export interface TraderState {
  bot: TraderBot;
  daily: TraderDaily;
  now: string;
}

export interface TraderOpenOrder {
  order_id: number;
  clob_order_hash: string | null;
  condition_id: string;
  outcome_index: number;
  outcome_name: string | null;
  asset_id: string;
  token_id: string | null;
  side: "BUY" | "SELL" | string;
  price: number;
  size: number;
  size_usdc: number;
  fill_size_shares: number;
  fill_usdc: number;
  remaining_shares: number;
  remaining_usdc: number;
  kind: "maker" | "taker" | "reconcile" | string;
  tag:  "new_pair" | "completion" | "sweep" | "reconcile" | string;
  status: string;
  signed_at: string;
  posted_at: string | null;
  placed_age_sec: number;
  title: string | null;
  slug: string | null;
  event_slug: string | null;
  icon: string | null;
  neg_risk: boolean;
  game_start: string | null;
  end_date: string | null;
  best_bid: number | null;
  best_ask: number | null;
  mid_at_place: number | null;
  /** Seconds since the per-outcome quote was last refreshed by the trader.
   *  Display warning if > ~60s (2026-05-26: ensures cur price reflects
   *  reality, not a 24-min stale cache like Kokkinakis incident). */
  quote_age_s: number | null;
}

export interface TraderOpenOrdersResp {
  orders: TraderOpenOrder[];
  total_open: number;
  total_remaining_usdc: number;
  now: string;
}

export interface TraderClosedOrder {
  order_id: number;
  clob_order_hash: string | null;
  condition_id: string;
  outcome_index: number;
  outcome_name: string | null;
  side: string;
  price: number;
  size: number;
  size_usdc: number;
  fill_size_shares: number;
  fill_usdc: number;
  rebate_usdc: number;
  kind: string;
  tag: string;
  status: string;
  signed_at: string;
  filled_at: string | null;
  cancelled_at: string | null;
  cancelled_reason: string | null;
  title: string | null;
  slug: string | null;
  event_slug: string | null;
  icon: string | null;
}

export interface TraderClosedOrdersResp {
  orders: TraderClosedOrder[];
  now: string;
}
