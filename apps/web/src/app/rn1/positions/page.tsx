"use client";
import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { useState, useEffect, useMemo, useRef, useCallback, Suspense } from "react";
import { useSearchParams, useRouter } from "next/navigation";
import { api, EventNode, MarketNode, PositionEntry, PositionsQuery, PriceLevel } from "@/lib/api";
import { fmt$, fmtN, fmtPx, cls, cn, relTime } from "@/lib/fmt";
import { ChevronRight, Layers, AlertCircle, CheckCircle2, Clock, Search, X, Filter, ArrowDownUp, Bell, ShoppingCart, Flame, Timer, Zap, TrendingUp } from "lucide-react";
import { PageBar } from "@/components/pagebar";

// 모듈 스코프 — 자식 컴포넌트 들이 prop type 으로 참조
type FlashInfo = { t: number; delta: number; price: number | null; outcome: string; title: string; eventSlug: string | null; cid: string; idx: number };

/** RN1 의 보유 포지션 — 이벤트 트리 + 페이지네이션 + 필터 + 검색 + 정렬.
 *  3K+ 이벤트를 한꺼번에 렌더하지 않고, 100개씩 페이지로. framer-motion 의
 *  layout/exit 애니메이션은 제거 (성능). flash highlight 는 CSS keyframe 으로.
 */
export default function PositionsPage() {
  // useSearchParams 는 Suspense boundary 필요 (Next.js 15 client component)
  return (
    <Suspense fallback={<Loading />}>
      <PositionsContent />
    </Suspense>
  );
}

function PositionsContent() {
  const searchParams = useSearchParams();
  const router = useRouter();
  // Activity 페이지에서 deep link 로 진입 시 — q 를 event slug 로 초기화해서 backend 가
  // 해당 이벤트만 보내도록 좁히고, useEffect 에서 매칭되는 EventNode 를 expand + scroll.
  const linkedEv = searchParams.get("ev");
  const linkedCid = searchParams.get("cid");

  const [sort, setSort]       = useState<NonNullable<PositionsQuery["sort"]>>("recent");
  const [filter, setFilter]   = useState<NonNullable<PositionsQuery["filter"]>>(linkedEv || linkedCid ? "all" : "recent");
  const [pageSize, setPageSize] = useState<number>(50);
  const [page, setPage]       = useState<number>(1);
  const [q, setQ]             = useState<string>(linkedEv ?? "");
  const [debouncedQ, setDebouncedQ] = useState<string>(linkedEv ?? "");
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(q), 250);
    return () => clearTimeout(t);
  }, [q]);
  // 옵션 변경 시 page 리셋
  useEffect(() => { setPage(1); }, [sort, filter, debouncedQ, pageSize]);

  const queryParams: PositionsQuery = { sort, filter, page, page_size: pageSize, q: debouncedQ || undefined };
  const queryRes = useQuery({
    queryKey: ["positions", queryParams],
    queryFn: () => api.positions(queryParams),
    // RN1 forensic 데이터 — 기록 용도. 30초마다 갱신이면 충분.
    refetchInterval: 30_000,
    staleTime: 25_000,
    placeholderData: keepPreviousData,
  });

  const [openEvents, setOpenEvents] = useState<Set<string>>(new Set());
  const [openMarkets, setOpenMarkets] = useState<Set<string>>(new Set());
  // 가격대 분해까지 펼침 — key 는 `${cid}:${outcome_index}` 형태. 부모 (event/toast) 가
  // 한꺼번에 자식 outcomes 까지 expand 시키기 위해 PositionsPage 로 끌어올린 state.
  const [openOutcomes, setOpenOutcomes] = useState<Set<string>>(new Set());

  // 변화 추적 — 새로 들어온 leg 강조 (8초 유지) + toast 알림
  const flashMapRef = useRef<Map<string, FlashInfo>>(new Map());
  const prevLegSnap = useRef<Map<string, number>>(new Map());
  const [toasts, setToasts] = useState<FlashInfo[]>([]);

  useEffect(() => {
    if (!queryRes.data) return;
    const now = Date.now();
    const cur = new Map<string, number>();
    const meta = new Map<string, { price: number | null; outcome: string; title: string; eventSlug: string | null }>();
    for (const ev of queryRes.data.events) {
      for (const m of ev.markets) {
        for (const o of m.outcomes) {
          const id = `${m.condition_id}:${o.outcome_index}`;
          cur.set(id, o.size);
          // 실제 fill 가격 우선 (last_fill_price) → 없으면 current_price → avg_price.
          // current_price 는 시장 mid 라서 RN1 의 maker rest 가격과 다름 → 토스트에서 1¢ 차이 노이즈.
          meta.set(id, {
            price: o.last_fill_price ?? o.current_price ?? o.avg_price,
            outcome: o.outcome_name,
            title: m.title || o.condition_id.slice(0, 14),
            eventSlug: ev.event_slug,
          });
        }
      }
    }
    const newToasts: FlashInfo[] = [];
    cur.forEach((size, id) => {
      const prev = prevLegSnap.current.get(id);
      // size 가 증가 (새 매수)
      if (prev != null && size > prev + 1e-6) {
        const m = meta.get(id)!;
        const [cid, idxStr] = id.split(":");
        const info: FlashInfo = {
          t: now, delta: size - prev,
          price: m.price, outcome: m.outcome, title: m.title, eventSlug: m.eventSlug,
          cid, idx: Number(idxStr),
        };
        flashMapRef.current.set(id, info);
        newToasts.push(info);
      }
    });
    prevLegSnap.current = cur;
    // garbage collect 8초 이상 된 flash
    const expire = now - 8_000;
    flashMapRef.current.forEach((info, k) => { if (info.t < expire) flashMapRef.current.delete(k); });
    if (newToasts.length) {
      setToasts(t => [...newToasts, ...t].slice(0, 5)); // 최근 5건
      // 자동 dismiss 8초
      setTimeout(() => setToasts(t => t.filter(x => Date.now() - x.t < 8000)), 8000);
    }
  }, [queryRes.data]);

  // Map<id, FlashInfo> — 자식 컴포넌트 가 delta/price 까지 조회 가능
  const flashIds = useMemo(() => new Map(flashMapRef.current), [toasts]);

  // Deep link 처리 — Activity row 클릭으로 ?ev=&cid= 가 들어오면
  // 매칭되는 event 를 expand + scroll. 동일 링크에 대해 한 번만, 다른 링크면 다시 적용.
  const lastDeepLinkRef = useRef<string>("");
  useEffect(() => {
    const key = `${linkedEv ?? ""}:${linkedCid ?? ""}`;
    if (key === ":" ) return;
    if (lastDeepLinkRef.current === key) return;
    if (!queryRes.data) return;

    const events = queryRes.data.events;
    const target = linkedEv
      ? events.find(e => e.event_slug === linkedEv)
      : events.find(e => e.markets.some(m => m.condition_id === linkedCid));
    if (!target) return;

    lastDeepLinkRef.current = key;

    setOpenEvents(prev => {
      const n = new Set(prev); n.add(target.event_slug); return n;
    });
    setOpenMarkets(prev => {
      const n = new Set(prev);
      target.markets.forEach(m => n.add(m.condition_id));
      return n;
    });
    setOpenOutcomes(prev => {
      const n = new Set(prev);
      target.markets.forEach(m =>
        m.outcomes.forEach(o => n.add(`${m.condition_id}:${o.outcome_index}`))
      );
      return n;
    });

    setTimeout(() => {
      const id = linkedCid ? `m-${linkedCid}` : `ev-${target.event_slug}`;
      const el = document.getElementById(id) ?? document.getElementById(`ev-${target.event_slug}`);
      el?.scrollIntoView({ behavior: "smooth", block: "start" });
      // 적용 후 URL 정리 — 새로고침/공유 시 deep link 가 stale 하지 않도록
      router.replace("/rn1/positions", { scroll: false });
    }, 200);
  }, [queryRes.data, linkedEv, linkedCid, router]);

  // outcome 한 개 토글 — OutcomeLine 의 chevron 클릭 시
  const toggleOutcome = useCallback((cid: string, idx: number) => {
    setOpenOutcomes(prev => {
      const n = new Set(prev);
      const key = `${cid}:${idx}`;
      if (n.has(key)) n.delete(key); else n.add(key);
      return n;
    });
  }, []);

  // event toggle — 펼칠 때 자식 markets + 모든 outcomes (가격대 표) 까지 자동 펼침.
  // 닫을 땐 event 만 닫기 (다음 펼칠 때 자식 상태 유지).
  // 모든 hook 은 early return 보다 위에 — Rules of Hooks 준수
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
      setOpenOutcomes(prev => {
        const n = new Set(prev);
        ev.markets.forEach(m =>
          m.outcomes.forEach(o => n.add(`${m.condition_id}:${o.outcome_index}`))
        );
        return n;
      });
    }
  }, [openEvents]);

  // toast 클릭 → 해당 이벤트 + 그 이벤트 의 모든 markets + outcomes 펼치고 scrollIntoView
  const goToFlash = useCallback((info: FlashInfo) => {
    const ev = info.eventSlug ? queryRes.data?.events.find(e => e.event_slug === info.eventSlug) : undefined;
    if (info.eventSlug) {
      setOpenEvents(prev => {
        const n = new Set(prev); n.add(info.eventSlug!); return n;
      });
    }
    setOpenMarkets(prev => {
      const n = new Set(prev);
      if (ev) ev.markets.forEach(m => n.add(m.condition_id));
      else n.add(info.cid);
      return n;
    });
    setOpenOutcomes(prev => {
      const n = new Set(prev);
      if (ev) {
        ev.markets.forEach(m =>
          m.outcomes.forEach(o => n.add(`${m.condition_id}:${o.outcome_index}`))
        );
      } else {
        n.add(`${info.cid}:${info.idx}`);
      }
      return n;
    });
    setTimeout(() => {
      const el = info.eventSlug
        ? document.getElementById(`ev-${info.eventSlug}`)
        : document.getElementById(`m-${info.cid}`);
      el?.scrollIntoView({ behavior: "smooth", block: "start" });
    }, 80);
  }, [queryRes.data]);

  if (queryRes.isLoading && !queryRes.data) return <Loading />;
  if (queryRes.isError) return <Empty msg={`api 연결 실패 — ${(queryRes.error as Error).message}`} />;
  if (!queryRes.data) return null;

  const d = queryRes.data;
  const events = d.events;
  const totalPages = d.total_pages ?? 1;

  return (
    <div className="space-y-3 relative">
      {/* 새 매수 toast 알림 — 우측 상단. 클릭하면 해당 이벤트 펼치고 그쪽으로 스크롤. */}
      <ToastStack
        toasts={toasts}
        onDismiss={(t) => setToasts(prev => prev.filter(x => x.t !== t))}
        onClick={goToFlash}
      />

      <Toolbar
        sort={sort} setSort={setSort}
        filter={filter} setFilter={setFilter}
        q={q} setQ={setQ}
        pageSize={pageSize} setPageSize={setPageSize}
        page={page} setPage={setPage}
        totalPages={totalPages}
        eventTotal={d.event_count}
        showingFrom={(page-1)*pageSize + 1}
        showingTo={Math.min(page*pageSize, d.event_count)}
        totalValue={d.total_value}
        totalCashPnl={d.total_cash_pnl}
      />

      {/* 헤더 PageBar (상단) — 페이지 변경 시 즉시 스크롤 안 내려가도록 */}
      {totalPages > 1 && (
        <PageBar page={page} totalPages={totalPages} setPage={setPage}
          totalLabel={`${fmtN(d.event_count)} events`} />
      )}

      {events.length === 0 ? (
        <Empty msg={debouncedQ ? `"${debouncedQ}" 검색 결과 없음` : "이 필터에 맞는 포지션 없음"} />
      ) : (
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
              flashIds={flashIds}
            />
          ))}
        </div>
      )}

      {/* 푸터 PageBar */}
      {totalPages > 1 && (
        <PageBar page={page} totalPages={totalPages} setPage={setPage} />
      )}
    </div>
  );
}

