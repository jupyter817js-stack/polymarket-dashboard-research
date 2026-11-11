"use client";
import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { useState, useEffect, useMemo, useCallback } from "react";
import { api, EventNode, MarketNode, PositionEntry, PositionsQuery, PriceLevel } from "@/lib/api";
import { fmt$, fmtN, fmtPx, cls, cn, relTime } from "@/lib/fmt";
import {
  ChevronRight, Layers, Clock, Search, X, Filter, ArrowDownUp,
  Flame, Timer, TrendingUp, BarChart3, Inbox,
} from "lucide-react";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";

/** /live/positions — RN1-style tree: Event → Market → Outcome (+ price levels).
 *  Polymarket-style 색상: outcome 0 = buy 톤, outcome 1 = sell 톤, PnL +/− 색. */
export default function LivePositionsPage() {
  const [sort, setSort]       = useState<NonNullable<PositionsQuery["sort"]>>("recent");
  const [filter, setFilter]   = useState<NonNullable<PositionsQuery["filter"]>>("all");
  const [pageSize, setPageSize] = useState<number>(50);
  const [page, setPage]       = useState<number>(1);
  const [q, setQ]             = useState<string>("");
  const [debouncedQ, setDebouncedQ] = useState<string>("");
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(q), 250);
    return () => clearTimeout(t);
  }, [q]);
  useEffect(() => { setPage(1); }, [sort, filter, debouncedQ, pageSize]);

  const queryParams: PositionsQuery = { sort, filter, page, page_size: pageSize, q: debouncedQ || undefined };
  const queryRes = useQuery({
    queryKey: ["live-positions-tree", queryParams],
    queryFn: () => api.livePositions(queryParams),
    refetchInterval: 4000,
    staleTime: 3000,
    placeholderData: keepPreviousData,
  });

  const [openEvents, setOpenEvents]       = useState<Set<string>>(new Set());
  const [openMarkets, setOpenMarkets]     = useState<Set<string>>(new Set());
  const [openOutcomes, setOpenOutcomes]   = useState<Set<string>>(new Set());

  const toggleOutcome = useCallback((cid: string, idx: number) => {
    setOpenOutcomes(prev => {
      const n = new Set(prev);
      const key = `${cid}:${idx}`;
      if (n.has(key)) n.delete(key); else n.add(key);
      return n;
    });
  }, []);

  const toggleEventWithChildren = useCallback((ev: EventNode) => {
    const wasOpen = openEvents.has(ev.event_slug);
    setOpenEvents(prev => {
      const n = new Set(prev);
      if (wasOpen) n.delete(ev.event_slug); else n.add(ev.event_slug);
      return n;
    });
    if (!wasOpen) {
      setOpenMarkets(prev => {
        const n = new Set(prev);
        ev.markets.forEach(m => n.add(m.condition_id));
        return n;
      });
    }
  }, [openEvents]);

  const d = queryRes.data;
  const events = d?.events ?? [];
  const totals = useMemo(() => {
    const value = events.reduce((s, ev) => s + ev.total_value, 0);
    const pnl = events.reduce((s, ev) => s + ev.total_cash_pnl, 0);
    const legs = events.reduce((s, ev) => s + ev.leg_count, 0);
    return { value, pnl, legs };
  }, [events]);

  return (
    <div className="space-y-4">
      <PageHeader
        icon={BarChart3}
        title="My Positions"
        subtitle={<>트레이더 누적 포지션 (BUY-only) · 이벤트 → 마켓 → outcome 트리</>}
        tone="accent"
      />

      <Toolbar
        sort={sort} setSort={setSort}
        filter={filter} setFilter={setFilter}
        q={q} setQ={setQ}
        pageSize={pageSize} setPageSize={setPageSize}
        totals={totals}
      />

      {queryRes.isLoading && !d && (
        <div className="surface px-5 py-12 text-center text-[12px] text-text-dim">로딩…</div>
      )}
      {!queryRes.isLoading && events.length === 0 && (
        <EmptyState
          icon={debouncedQ ? Search : Inbox}
          title={debouncedQ ? `"${debouncedQ}" 검색 결과 없음` : "트레이더 누적 포지션 없음"}
          message={debouncedQ ? "다른 검색어를 시도해보세요." : "첫 페어 체결 후 자동 표시됩니다."}
          tone="neutral"
        />
      )}
      {events.length > 0 && (
        <div className="space-y-2">
          {events.map(ev => (
            <EventCard
              key={ev.event_slug}
              ev={ev}
              open={openEvents.has(ev.event_slug)}
              onToggle={() => toggleEventWithChildren(ev)}
              openMarkets={openMarkets}
              toggleMarket={(cid) => toggleSet(openMarkets, cid, setOpenMarkets)}
              openOutcomes={openOutcomes}
              toggleOutcome={toggleOutcome}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function Toolbar({ sort, setSort, filter, setFilter, q, setQ, pageSize, setPageSize, totals }: {
  sort: NonNullable<PositionsQuery["sort"]>; setSort: (v: NonNullable<PositionsQuery["sort"]>) => void;
  filter: NonNullable<PositionsQuery["filter"]>; setFilter: (v: NonNullable<PositionsQuery["filter"]>) => void;
  q: string; setQ: (v: string) => void;
  pageSize: number; setPageSize: (n: number) => void;
  totals: { value: number; pnl: number; legs: number };
}) {
  return (
    <div className="surface px-4 py-3 flex flex-wrap items-center gap-3">
      <div className="relative flex-1 min-w-[220px]">
        <Search className="size-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-text-dim pointer-events-none" />
        <label htmlFor="live-positions-search" className="sr-only">검색</label>
        <input
          id="live-positions-search" name="live-positions-search"
          type="search" autoComplete="off"
          placeholder="제목 / 이벤트 / outcome 검색"
          value={q} onChange={e => setQ(e.target.value)}
          className="input pl-9 pr-8"
        />
        {q && (
          <button onClick={() => setQ("")}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text"
            aria-label="검색 지우기">
            <X className="size-3.5" />
          </button>
        )}
      </div>

      <SimpleSelect label={<><ArrowDownUp className="size-3" /> 정렬</>}
        value={sort} onChange={v => setSort(v as NonNullable<PositionsQuery["sort"]>)}
        options={[
          ["fill",      "최근 체결 ↓"],
          ["recent",    "최근 활동 ↓"],
          ["value",     "Value ↓"],
          ["pnl",       "PnL ↓"],
          ["pnl_asc",   "PnL ↑"],
          ["legs",      "Leg 수 ↓"],
          ["markets",   "Market 수 ↓"],
          ["name",      "이름 A→Z"],
        ]} />

      <SimpleSelect label={<><Filter className="size-3" /> 필터</>}
        value={filter} onChange={v => setFilter(v as NonNullable<PositionsQuery["filter"]>)}
        options={[
          ["all",        "전체"],
          ["active",     "활성 (size>0)"],
          ["redeemable", "정산 대기"],
          ["mergeable",  "MERGE 가능"],
        ]} />

      <SimpleSelect label="페이지" value={String(pageSize)} onChange={v => setPageSize(Number(v))}
        options={[["50", "50/page"], ["100", "100/page"], ["250", "250/page"]]} />

      <div className="ml-auto text-[11px] text-text-muted flex items-center gap-2">
        <span><span className="num text-text">{fmtN(totals.legs)}</span> legs</span>
        <span className="text-text-faint">·</span>
        <span className="num text-text">{fmt$(totals.value, 0)}</span>
        <span className="text-text-faint">·</span>
        <span className={cn("num", cls(totals.pnl))}>{fmt$(totals.pnl, 0)} pnl</span>
      </div>
    </div>
  );
}

function SimpleSelect({ label, value, onChange, options }: {
  label: React.ReactNode; value: string; onChange: (v: string) => void;
  options: [string, string][];
}) {
  return (
    <label className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="flex items-center gap-1">{label}</span>
      <select value={value} onChange={e => onChange(e.target.value)} className="select">
        {options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
      </select>
    </label>
  );
}

function toggleSet(set: Set<string>, key: string, setter: (s: Set<string>) => void) {
  const next = new Set(set);
  if (next.has(key)) next.delete(key); else next.add(key);
  setter(next);
}

// ─────────────────────────────────────────────────────────────────────────
//   EVENT CARD
// ─────────────────────────────────────────────────────────────────────────
function EventCard({ ev, open, onToggle, openMarkets, toggleMarket, openOutcomes, toggleOutcome }: {
  ev: EventNode; open: boolean; onToggle: () => void;
  openMarkets: Set<string>; toggleMarket: (cid: string) => void;
  openOutcomes: Set<string>; toggleOutcome: (cid: string, idx: number) => void;
}) {
  const ago = relTime(ev.last_updated);
  const ageMs = Date.now() - new Date(ev.last_updated).getTime();
  const fresh = ageMs < 60_000;
  const veryFresh = ageMs < 10_000;
  const pnlPositive = ev.total_cash_pnl > 0;
  const pnlNegative = ev.total_cash_pnl < 0;
  const accentColor =
    pnlPositive  ? "bg-gradient-to-b from-buy via-buy/80 to-buy/40" :
    pnlNegative  ? "bg-gradient-to-b from-sell via-sell/80 to-sell/40" :
    veryFresh    ? "bg-accent" :
    fresh        ? "bg-accent/60" : "bg-line";

  const endTs = ev.earliest_end ? new Date(ev.earliest_end).getTime() : null;
  const eta = endTs ? Math.round((endTs - Date.now()) / 1000) : null;

  return (
    <div id={`ev-${ev.event_slug}`}
      className="surface-flush relative scroll-mt-4 transition-all duration-200 hover:border-line-strong">
      <div className={cn("absolute left-0 top-0 bottom-0 w-1 rounded-l-xl", accentColor)} />

      <button onClick={onToggle}
        className="w-full grid grid-cols-[20px_44px_1fr_auto] items-center gap-3 pl-4 pr-5 py-3.5
                   hover:bg-bg-panel/40 transition-colors text-left">
        <ChevronRight className={cn("size-4 text-text-dim shrink-0 transition-transform duration-200", open && "rotate-90")} />

        {ev.sample_icon ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img src={ev.sample_icon} alt="" className="size-11 rounded-xl object-cover shrink-0 bg-bg-panel ring-1 ring-line/60 shadow-soft" loading="lazy" />
        ) : (
          <div className="size-11 rounded-xl bg-gradient-to-br from-accent/15 via-bg-panel to-bg-panel border border-line/70 shrink-0 flex items-center justify-center">
            <Layers className="size-4 text-accent/70" />
          </div>
        )}

        <div className="min-w-0">
          <div className="font-semibold text-[14px] leading-tight truncate text-text mb-1">
            {ev.sample_title || ev.event_slug}
          </div>
          <div className="flex items-center gap-1.5 text-[11px] text-text-muted flex-wrap">
            <span className="chip-outline"><strong className="text-text num">{ev.market_count}</strong> markets</span>
            <span className="chip-outline"><strong className="text-text num">{ev.leg_count}</strong> legs</span>
            {ev.any_redeemable && <span className="chip-green">▶ redeemable</span>}
            {ev.any_mergeable && <span className="chip-purple">⇄ mergeable</span>}
            <EtaBadge eta={eta} />
            <span className={cn("inline-flex items-center gap-1 text-[10px]",
              veryFresh ? "text-buy font-medium" : fresh ? "text-buy/80" : "text-text-dim")}>
              <Clock className="size-3" /> {ago}
            </span>
          </div>
        </div>

        <div className="text-right shrink-0 space-y-0.5">
          <div className="num text-[18px] font-semibold tracking-tightish leading-none">{fmt$(ev.total_value, 0)}</div>
          <div className={cn("num text-[11px] font-medium", cls(ev.total_cash_pnl))}>
            {pnlPositive ? "▲" : pnlNegative ? "▼" : "·"} {fmt$(ev.total_cash_pnl, 2)}
          </div>
        </div>
      </button>

      {open && (
        <div className="border-t border-line/70 bg-bg-panel/30 px-2 py-1.5 space-y-1">
          {ev.markets.map(m => (
            <MarketRow
              key={m.condition_id}
              m={m}
              open={openMarkets.has(m.condition_id)}
              onToggle={() => toggleMarket(m.condition_id)}
              openOutcomes={openOutcomes}
              toggleOutcome={toggleOutcome}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function EtaBadge({ eta }: { eta: number | null }) {
  if (eta == null) return null;
  if (eta < -3600 * 24) return null;
  const abs = Math.abs(eta);
  const label = abs < 3600 ? `${Math.round(abs/60)}m`
              : abs < 86400 ? `${(abs/3600).toFixed(1)}h`
              : `${(abs/86400).toFixed(1)}d`;
  if (eta > 0 && eta < 3600) {
    return <span className="inline-flex items-center gap-1 text-[10px] text-warn font-medium"><Flame className="size-3 animate-pulse" /> {label}</span>;
  }
  if (eta > 0) {
    return <span className="inline-flex items-center gap-1 text-[10px] text-text-muted"><Timer className="size-3" /> {label}</span>;
  }
  return <span className="text-[10px] text-text-dim">ended {label} ago</span>;
}

// ─────────────────────────────────────────────────────────────────────────
//   MARKET ROW (+ ladder helpers)
// ─────────────────────────────────────────────────────────────────────────
function MarketRow({ m, open, onToggle, openOutcomes, toggleOutcome }: {
  m: MarketNode; open: boolean; onToggle: () => void;
  openOutcomes: Set<string>; toggleOutcome: (cid: string, idx: number) => void;
}) {
  const sizes = m.outcomes.map(o => o.size);
  const totalSize = sizes.reduce((a, b) => a + b, 0) || 1;
  const isTwoSided = m.outcomes.length === 2 && sizes.every(s => s > 0);
  const isNaked = m.outcomes.length === 2 && sizes.some(s => s > 0) && sizes.some(s => s === 0);
  const pnlPositive = m.cash_pnl > 0;
  const pnlNegative = m.cash_pnl < 0;

  const hasOpenOutcome = open && m.outcomes.some(o => openOutcomes.has(`${m.condition_id}:${o.outcome_index}`));
  const breakdownQ = useQuery({
    queryKey: ["live-breakdown", m.condition_id],
    queryFn: () => api.livePositionBreakdown(m.condition_id),
    enabled: hasOpenOutcome,
    staleTime: 15_000,
    refetchInterval: hasOpenOutcome ? 10_000 : false,
  });

  return (
    <div id={`m-${m.condition_id}`} className="rounded-md scroll-mt-4">
      <button onClick={onToggle}
        className={cn(
          "w-full flex items-center gap-2 px-3 py-2 hover:bg-bg-panel rounded-md text-left transition-colors",
          isNaked && "ring-1 ring-warn/30"
        )}>
        <ChevronRight className={cn("size-3.5 text-text-dim transition-transform duration-200", open && "rotate-90")} />
        <div className="flex-1 min-w-0">
          <div className="text-[13px] truncate flex items-center gap-1.5">
            <span className="truncate">{m.title || m.condition_id.slice(0, 14)}</span>
            {m.neg_risk && <span className="chip-amber">neg</span>}
            {isTwoSided && <span className="chip-purple">two-sided</span>}
            {isNaked && <span className="chip-amber">naked</span>}
          </div>
          {m.outcomes.length === 2 && (
            <div className="flex h-1 mt-1.5 rounded-full overflow-hidden bg-line/40 max-w-[300px]">
              {m.outcomes.map(o => (
                <div key={o.outcome_index}
                  className={cn(o.outcome_index === 0 ? "bg-buy/70" : "bg-sell/70")}
                  style={{ width: `${(o.size / totalSize) * 100}%` }} />
              ))}
            </div>
          )}
          <div className="flex items-center gap-1 mt-1 text-[10px] text-text-dim flex-wrap">
            {m.outcomes.map(o => {
              const stale = (o.quote_age_s ?? 0) > 60;
              return (
                <span key={o.outcome_index} className={cn("chip transition-colors",
                  o.outcome_index === 0 ? "chip-green" : "chip-red")}>
                  {o.outcome_name} · <span className="num">{fmtN(o.size, 1)}</span>
                  @ <span className="num">{fmtPx(o.avg_price)}</span>
                  <span className="text-text-dim mx-0.5">→</span>
                  <span className={cn("num font-medium",
                    o.outcome_index === 0 ? "text-buy" : "text-sell")}>{fmtPx(o.current_price)}</span>
                  {stale && (
                    <span className="ml-1 px-1 rounded text-[9px] bg-warn/20 text-warn font-medium"
                          title={`Quote ${Math.floor((o.quote_age_s ?? 0) / 60)}분 stale`}>
                      stale
                    </span>
                  )}
                </span>
              );
            })}
          </div>
        </div>
        <div className="text-right shrink-0">
          <div className="num text-[13px] font-medium">{fmt$(m.current_value, 2)}</div>
          <div className={cn("num text-[10.5px] font-medium", cls(m.cash_pnl))}>
            {pnlPositive ? "▲" : pnlNegative ? "▼" : "·"} {fmt$(m.cash_pnl, 2)}
          </div>
        </div>
      </button>

      {open && (
        <div className="px-3 pb-2 pt-1 space-y-0.5">
          {m.outcomes.map(o => (
            <OutcomeLine key={o.outcome_index} o={o} cid={m.condition_id}
              isOpen={openOutcomes.has(`${m.condition_id}:${o.outcome_index}`)}
              onToggle={() => toggleOutcome(m.condition_id, o.outcome_index)}
              levels={breakdownQ.data?.price_levels.filter(l => l.outcome_index === o.outcome_index) ?? []}
              isLoadingLevels={breakdownQ.isLoading || breakdownQ.isFetching}
              hasLevelsData={!!breakdownQ.data}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
//   OUTCOME LINE
// ─────────────────────────────────────────────────────────────────────────
function OutcomeLine({ o, isOpen, onToggle, levels, isLoadingLevels, hasLevelsData }: {
  o: PositionEntry; cid: string;
  isOpen: boolean; onToggle: () => void;
  levels: PriceLevel[];
  isLoadingLevels: boolean; hasLevelsData: boolean;
}) {
  const isIdx0 = o.outcome_index === 0;
  const tone = isIdx0 ? "buy" : "sell";
  const pnlPositive = o.cash_pnl > 0;
  const pnlNegative = o.cash_pnl < 0;
  const pnlPct = (o.avg_price && o.avg_price > 0)
    ? ((o.current_price ?? 0) - o.avg_price) / o.avg_price * 100
    : null;

  return (
    <div className={cn(
      "ml-4 rounded-md transition-colors relative",
      "border-l-2",
      tone === "buy" ? "border-buy/40" : "border-sell/40",
    )}>
      <button onClick={onToggle}
        className="w-full py-1.5 px-2 text-[12px] grid grid-cols-12 gap-2 text-left hover:bg-bg-panel/60 rounded">
        <div className="col-span-4 truncate flex items-center gap-1.5">
          <ChevronRight className={cn("size-3 text-text-dim shrink-0 transition-transform duration-200", isOpen && "rotate-90")} />
          <span className={tone === "buy" ? "dot-buy" : "dot-sell"} />
          <span className={cn("text-text-dim text-[10px] font-mono")}>[{o.outcome_index}]</span>
          <span className={cn("font-medium truncate",
            tone === "buy" ? "text-buy" : "text-sell")}>{o.outcome_name}</span>
        </div>
        <div className="col-span-2 num text-text-muted">
          size <span className="text-text font-medium">{fmtN(o.size, 1)}</span>
        </div>
        <div className="col-span-2 num text-text-muted">
          avg <span className="text-text">{fmtPx(o.avg_price)}</span>
        </div>
        <div className="col-span-2 num text-text-muted">
          cur <span className={cn(
            tone === "buy" ? "text-buy" : "text-sell"
          )}>{fmtPx(o.current_price)}</span>
          {(o.quote_age_s ?? 0) > 60 && (
            <span className="ml-1 px-1 rounded text-[9px] bg-warn/20 text-warn font-medium"
                  title={`Quote ${Math.floor((o.quote_age_s ?? 0) / 60)}분 stale — 가격이 부정확할 수 있음`}>
              stale
            </span>
          )}
        </div>
        <div className="col-span-2 num text-right">
          <span className={cn("font-semibold", cls(o.cash_pnl))}>
            {pnlPositive ? "+" : ""}{fmt$(o.cash_pnl, 2)}
          </span>
          {pnlPct != null && (
            <span className={cn("ml-1 text-[10px]", cls(o.cash_pnl))}>
              ({pnlPct >= 0 ? "+" : ""}{pnlPct.toFixed(1)}%)
            </span>
          )}
        </div>
      </button>

      {isOpen && (
        <PriceLevelTable
          levels={levels}
          isLoading={isLoadingLevels && !hasLevelsData}
          outcome={o}
        />
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
//   PRICE LEVEL TABLE
// ─────────────────────────────────────────────────────────────────────────
function PriceLevelTable({ levels, isLoading, outcome }: {
  levels: PriceLevel[]; isLoading: boolean; outcome: PositionEntry;
}) {
  const isIdx0 = outcome.outcome_index === 0;
  const idxBar    = isIdx0 ? "bg-buy/60" : "bg-sell/60";
  const idxChip   = isIdx0 ? "bg-buy/15 text-buy"     : "bg-sell/15 text-sell";
  const idxBorder = isIdx0 ? "border-buy/30"          : "border-sell/30";

  const recalc = useMemo(() => {
    let totalSize = 0, totalUsdc = 0;
    for (const l of levels) {
      if (l.side === "BUY") {
        totalSize += l.total_size;
        totalUsdc += l.total_usdc;
      }
    }
    return { totalSize, totalUsdc, wavg: totalSize > 0 ? totalUsdc / totalSize : null };
  }, [levels]);

  if (isLoading) {
    return (
      <div className={cn("ml-7 mb-2 mr-2 px-3 py-2 text-[11px] text-text-muted bg-bg-panel/60 border-l-2 rounded", idxBorder)}>
        가격대별 분해 로드 중…
      </div>
    );
  }
  if (levels.length === 0) {
    return (
      <div className={cn("ml-7 mb-2 mr-2 px-3 py-2 text-[11px] text-text-muted bg-bg-panel/60 border-l-2 rounded", idxBorder)}>
        이 outcome 에 트레이더 fill 없음.
      </div>
    );
  }

  const sorted = [...levels].sort((a, b) => b.price_cents - a.price_cents);
  const maxSize = Math.max(...sorted.map(l => l.total_size));

  return (
    <div className={cn(
      "ml-7 mb-2 mr-2 bg-bg-panel/40 border-l-2 rounded-md overflow-hidden",
      idxBorder,
    )}>
      <div className="grid grid-cols-[60px_50px_1fr_90px_80px_60px_80px] px-3 py-2 text-[10px] uppercase tracking-wider2 text-text-muted font-medium border-b border-line/40 bg-bg-soft/30">
        <div>Price</div><div>Side</div><div>크기 (시각)</div>
        <div className="text-right">Shares</div>
        <div className="text-right">USDC</div>
        <div className="text-right">Fills</div>
        <div className="text-right">Last</div>
      </div>
      {sorted.map((l) => {
        const pct = (l.total_size / maxSize) * 100;
        return (
          <div key={`${l.side}-${l.price_cents}`}
            className="grid grid-cols-[60px_50px_1fr_90px_80px_60px_80px] px-3 py-1.5 text-[11px] items-center border-b border-line/20 last:border-b-0 hover:bg-bg-panel/40">
            <div className="num font-medium">{l.price_cents}¢</div>
            <div><span className={cn("text-[10px] px-1.5 py-0.5 rounded font-medium", idxChip)}>{l.side}</span></div>
            <div>
              <div className="h-1.5 rounded-full bg-line/30 overflow-hidden max-w-[260px]">
                <div className={cn("h-full", idxBar)} style={{ width: `${pct}%` }} />
              </div>
            </div>
            <div className="text-right num">{fmtN(l.total_size, 1)}</div>
            <div className="text-right num text-text-muted">{fmt$(l.total_usdc, 2)}</div>
            <div className="text-right num text-text-muted">{l.fill_count}</div>
            <div className="text-right text-[10px] text-text-dim">{relTime(l.last_ts)}</div>
          </div>
        );
      })}

      {recalc.wavg != null && (
        <div className="bg-bg-soft/40 border-t border-line/40 px-3 py-2 text-[10px] flex items-center gap-2.5 flex-wrap">
          <TrendingUp className="size-3 text-text-dim shrink-0" />
          <span className="text-text-muted">
            BUY 가중평균 <span className="text-text num font-medium">{recalc.wavg.toFixed(4)}</span>
            <span className="text-text-dim ml-1">({fmt$(recalc.totalUsdc, 2)} / {fmtN(recalc.totalSize, 1)})</span>
          </span>
        </div>
      )}
    </div>
  );
}