function Toolbar(props: {
  sort: NonNullable<PositionsQuery["sort"]>; setSort: (v: NonNullable<PositionsQuery["sort"]>) => void;
  filter: NonNullable<PositionsQuery["filter"]>; setFilter: (v: NonNullable<PositionsQuery["filter"]>) => void;
  q: string; setQ: (v: string) => void;
  pageSize: number; setPageSize: (n: number) => void;
  page: number; setPage: (n: number) => void;
  totalPages: number;
  eventTotal: number; showingFrom: number; showingTo: number;
  totalValue: number; totalCashPnl: number;
}) {
  const { sort, setSort, filter, setFilter, q, setQ, pageSize, setPageSize, totalPages,
    eventTotal, showingFrom, showingTo, totalValue, totalCashPnl } = props;
  return (
    <div className="surface px-4 py-3 flex flex-wrap items-center gap-3">
      <div className="relative flex-1 min-w-[220px]">
        <Search className="size-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-text-dim pointer-events-none" />
        <label htmlFor="rn1-positions-search" className="sr-only">제목 / 이벤트 / outcome 검색</label>
        <input
          id="rn1-positions-search" name="rn1-positions-search"
          type="search" autoComplete="off"
          placeholder="제목 / 이벤트 / outcome 검색"
          value={q} onChange={e => setQ(e.target.value)}
          className="input pl-9 pr-8"
        />
        {q && (
          <button onClick={() => setQ("")}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text transition-colors"
            aria-label="검색 지우기">
            <X className="size-3.5" />
          </button>
        )}
      </div>

      <Select label={<><ArrowDownUp className="size-3" /> 정렬</>} value={sort} onChange={setSort} options={[
        ["recent",    "최근 활동 ↓"],
        ["oldest",    "오래된 활동 ↑"],
        ["value",     "Cur Value ↓"],
        ["value_asc", "Cur Value ↑"],
        ["pnl",       "Cash PnL ↓"],
        ["pnl_asc",   "Cash PnL ↑"],
        ["realized",  "Realized PnL ↓"],
        ["legs",      "Leg 수 ↓"],
        ["markets",   "Market 수 ↓"],
        ["name",      "이름 A→Z"],
        ["end",       "종료 임박 ↑"],
        ["end_desc",  "종료 멀음 ↓"],
      ]} />

      <Select label={<><Filter className="size-3" /> 필터</>} value={filter} onChange={setFilter} options={[
        ["recent",     "최근 24h 활동"],
        ["active",     "활성 (size>0)"],
        ["redeemable", "정산 대기"],
        ["mergeable",  "MERGE 가능"],
        ["all",        "전체"],
      ]} />

      <Select label="페이지" value={String(pageSize)} onChange={v => setPageSize(Number(v))} options={[
        ["50", "50/page"], ["100", "100/page"], ["250", "250/page"], ["500", "500/page"],
      ]} />

      <div className="ml-auto text-[11px] text-text-muted flex items-center gap-2">
        <span><span className="num">{showingFrom}</span>–<span className="num">{showingTo}</span> <span className="text-text-dim">/</span> <span className="num">{fmtN(eventTotal)}</span> events</span>
        <span className="text-text-faint">·</span>
        <span className="num text-text">{fmt$(totalValue, 0)}</span>
        <span className="text-text-faint">·</span>
        <span className={cn("num", cls(totalCashPnl))}>{fmt$(totalCashPnl, 0)} cash</span>
      </div>
    </div>
  );
}

function Select({ label, value, onChange, options }: {
  label: React.ReactNode; value: string; onChange: (v: any) => void;
  options: [string, string][];
}) {
  return (
    <label className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="flex items-center gap-1">{label}</span>
      <select value={value} onChange={e => onChange(e.target.value)} className="select">
        {options.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
      </select>
    </label>
  );
}

function toggleSet(set: Set<string>, key: string, setter: (s: Set<string>) => void) {
  const next = new Set(set);
  if (next.has(key)) next.delete(key); else next.add(key);
  setter(next);
}

function EventCard({ ev, open, onToggle, openMarkets, toggleMarket, openOutcomes, toggleOutcome, flashIds }: {
  ev: EventNode; open: boolean; onToggle: () => void;
  openMarkets: Set<string>; toggleMarket: (cid: string) => void;
  openOutcomes: Set<string>; toggleOutcome: (cid: string, idx: number) => void;
  flashIds: Map<string, FlashInfo>;
}) {
  const ago = relTime(ev.last_updated);
  const ageMs = Date.now() - new Date(ev.last_updated).getTime();
  const fresh = ageMs < 60_000;
  const veryFresh = ageMs < 10_000;
  // 어느 outcome 들이 flashed 인지 — event 단위 boolean 이 아니라 개별 식별이 핵심
  const flashedLegs = ev.markets.flatMap(m => m.outcomes
    .filter(o => flashIds.has(`${m.condition_id}:${o.outcome_index}`))
    .map(o => ({ cid: m.condition_id, idx: o.outcome_index, name: o.outcome_name, title: m.title })));
  const flashed = flashedLegs.length > 0;

  // 활동성 — 24h fill 추정 (positions metadata 에서 직접 못 봐서 ev 의 outcomes total_bought 변화량으로 대신, here we use leg_count and updated freshness)
  // 좌측 컬러 바: 활동성/상태 시각화. 전체 카드 pulse 는 제거 — 어느 outcome 이 새로 들어왔는지 식별 못 함.
  const accentColor =
    flashed ? "bg-buy" :
    ev.any_redeemable ? "bg-buy/70" :
    ev.any_mergeable ? "bg-accent-2/70" :
    veryFresh ? "bg-accent" :
    fresh ? "bg-accent/60" : "bg-line";

  // game timing (가장 임박한 end_date 사용)
  const endTs = ev.earliest_end ? new Date(ev.earliest_end).getTime() : null;
  const eta = endTs ? Math.round((endTs - Date.now()) / 1000) : null;

  return (
    <div id={`ev-${ev.event_slug}`} className={cn(
      "surface relative transition-all scroll-mt-4 overflow-hidden",
      // 새 fill 식별 — 매우 미묘한 색조, 깜박임 없음
      flashed && "bg-buy/[0.025]",
    )}>
      {/* 좌측 강조 바 */}
      <div className={cn("absolute left-0 top-0 bottom-0 w-1", accentColor)} />

      <button onClick={onToggle}
        className="w-full grid grid-cols-[20px_44px_1fr_auto] items-center gap-3 pl-4 pr-5 py-3.5 hover:bg-bg-panel/40 transition-colors text-left">
        <ChevronRight className={cn("size-4 text-text-dim shrink-0 transition-transform", open && "rotate-90")} />

        {ev.sample_icon ? (
          <img src={ev.sample_icon} alt="" className="size-11 rounded-lg object-cover shrink-0 bg-bg-panel ring-1 ring-line/60" loading="lazy" />
        ) : (
          <div className="size-11 rounded-lg bg-bg-panel border border-line shrink-0 flex items-center justify-center">
            <Layers className="size-4 text-text-dim" />
          </div>
        )}

        <div className="min-w-0">
          <div className="flex items-center gap-2 mb-1.5">
            {flashed && (
              <span className="inline-flex items-center gap-1 text-[10px] text-buy font-medium shrink-0"
                title={flashedLegs.map(l => `${l.name} (${l.title ?? l.cid.slice(0,10)})`).join("\n")}>
                <Zap className="size-3" />{flashedLegs.length > 1 ? ` ×${flashedLegs.length}` : ""}
              </span>
            )}
            <div className="font-semibold text-[14px] leading-tight truncate flex-1 text-text">
              {ev.sample_title || ev.event_slug}
            </div>
          </div>
          <div className="flex items-center gap-1.5 text-[11px] text-text-muted flex-wrap">
            <span className="chip-outline"><strong className="text-text num">{ev.market_count}</strong> markets</span>
            <span className="chip-outline"><strong className="text-text num">{ev.leg_count}</strong> legs</span>
            {ev.any_redeemable && <span className="chip-green">▶ redeemable</span>}
            {ev.any_mergeable && <span className="chip-purple">⇄ mergeable</span>}
            <EtaBadge eta={eta} />
            <span className={cn("inline-flex items-center gap-1 text-[10px]", veryFresh ? "text-buy font-medium" : fresh ? "text-buy/80" : "text-text-dim")}>
              <Clock className="size-3" /> {ago}
            </span>
          </div>
        </div>

        <div className="text-right shrink-0 space-y-0.5">
          <div className="num text-[18px] font-semibold tracking-tightish leading-none">{fmt$(ev.total_value, 0)}</div>
          <div className={cn("num text-[11px]", cls(ev.total_cash_pnl))}>
            {ev.total_cash_pnl > 0 ? "▲" : ev.total_cash_pnl < 0 ? "▼" : "·"} {fmt$(ev.total_cash_pnl, 0)} <span className="text-text-dim font-sans">cash</span>
          </div>
          {ev.total_realized_pnl !== 0 && (
            <div className={cn("num text-[10.5px]", cls(ev.total_realized_pnl))}>
              ◆ {fmt$(ev.total_realized_pnl, 0)} <span className="text-text-dim font-sans">real</span>
            </div>
          )}
        </div>
      </button>

      {open && (
        <div className="border-t border-line bg-bg-panel/40 px-2 py-1">
          {(() => {
            // Markets 를 category 별로 묶어서 그룹 헤더 표시.
            // 같은 event 안에 moneyline / ou-totals / spread / etc 흩어진 markets 를
            // family 단위로 시각 grouping.
            const groups: { cat: string; markets: MarketNode[] }[] = [];
            let curCat: string | null = null;
            for (const m of ev.markets) {
              if (m.category !== curCat) {
                groups.push({ cat: m.category, markets: [m] });
                curCat = m.category;
              } else {
                groups[groups.length - 1].markets.push(m);
              }
            }
            return groups.map((g, gi) => {
              // ladder view 적용 — O/U 또는 spread 의 multi-threshold 인 경우
              const isLadder = (g.cat === "ou-totals" || g.cat === "spread") && g.markets.length >= 2;
              return (
                <div key={gi}>
                  {ev.markets.length > 1 && g.markets.length > 0 && (
                    <CategoryHeader cat={g.cat} count={g.markets.length} markets={g.markets} />
                  )}
                  {isLadder ? (
                    <LadderPanel
                      cat={g.cat}
                      markets={g.markets}
                      openMarkets={openMarkets}
                      toggleMarket={toggleMarket}
                      openOutcomes={openOutcomes}
                      toggleOutcome={toggleOutcome}
                      flashIds={flashIds}
                    />
                  ) : (
                    g.markets.map(m => (
                      <MarketRow
                        key={m.condition_id}
                        m={m}
                        open={openMarkets.has(m.condition_id)}
                        onToggle={() => toggleMarket(m.condition_id)}
                        openOutcomes={openOutcomes}
                        toggleOutcome={toggleOutcome}
                        flashIds={flashIds}
                      />
                    ))
                  )}
                </div>
              );
            });
          })()}
        </div>
      )}
    </div>
  );
}

/** O/U 또는 spread 의 multi-threshold family 를 neg-risk 같은 ladder table 로 표시.
 *  각 row = 한 threshold (= 한 condition_id), 두 column = Over/Under 또는 Side A/B.
 *  빈 cell = 매수 안 한 outcome. 클릭하면 그 market 의 price-level breakdown 펼침. */
function LadderPanel({ cat, markets, openMarkets, toggleMarket, openOutcomes, toggleOutcome, flashIds }: {
  cat: string;
  markets: MarketNode[];
  openMarkets: Set<string>; toggleMarket: (cid: string) => void;
  openOutcomes: Set<string>; toggleOutcome: (cid: string, idx: number) => void;
  flashIds: Map<string, FlashInfo>;
}) {
  // 두 column label
  const colLabels = cat === "ou-totals"
    ? ["Over", "Under"]
    : ["Side A", "Side B"];  // spread 는 cid 별로 home/away 다름, generic label

  const isMergeable = cat === "ou-totals";  // O/U 는 threshold 간 MERGE 관계는 없지만, 같은 threshold 안의 over+under 는 페어 MERGE 가능

  return (
    <div className="mx-2 my-1 rounded-md border border-line/60 bg-bg-elevated/30 overflow-hidden">
      <div className="grid grid-cols-[80px_1fr_1fr] px-3 py-1.5 text-[9.5px] uppercase tracking-wider2 text-text-muted font-medium border-b border-line bg-bg-soft/30">
        <div>threshold</div>
        <div className="text-buy">{colLabels[0]}</div>
        <div className="text-sell">{colLabels[1]}</div>
      </div>
      {markets.map(m => {
        const o0 = m.outcomes.find(o => o.outcome_index === 0);
        const o1 = m.outcomes.find(o => o.outcome_index === 1);
        const isMOpen = openMarkets.has(m.condition_id);
        const flashed0 = o0 && flashIds.has(`${m.condition_id}:0`);
        const flashed1 = o1 && flashIds.has(`${m.condition_id}:1`);
        const hasBoth = !!o0 && !!o1;  // 양면 매수 = MERGE 가능

        // For spread, infer side label from outcome_name
        const labelA = o0?.outcome_name || colLabels[0];
        const labelB = o1?.outcome_name || colLabels[1];

        return (
          <div key={m.condition_id}>
            <div className="grid grid-cols-[80px_1fr_1fr] px-3 py-2 text-[11px] items-center border-b border-line/30 hover:bg-bg-panel/30 transition-colors">
              {/* threshold column */}
              <button onClick={() => toggleMarket(m.condition_id)}
                className="flex items-center gap-1 text-left hover:text-accent transition-colors">
                <ChevronRight className={cn("size-3 text-text-dim shrink-0 transition-transform", isMOpen && "rotate-90")} />
                <span className="num font-semibold">
                  {cat === "ou-totals" ? "≥" : ""}{m.family_param ?? "—"}
                </span>
                {hasBoth && isMergeable && (
                  <span className="ml-1 text-[8.5px] text-accent-2 font-bold" title="양 outcome 매수 — MERGE 가능">⇄</span>
                )}
              </button>
              {/* Side A (outcome 0) */}
              <LadderCell o={o0} flashed={flashed0} side="A" cid={m.condition_id} sideLabel={labelA}
                tone="buy" toggleOutcome={() => toggleOutcome(m.condition_id, 0)}
                isExpanded={openOutcomes.has(`${m.condition_id}:0`)} />
              {/* Side B (outcome 1) */}
              <LadderCell o={o1} flashed={flashed1} side="B" cid={m.condition_id} sideLabel={labelB}
                tone="sell" toggleOutcome={() => toggleOutcome(m.condition_id, 1)}
                isExpanded={openOutcomes.has(`${m.condition_id}:1`)} />
            </div>
            {/* 펼침 — market expanded 면 MarketRow 의 outcomes 영역만 표시 */}
            {isMOpen && (
              <div className="px-2 pb-2 pt-1 bg-bg-panel/20">
                <MarketRowExpansion m={m} openOutcomes={openOutcomes}
                  toggleOutcome={toggleOutcome} flashIds={flashIds} />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function LadderCell({ o, flashed, side, cid, sideLabel, tone, toggleOutcome, isExpanded }: {
  o: PositionEntry | undefined;
  flashed: boolean | undefined;
  side: "A" | "B";
  cid: string;
  sideLabel: string;
  tone: "buy" | "sell";
  toggleOutcome: () => void;
  isExpanded: boolean;
}) {
  if (!o) {
    return <div className="text-[10px] text-text-faint italic">—</div>;
  }
  const toneClass = tone === "buy" ? "text-buy" : "text-sell";
  const bgClass = tone === "buy" ? "bg-buy/[0.03] hover:bg-buy/[0.08]" : "bg-sell/[0.03] hover:bg-sell/[0.08]";
  return (
    <button onClick={toggleOutcome}
      className={cn("rounded px-2 py-1 text-left transition-colors", bgClass,
        flashed && "ring-1 ring-buy/40")}>
      <div className="flex items-center gap-1.5 text-[10.5px]">
        <span className={cn("size-1.5 rounded-full shrink-0", tone === "buy" ? "bg-buy" : "bg-sell")} />
        <span className={cn("font-medium truncate", toneClass)} title={sideLabel}>
          {sideLabel.length > 14 ? sideLabel.slice(0, 14) + "…" : sideLabel}
        </span>
        {flashed && <Zap className="size-2.5 text-buy shrink-0" />}
      </div>
      <div className="flex items-center gap-2 mt-1 text-[10.5px]">
        <span className="num">{fmtN(o.size, 1)} sh</span>
        <span className="text-text-muted">@</span>
        <span className="num font-medium">{fmtPx(o.avg_price)}</span>
        <span className="num text-text-muted ml-auto">{fmt$(o.current_value, 0)}</span>
      </div>
    </button>
  );
}

/** MarketRow 의 expanded outcomes 영역 만 단독 렌더 — LadderPanel 에서 사용 */
function MarketRowExpansion({ m, openOutcomes, toggleOutcome, flashIds }: {
  m: MarketNode;
  openOutcomes: Set<string>; toggleOutcome: (cid: string, idx: number) => void;
  flashIds: Map<string, FlashInfo>;
}) {
  // first leg 계산
  const firstLegIdx = (() => {
    const withTs = m.outcomes.filter(o => o.first_fill_ts);
    if (withTs.length < 2) return null;
    const sorted = [...withTs].sort((a, b) =>
      new Date(a.first_fill_ts!).getTime() - new Date(b.first_fill_ts!).getTime());
    return sorted[0].outcome_index;
  })();
  // breakdown 가져옴
  const breakdownQ = useQuery({
    queryKey: ["breakdown", m.condition_id],
    queryFn: () => api.positionBreakdown(m.condition_id),
    enabled: m.outcomes.some(o => openOutcomes.has(`${m.condition_id}:${o.outcome_index}`)),
    staleTime: 45_000,
    refetchInterval: 60_000,
  });
  return (
    <>
      {m.outcomes.map(o => (
        <OutcomeLine key={o.outcome_index} o={o} cid={m.condition_id}
          flashInfo={flashIds.get(`${m.condition_id}:${o.outcome_index}`)}
          isFirstLeg={firstLegIdx === o.outcome_index}
          isOpen={openOutcomes.has(`${m.condition_id}:${o.outcome_index}`)}
          onToggle={() => toggleOutcome(m.condition_id, o.outcome_index)}
          levels={breakdownQ.data?.price_levels.filter(l => l.outcome_index === o.outcome_index) ?? []}
          isLoadingLevels={breakdownQ.isLoading || breakdownQ.isFetching}
          hasLevelsData={!!breakdownQ.data}
        />
      ))}
    </>
  );
}

function CategoryHeader({ cat, count, markets }: { cat: string; count: number; markets: MarketNode[] }) {
  const meta: Record<string, { label: string; color: string; icon: string }> = {
    "moneyline":    { label: "Moneyline",        color: "text-buy bg-buy/10 ring-buy/30",        icon: "🏆" },
    "ou-totals":    { label: "Over / Under",     color: "text-accent bg-accent/10 ring-accent/30", icon: "📊" },
    "spread":       { label: "Spread",           color: "text-warn bg-warn/10 ring-warn/30",      icon: "🎯" },
    "first-inning": { label: "First Inning",     color: "text-info bg-info/10 ring-info/30",      icon: "①" },
    "alt-spread":   { label: "Alt Spread",       color: "text-accent-2 bg-accent-2/10 ring-accent-2/30", icon: "🎯" },
    "other":        { label: "Other",            color: "text-text-muted bg-bg-soft ring-line",   icon: "·" },
  };
  const m = meta[cat] ?? meta["other"];
  // 그룹 단위 집계
  const total_value = markets.reduce((s, mk) => s + mk.current_value, 0);
  const total_cash  = markets.reduce((s, mk) => s + mk.cash_pnl,      0);
  // family_param 범위 표시 (O/U 6.5~11.5 등)
  const params = markets.map(mk => mk.family_param).filter(p => p) as string[];
  const paramRange = params.length > 1
    ? `${params[0]} ~ ${params[params.length - 1]}`
    : params[0] ?? null;

  return (
    <div className="flex items-center gap-2 px-2 py-1.5 mt-1 first:mt-0 text-[10.5px]">
      <span className={cn("inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium ring-1", m.color)}>
        <span className="text-[11px]">{m.icon}</span>
        {m.label}
        {count > 1 && <span className="ml-0.5 num font-bold">×{count}</span>}
        {paramRange && <span className="ml-1 num text-text-muted font-normal">{paramRange}</span>}
      </span>
      <span className="ml-auto text-text-dim num">
        {total_value > 0 ? `$${total_value.toFixed(0)}` : ""}
        {total_cash !== 0 && <span className={cn("ml-1.5", cls(total_cash))}>
          {total_cash > 0 ? "+" : ""}${total_cash.toFixed(0)}
        </span>}
      </span>
    </div>
  );
}

function EtaBadge({ eta }: { eta: number | null }) {
  if (eta == null) return null;
  if (eta < -3600 * 24) return null; // too old
  const abs = Math.abs(eta);
  const label = abs < 3600 ? `${Math.round(abs/60)}m`
              : abs < 86400 ? `${(abs/3600).toFixed(1)}h`
              : `${(abs/86400).toFixed(1)}d`;
  if (eta > 0 && eta < 3600) {
    return <span className="inline-flex items-center gap-1 text-[10px] text-warn font-medium"><Flame className="size-3 animate-pulse" /> game in {label}</span>;
  }
  if (eta > 0) {
    return <span className="inline-flex items-center gap-1 text-[10px] text-text-muted"><Timer className="size-3" /> in {label}</span>;
  }
  return <span className="text-[10px] text-text-dim">ended {label} ago</span>;
}

function MarketRow({ m, open, onToggle, openOutcomes, toggleOutcome, flashIds }: {
  m: MarketNode; open: boolean; onToggle: () => void;
  openOutcomes: Set<string>; toggleOutcome: (cid: string, idx: number) => void;
  flashIds: Map<string, FlashInfo>;
}) {
  const sizes = m.outcomes.map(o => o.size);
  const totalSize = sizes.reduce((a, b) => a + b, 0) || 1;
  const isTwoSided = m.outcomes.length === 2 && sizes.every(s => s > 0);

  // 첫 leg 식별 — 양면 마켓에서 어느 outcome 이 먼저 시작됐는지.
  // first_fill_ts 가 더 이른 쪽이 first leg.
  const firstLegIdx = useMemo(() => {
    if (!isTwoSided) return null;
    const withTs = m.outcomes.filter(o => o.first_fill_ts);
    if (withTs.length < 2) return null;
    const sorted = [...withTs].sort((a, b) =>
      new Date(a.first_fill_ts!).getTime() - new Date(b.first_fill_ts!).getTime());
    return sorted[0].outcome_index;
  }, [m.outcomes, isTwoSided]);

  // 가격대별 분해 — 부모 (PositionsPage) 에서 들어 올린 state. 한 outcome 이라도 펼쳐졌으면 fetch.
  const hasOpenOutcome = open && m.outcomes.some(o => openOutcomes.has(`${m.condition_id}:${o.outcome_index}`));
  const breakdownQ = useQuery({
    queryKey: ["breakdown", m.condition_id],
    queryFn: () => api.positionBreakdown(m.condition_id),
    enabled: hasOpenOutcome,
    staleTime: 45_000,
    refetchInterval: hasOpenOutcome ? 60_000 : false,
  });

  return (
    <div id={`m-${m.condition_id}`} className="rounded-md mb-1 transition-colors scroll-mt-4">
      <button onClick={onToggle}
        className="w-full flex items-center gap-2 px-3 py-2 hover:bg-bg-panel rounded-md text-left">
        <ChevronRight className={cn("size-3.5 text-text-dim transition-transform", open && "rotate-90")} />
        <div className="flex-1 min-w-0">
          <div className="text-[13px] truncate flex items-center gap-1.5">
            <span className="truncate">{m.title || m.condition_id.slice(0, 14)}</span>
            {m.neg_risk && <span className="chip-amber">neg</span>}
            {isTwoSided && <span className="chip-purple">two-sided</span>}
          </div>
          {/* 양 outcome 비중 진행바 (idx0=green / idx1=red) */}
          {m.outcomes.length === 2 && (
            <div className="flex h-1 mt-1.5 rounded-full overflow-hidden bg-line/40 max-w-[300px]">
              {m.outcomes.map(o => (
                <div key={o.outcome_index}
                  className={cn(o.outcome_index === 0 ? "bg-buy/60" : "bg-sell/60")}
                  style={{ width: `${(o.size / totalSize) * 100}%` }}
                />
              ))}
            </div>
          )}
          <div className="flex items-center gap-1 mt-1 text-[10px] text-text-dim flex-wrap">
            {m.outcomes.map(o => {
              const isFlash = flashIds.has(`${m.condition_id}:${o.outcome_index}`);
              const isFirst = firstLegIdx === o.outcome_index;
              return (
                <span key={o.outcome_index} className={cn(
                  "chip transition-colors",
                  o.outcome_index === 0 ? "chip-green" : "chip-red",
                  isFlash && "ring-1 ring-buy/70",
                  // 첫 leg — 좌측 accent 색 border (양면 마켓에서 어느 쪽이 먼저 시작됐는지 식별)
                  isFirst && "ring-1 ring-accent/70 ring-offset-1 ring-offset-bg-elevated",
                )}>
                  {isFirst && <span className="text-accent font-bold mr-0.5" title="이 leg 부터 시작">1ST</span>}
                  {isFlash && <Zap className="inline size-2.5 -mt-0.5 text-buy mr-0.5" />}
                  {o.outcome_name} · <span className="num">{fmtN(o.size, 1)}</span>
                  @ <span className="num">{fmtPx(o.avg_price)}</span>
                </span>
              );
            })}
          </div>
        </div>
        <div className="text-right shrink-0">
          <div className="num text-[13px] font-medium">{fmt$(m.current_value, 2)}</div>
          <div className={cn("num text-[10px]", cls(m.cash_pnl))}>
            {m.cash_pnl > 0 ? "▲" : m.cash_pnl < 0 ? "▼" : "·"} {fmt$(m.cash_pnl, 2)}
          </div>
        </div>
      </button>

      {open && (
        <div className="px-3 pb-2 pt-1">
          {m.outcomes.map(o => (
            <OutcomeLine key={o.outcome_index} o={o} cid={m.condition_id}
              flashInfo={flashIds.get(`${m.condition_id}:${o.outcome_index}`)}
              isFirstLeg={firstLegIdx === o.outcome_index}
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

function OutcomeLine({ o, cid, flashInfo, isFirstLeg, isOpen, onToggle, levels, isLoadingLevels, hasLevelsData }: {
  o: PositionEntry; cid: string; flashInfo: FlashInfo | undefined;
  isFirstLeg: boolean;
  isOpen: boolean; onToggle: () => void;
  levels: PriceLevel[];
  isLoadingLevels: boolean; hasLevelsData: boolean;
}) {
  const flashed = flashInfo != null;
  const priceC = flashInfo?.price != null ? Math.round(flashInfo.price * 100) : null;
  const usdc = flashInfo?.price != null ? flashInfo.delta * flashInfo.price : null;
  return (
    <div className={cn(
      "ml-4 rounded transition-colors relative",
      flashed && "bg-buy/10",
      isFirstLeg && "border-l-2 border-accent/60 pl-1",
    )}>
      <button
        onClick={onToggle}
        className="w-full py-1.5 px-2 text-[12px] grid grid-cols-12 gap-2 text-left hover:bg-bg-panel/60 rounded"
      >
        <div className="col-span-3 truncate flex items-center gap-1.5">
          <ChevronRight className={cn("size-3 text-text-dim shrink-0 transition-transform", isOpen && "rotate-90")} />
          <span className="text-text-dim mr-0.5">[{o.outcome_index}]</span>
          <span className={cn("font-medium truncate", flashed && "text-buy")}>{o.outcome_name}</span>
          {isFirstLeg && (
            <span className="text-[9.5px] font-bold text-accent shrink-0 px-1 py-0.5 rounded bg-accent/10 ring-1 ring-accent/40"
              title={o.first_fill_ts ? `이 leg 부터 시작 — ${new Date(o.first_fill_ts).toLocaleString()}` : "이 leg 부터 시작"}>
              1ST
            </span>
          )}
          {flashed && <Zap className="size-3 text-buy shrink-0" />}
        </div>
        <div className="col-span-2 num text-text-muted">size <span className="text-text">{fmtN(o.size, 2)}</span></div>
        <div className="col-span-2 num text-text-muted">avg <span className="text-text">{fmtPx(o.avg_price)}</span></div>
        <div className="col-span-2 num text-text-muted">cur <span className="text-text">{fmtPx(o.current_price)}</span></div>
        <div className="col-span-2 num text-text-muted">value <span className="text-text">{fmt$(o.current_value, 2)}</span></div>
        <div className="col-span-1 flex items-center justify-end gap-1">
          {o.redeemable && <span className="chip-green">R</span>}
          {o.mergeable && <span className="chip-purple">M</span>}
        </div>
      </button>

      {isOpen && (
        <PriceLevelTable
          levels={levels}
          isLoading={isLoadingLevels && !hasLevelsData}
          outcome={o}
          flashInfo={flashInfo}
          isFirstLeg={isFirstLeg}
        />
      )}
    </div>
  );
}

/** 가격대별 매수 분해 — RN1 이 한 outcome 에 대해 서로 다른 가격으로 몇 번
 *  매수했는지를 가격(¢) 별로 그룹화하여 표시. 가중 평균이 정확한지 시각적으로 검증.
 *  flashInfo 가 있으면 그 가격대 row 에 NEW 강조 — 잇는 위치에 추가 매수 식별.
 */
function PriceLevelTable({ levels, isLoading, outcome, flashInfo, isFirstLeg }: {
  levels: PriceLevel[]; isLoading: boolean; outcome: PositionEntry;
  flashInfo: FlashInfo | undefined;
  isFirstLeg: boolean;
}) {
  // 가중평균 재계산 — DB 의 avg_price 와 일치하는지 검증용
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

  // 진짜 per-row delta — 이전 fetch 의 total_size 와 비교.
  // 이전: flashInfo.delta (outcome 전체 합계) 를 모든 fresh row 에 똑같이 표시 → 오인
  // 지금: 각 (side, price_cents) 별로 size_now − size_prev 정확 계산
  const prevLevelsRef = useRef<Map<string, number>>(new Map());
  const rowDeltas = useMemo(() => {
    const out = new Map<string, number>();
    for (const l of levels) {
      const key = `${l.side}-${l.price_cents}`;
      const prev = prevLevelsRef.current.get(key);
      if (prev != null && l.total_size > prev + 0.01) {
        out.set(key, l.total_size - prev);
      }
    }
    return out;
  }, [levels]);
  useEffect(() => {
    const m = new Map<string, number>();
    for (const l of levels) {
      m.set(`${l.side}-${l.price_cents}`, l.total_size);
    }
    prevLevelsRef.current = m;
  }, [levels]);

  if (isLoading) {
    return (
      <div className="ml-7 mb-2 mr-2 px-3 py-2 text-[11px] text-text-muted bg-bg-panel/60 border-l-2 border-line rounded">
        가격대별 분해 로드 중…
      </div>
    );
  }
  if (levels.length === 0) {
    return (
      <div className="ml-7 mb-2 mr-2 px-3 py-2 text-[11px] text-text-muted bg-bg-panel/60 border-l-2 border-line rounded">
        이 outcome 에 기록된 fill 없음 (관측 시작 이전 매수일 수 있음)
      </div>
    );
  }

  // 가격대 정렬 — 가격 내림차순 (높은 가격 먼저)
  const sorted = [...levels].sort((a, b) => b.price_cents - a.price_cents);
  const maxSize = Math.max(...sorted.map(l => l.total_size));
  // flashInfo 가 있으면 그 사건 (=flashInfo.t) 시각 이후 fill 된 row 가 "잇는 위치에 새로 추가" 된 row.
  // 60초 윈도우로 last_ts 비교 (collector polling 지연 흡수).
  const flashThresholdMs = flashInfo ? flashInfo.t - 60_000 : null;
  // 이 leg 가 *최초로* 시작된 가격대 = MIN(first_ts) 인 row.
  // 사용자 가 "어디서 첫 매수" 인지 한눈에 식별 가능.
  const earliestFirstTs = sorted.reduce<number>((min, l) => {
    const t = new Date(l.first_ts).getTime();
    return t < min ? t : min;
  }, Number.POSITIVE_INFINITY);
  const firstFillCents = sorted.find(l => new Date(l.first_ts).getTime() === earliestFirstTs)?.price_cents;
  // outcome_index 로 색 분리 — idx 0 = green (yes/over/team1), idx 1 = red (no/under/team2).
  // RN1 이 BUY 만 해도 outcome 별로 색이 달라야 시각적으로 구분 가능.
  const isIdx0 = outcome.outcome_index === 0;
  const idxBar = isIdx0 ? "bg-buy/50" : "bg-sell/50";
  const idxChip = isIdx0 ? "bg-buy/15 text-buy" : "bg-sell/15 text-sell";
  const idxBorder = isIdx0 ? "border-buy/40" : "border-sell/40";
  const idxFreshBg = isIdx0 ? "bg-buy/10" : "bg-sell/10";
  const idxAccent = isIdx0 ? "text-buy" : "text-sell";
  const idxIconBg = isIdx0 ? "text-buy" : "text-sell";

  return (
    <div className={cn(
      "ml-7 mb-2 mr-2 bg-bg-panel/40 border-l-2 rounded-md overflow-hidden",
      // first leg → accent 색 border + 약간 진한 bg
      isFirstLeg ? "border-accent/70 bg-accent/[0.03]" : idxBorder,
    )}>
      {/* 표 헤더 — first leg 면 outcome 정보 + 1ST 배지 라벨 */}
      <div className="px-3 py-1.5 border-b border-line bg-bg-soft/40 flex items-center justify-between">
        <div className="flex items-center gap-1.5 text-[10.5px] text-text-muted">
          <span className={cn("size-1.5 rounded-full", isIdx0 ? "bg-buy" : "bg-sell")} />
          <span className="font-medium text-text">[{outcome.outcome_index}] {outcome.outcome_name}</span>
          {isFirstLeg && (
            <span className="ml-1 text-[9.5px] font-bold text-accent px-1 py-0.5 rounded bg-accent/15 ring-1 ring-accent/40"
              title={outcome.first_fill_ts ? `이 leg 가 ${new Date(outcome.first_fill_ts).toLocaleString()} 부터 시작` : "first leg"}>
              1ST LEG
            </span>
          )}
        </div>
        {outcome.first_fill_ts && (
          <span className="text-[9.5px] text-text-dim">
            첫 fill {relTime(outcome.first_fill_ts)} · 마지막 fill {outcome.last_fill_ts ? relTime(outcome.last_fill_ts) : "—"}
          </span>
        )}
      </div>

      <div className="grid grid-cols-[60px_50px_1fr_90px_80px_60px_80px] px-3 py-2 text-[10px] uppercase tracking-wider2 text-text-muted font-medium border-b border-line bg-bg-soft/40">
        <div>Price</div>
        <div>Side</div>
        <div>크기 (시각)</div>
        <div className="text-right">Shares</div>
        <div className="text-right">USDC</div>
        <div className="text-right">Fills</div>
        <div className="text-right">Last</div>
      </div>
      {sorted.map((l) => {
        const pct = (l.total_size / maxSize) * 100;
        const lastMs = new Date(l.last_ts).getTime();
        const isFresh = flashThresholdMs != null && lastMs >= flashThresholdMs;
        const isFirstFill = l.price_cents === firstFillCents;
        return (
          <div key={`${l.side}-${l.price_cents}`}
            className={cn(
              "grid grid-cols-[60px_50px_1fr_90px_80px_60px_80px] px-3 py-1.5 text-[11px] items-center border-b border-line/30 last:border-b-0 transition-colors hover:bg-bg-panel/40 relative",
              isFresh && idxFreshBg,
              // 최초 시작 가격대 — accent-2 (purple) tint + 좌측 ribbon
              isFirstFill && !isFresh && "bg-accent-2/[0.08]",
            )}
            title={isFirstFill ? `최초 매수 가격대 — ${new Date(l.first_ts).toLocaleString()} 부터` : undefined}>
            {/* 최초 가격대 ribbon (좌측 vertical bar) */}
            {isFirstFill && <span className="absolute left-0 top-0 bottom-0 w-[3px] bg-accent-2" />}
            <div className="num font-medium flex items-center gap-1">
              {isFresh && <Zap className={cn("size-3 shrink-0", idxIconBg)} />}
              {isFirstFill && (
                <span className="text-[9px] font-bold text-accent-2 px-1 rounded bg-accent-2/15 ring-1 ring-accent-2/40 leading-tight"
                  title="이 leg 의 최초 매수 가격대">
                  1st
                </span>
              )}
              <span className={cn(
                isFresh && idxAccent,
                isFirstFill && !isFresh && "text-accent-2 font-bold",
              )}>{l.price_cents}¢</span>
            </div>
            <div>
              <span className={cn("text-[10px] px-1.5 py-0.5 rounded", idxChip)}>{l.side}</span>
            </div>
            <div>
              <div className="h-1.5 rounded-full bg-line/30 overflow-hidden max-w-[260px]">
                <div className={cn("h-full", idxBar)}
                  style={{ width: `${pct}%` }} />
              </div>
            </div>
            <div className={cn("text-right num", isFresh && idxAccent)}>
              {fmtN(l.total_size, 1)}
              {(() => {
                const rd = rowDeltas.get(`${l.side}-${l.price_cents}`);
                return rd != null ? (
                  <span className={cn("ml-1 text-[10px] font-semibold", idxAccent)}
                    title={`이 가격대에 ${rd.toFixed(1)} shares 증가`}>+{fmtN(rd, 1)}</span>
                ) : null;
              })()}
            </div>
            <div className="text-right num text-text-muted">{fmt$(l.total_usdc, 2)}</div>
            <div className="text-right num text-text-muted">{l.fill_count}</div>
            <div className={cn(
              "text-right text-[10px]",
              isFresh ? idxAccent : isFirstFill ? "text-accent-2" : "text-text-dim",
            )}>
              {isFirstFill ? (
                <span title={`first ${relTime(l.first_ts)}`}>
                  {relTime(l.first_ts)} <span className="text-text-dim">→</span> {relTime(l.last_ts)}
                </span>
              ) : (
                relTime(l.last_ts)
              )}
            </div>
          </div>
        );
      })}

      {/* 가중평균 검증 푸터 — gross vs rebate-adjusted + MERGE settled detection */}
      {recalc.wavg != null && (() => {
        const delta = (recalc.wavg ?? 0) - (outcome.avg_price ?? 0);
        const absDelta = Math.abs(delta);
        const relPct = outcome.avg_price ? absDelta / outcome.avg_price * 100 : 0;
        // MERGE 감지 — 우리 누적 shares 가 현재 position size 보다 훨씬 큼 = 일부 정산됨
        const settledShares = recalc.totalSize - outcome.size;
        const settledPct = recalc.totalSize > 0 ? (settledShares / recalc.totalSize * 100) : 0;
        const hasMerge = settledShares > 50 && settledPct > 5;
        // 추정 settled cost — settled shares × (전체 avg 또는 cheap 매수 avg 가정)
        const settledCost = recalc.totalUsdc - (outcome.size * (outcome.avg_price ?? 0));

        // 분류
        const status: "match" | "rebate" | "merge" | "mismatch" =
          relPct < 0.5 ? "match"
          : hasMerge ? "merge"
          : relPct < 1.5 ? "rebate"
          : "mismatch";

        return (
          <div className="bg-bg-soft/40 border-t border-line">
            <div className="px-3 py-2 text-[10px] flex items-center gap-2.5 flex-wrap">
              <TrendingUp className="size-3 text-text-dim shrink-0" />
              <span className="text-text-muted">
                BUY 가중평균 재계산 <span className="text-text num font-medium">{recalc.wavg.toFixed(4)}</span>
                <span className="text-text-dim ml-1">({fmt$(recalc.totalUsdc, 2)} / {fmtN(recalc.totalSize, 1)})</span>
              </span>
              {outcome.avg_price != null && (
                <>
                  <span className="text-text-faint">·</span>
                  <span className="text-text-muted">
                    DB avg <span className="text-text num font-medium">{outcome.avg_price.toFixed(4)}</span>
                    {status === "match" && <span className="ml-1.5 text-buy font-medium">✓ 일치 (Δ {delta.toFixed(4)})</span>}
                    {status === "rebate" && (
                      <span className="ml-1.5 text-warn font-medium" title="0.5-1.5% 차이 — rebate adjustment 가설 범위">
                        ⚠ {relPct.toFixed(2)}% 차이 — rebate adj. 추정
                      </span>
                    )}
                    {status === "merge" && (
                      <span className="ml-1.5 text-accent font-medium" title="MERGE/REDEEM 으로 일부 shares 정산됨 — breakdown 은 lifetime fills, DB avg 는 잔여 shares 기준">
                        ⚡ {relPct.toFixed(1)}% — MERGE 흔적
                      </span>
                    )}
                    {status === "mismatch" && (
                      <span className="ml-1.5 text-sell font-medium">❌ {relPct.toFixed(2)}% 불일치 (Δ {delta.toFixed(4)})</span>
                    )}
                  </span>
                </>
              )}
              <span className="ml-auto text-[9.5px] text-text-dim italic">
                우리=lifetime gross / Polymarket=잔여 shares 기준
              </span>
            </div>
            {/* MERGE settled 정보 — significant 한 경우만 별도 표시 */}
            {hasMerge && (
              <div className="px-3 py-1.5 border-t border-line/40 bg-accent/[0.05] text-[10px] flex items-center gap-2 flex-wrap">
                <Layers className="size-3 text-accent shrink-0" />
                <span className="text-accent font-medium">MERGE/REDEEM 정산</span>
                <span className="text-text-muted">
                  <span className="num text-text font-medium">{fmtN(settledShares, 0)}</span> shares (
                  <span className="num text-text">{settledPct.toFixed(1)}%</span>) 페어링되어 사라짐 ·
                  추정 cost <span className="num text-text">{fmt$(settledCost, 0)}</span>
                  <span className="text-text-dim ml-1">
                    → cheap 매수 분 (avg ~{(settledCost / Math.max(settledShares,1)).toFixed(2)}) 이 MERGE 로 페어
                  </span>
                </span>
                <span className="ml-auto text-text-dim text-[9.5px]">
                  현재 잔여 {fmtN(outcome.size, 0)} shares @ {outcome.avg_price?.toFixed(4)} (높은 가격대)
                </span>
              </div>
            )}
          </div>
        );
      })()}
    </div>
  );
}

// ToastInfo — 새 매수 알림
type ToastInfo = { t: number; delta: number; price: number | null; outcome: string; title: string; eventSlug: string | null; cid: string; idx: number };

function ToastStack({ toasts, onDismiss, onClick }: {
  toasts: ToastInfo[]; onDismiss: (t: number) => void;
  onClick: (info: ToastInfo) => void;
}) {
  if (!toasts.length) return null;
  return (
    <div className="fixed top-4 right-6 z-50 flex flex-col gap-2 w-[360px] pointer-events-none">
      {toasts.map(t => <ToastItem key={t.t + ":" + t.cid + ":" + t.idx} t={t} onDismiss={onDismiss} onClick={onClick} />)}
    </div>
  );
}

function ToastItem({ t, onDismiss, onClick }: { t: ToastInfo; onDismiss: (id: number) => void; onClick: (info: ToastInfo) => void }) {
  const isYes = t.idx === 0;
  const priceC = t.price != null ? Math.round(t.price * 100) : null;
  const usdc = t.price != null ? t.delta * t.price : null;
  return (
    <div
      onClick={() => onClick(t)}
      title="클릭하면 해당 포지션으로 이동"
      className="pointer-events-auto surface border-buy/35 bg-gradient-to-br from-bg-elevated via-bg-elevated to-buy/[0.06] px-4 py-3 shadow-lift animate-slide-up cursor-pointer hover:border-buy/70 hover:shadow-glow transition-all group"
    >
      <div className="flex items-start gap-2.5">
        <div className="size-7 rounded-md bg-buy/15 border border-buy/30 flex items-center justify-center shrink-0">
          <ShoppingCart className="size-3.5 text-buy" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5 label-eyebrow">
            <Bell className="size-3 text-buy" />
            <span className="text-buy/90">RN1 새 매수 감지</span>
            <span className="ml-auto text-[9px] text-text-dim normal-case tracking-normal opacity-0 group-hover:opacity-100 transition-opacity">↗ 클릭</span>
          </div>
          <div className="text-[13px] text-text font-medium mt-1 truncate" title={t.title}>{t.title}</div>
          {t.eventSlug && t.eventSlug !== t.title && (
            <div className="text-[10px] text-text-dim truncate mt-0.5">↳ {t.eventSlug}</div>
          )}
          <div className="flex items-center gap-1.5 text-[11px] mt-2 flex-wrap">
            <span className={isYes ? "chip-green" : "chip-red"}>
              {t.outcome}{priceC != null ? ` ${priceC}¢` : ""}
            </span>
            <span className="text-text num font-medium">+{fmtN(t.delta, 1)} shares</span>
            {usdc != null && (
              <span className="text-text-muted num">≈ {fmt$(usdc, 2)}</span>
            )}
          </div>
        </div>
        <button
          onClick={(e) => { e.stopPropagation(); onDismiss(t.t); }}
          className="text-text-dim hover:text-text transition-colors shrink-0 -mt-0.5 -mr-0.5 p-1 rounded hover:bg-bg-panel"
          aria-label="dismiss">
          <X className="size-3.5" />
        </button>
      </div>
    </div>
  );
}

function Loading() {
  return (
    <div className="surface px-5 py-16 text-center">
      <div className="skeleton h-3 w-32 mx-auto mb-3" />
      <div className="text-[12px] text-text-muted">데이터 로드 중…</div>
    </div>
  );
}
function Empty({ msg }: { msg: string }) {
  return (
    <div className="surface px-5 py-16 text-center text-text-muted">
      <AlertCircle className="size-7 mx-auto mb-2.5 text-text-dim" />
      <div className="text-[13px]">{msg}</div>
    </div>
  );
}
