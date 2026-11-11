"use client";
import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { api, MarketDetail, MarketsDetailQuery, MarketsDetailSort,
  MakerMarket, MakerBehaviorQuery, MakerSort, EntryPhase, ActionRow,
  MidPriceResp, SportRow, LifecyclePatternsResp, FirstLegContextResp,
  RealizedPnlResp } from "@/lib/api";
import { fmt$, fmtN, fmtPx, cls, cn, relTime } from "@/lib/fmt";
import { Activity, Target, TrendingUp, BarChart3, Layers, ShieldOff, Zap, Droplets, Timer, Gauge, Flame, ExternalLink, Search, X, ArrowDownUp, Filter, Repeat, GitMerge, Coins, AlertTriangle, Crosshair, ScanLine, Trophy, Workflow, MapPin, CheckCircle2, CircleDollarSign } from "lucide-react";
import { ResponsiveContainer, ComposedChart, Bar, Line, XAxis, YAxis, Tooltip, CartesianGrid, ReferenceLine } from "recharts";
import { PageBar } from "@/components/pagebar";

type Scope = "recent" | "all";

export default function AnalysisPage() {
  const [scope, setScope] = useState<Scope>("recent");
  const q = useQuery({ queryKey: ["analysis", scope], queryFn: () => api.analysis(scope), refetchInterval: 60_000, staleTime: 50_000 });
  if (q.isLoading) return <div className="surface px-5 py-16 text-text-muted text-center text-[13px]">로드 중…</div>;
  if (q.isError || !q.data) return <div className="surface px-5 py-16 text-sell text-center text-[13px]">api 연결 실패</div>;
  const d = q.data;

  return (
    <div className="space-y-4">
      {/* 전략 시그너처 */}
      <section className="surface p-5">
        <SectionHead icon={<Target className="size-4 text-accent" />} title="전략 시그너처" />
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-[13px]">
          <Block icon={<Zap className="size-3.5 text-accent" />} title="진입">
            <div className="text-text">{d.strategy_signature.entry}</div>
          </Block>
          <Block icon={<Layers className="size-3.5 text-accent" />} title="청산">
            <div className="text-text">{d.strategy_signature.exit}</div>
          </Block>
          <Block icon={<TrendingUp className="size-3.5 text-buy" />} title="알파 원천">
            <ul className="text-text-muted text-[12px] space-y-1 list-disc pl-4">
              {d.strategy_signature.alpha_sources.map((s, i) => <li key={i}>{s}</li>)}
            </ul>
          </Block>
          <Block icon={<ShieldOff className="size-3.5 text-sell" />} title="비사용 (의도적)">
            <div className="text-text">{d.strategy_signature.non_used}</div>
          </Block>
          <Block icon={<BarChart3 className="size-3.5 text-accent-2" />} title="Universe">
            <div className="text-text">{d.strategy_signature.universe}</div>
          </Block>
          <Block icon={<Activity className="size-3.5 text-warn" />} title="타이밍 (p50)">
            <div className="text-text-muted text-[12px]">Entry lag: <span className="text-text num">{fmtN(d.strategy_signature.timing_p50.entry_lag_minutes / 1440, 1)}일</span></div>
            <div className="text-text-muted text-[12px]">Last trade → game: <span className="text-text num">{fmtN(d.strategy_signature.timing_p50.last_trade_minus_gameStart_min, 0)}분</span> (게임 후)</div>
          </Block>
        </div>
      </section>

      {/* 정산 P&L 검증 — Forensic baseline 재현 여부 */}
      <RealizedPnlSection />

      {/* Lifecycle pattern aggregate — "RN1 의 표준 진입 패턴" 추출 */}
      <LifecyclePatternsSection />

      {/* First-leg context — 첫 leg 의 cheaper/expensive 분류 + 양 leg 첫 가격 */}
      <FirstLegContextSection />

      {/* 메이커 행동 — 진입 타이밍 / 가격 추적 갱신 / inter-fill 빈도 */}
      <MakerBehaviorSection />

      {/* Mid-price tracking — fills × book_events JOIN 으로 maker/taker 분류 */}
      <MidPriceSection />

      {/* 종목별 (sport) 진입 패턴 브레이크다운 */}
      <PerSportSection />

      {/* MERGE / REDEEM / SPLIT 이력 — Forensic v0.1 의 62% net alpha (MERGE-lock) 검증 */}
      <ActionsRecentSection />

      {/* 데이터 커버리지 진단 — W4: 우리 fills 합계 vs API total_bought 격차 */}
      <CoverageSection />

      {/* 활동 마켓 상세 — 진입 결정용 메트릭 (현재 24h activity 한정) */}
      <MarketsDetailSection />

      {/* Forensic v0.1 — 21일 baseline */}
      <section className="surface p-5">
        <SectionHead icon={<BarChart3 className="size-4 text-accent-2" />} title="Forensic v0.1 baseline" note="2026-04-28 ~ 05-18, 21일" />
        <div className="grid grid-cols-2 md:grid-cols-6 gap-3">
          <ForensicKpi label="체결" value={fmtN(d.forensic_v0_1.trades as number, 0)} sub={`BUY ${d.forensic_v0_1.buy_pct}%`} />
          <ForensicKpi label="시장 / 이벤트" value={`${fmtN(d.forensic_v0_1.markets as number)}/${fmtN(3151)}`} sub={`two-sided ${d.forensic_v0_1.two_sided_pct}%`} />
          <ForensicKpi label="Gross Notional" value={fmt$(d.forensic_v0_1.gross_notional as number, 0)} />
          <ForensicKpi label="MERGE-lock" value={fmt$(d.forensic_v0_1.merge_locked as number, 0)} cls="text-buy" sub="62% of net" />
          <ForensicKpi label="Maker Rebate" value={fmt$(d.forensic_v0_1.maker_rebate as number, 0)} cls="text-buy" sub="19% of net · 0.75%×gross" />
          <ForensicKpi label="Net / day" value={fmt$(d.forensic_v0_1.daily_avg as number, 0)} cls="text-buy" sub={`총 ${fmt$(d.forensic_v0_1.net as number, 0)}`} />
        </div>
      </section>

      {/* 라이브 누적 — scope toggle 로 24h activity vs 전체 전환 */}
      <section className="surface p-5">
        <div className="flex items-center justify-between gap-2 mb-3">
          <SectionHead icon={<Activity className="size-4 text-buy" />} title="라이브 누적" note={d.scope_label || "전체"} />
          <div className="seg">
            {(["recent", "all"] as const).map(s => (
              <button key={s} onClick={() => setScope(s)}
                className={cn("seg-btn whitespace-nowrap", scope === s && "seg-btn-active")}>
                {s === "recent" ? "최근 24h" : "전체"}
              </button>
            ))}
          </div>
        </div>
        <div className="grid grid-cols-2 md:grid-cols-5 gap-3">
          <ForensicKpi label="현재 포지션" value={fmtN(d.live.position_legs)} sub="legs" />
          <ForensicKpi label="현재 가치" value={fmt$(d.live.current_value, 0)} />
          <ForensicKpi label="Cash PnL" value={fmt$(d.live.cash_pnl, 0)} cls={cls(d.live.cash_pnl)} />
          <ForensicKpi label="Realized PnL" value={fmt$(d.live.realized_pnl, 0)} cls={cls(d.live.realized_pnl)} />
          <ForensicKpi label="관찰 체결" value={fmtN(d.live.fills_observed)} sub={fmt$(d.live.gross_notional_observed, 0) + " gross"} />
        </div>
      </section>

      {/* 카테고리 별 (라이브 fills) */}
      {d.categories.length > 0 && (
        <section className="surface p-5">
          <SectionHead icon={<Layers className="size-4 text-info" />} title="카테고리 분포" note="라이브 fills" />
          <table className="w-full text-[12.5px]">
            <thead>
              <tr className="border-b border-line">
                <th className="th text-left">Slug prefix</th>
                <th className="th text-right">Fills</th>
                <th className="th text-right">Notional</th>
              </tr>
            </thead>
            <tbody>
              {d.categories.map(c => (
                <tr key={c.category} className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors">
                  <td className="td"><span className="chip-blue">{c.category}</span></td>
                  <td className="td text-right num">{fmtN(c.fills)}</td>
                  <td className="td text-right num">{fmt$(c.notional, 0)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </div>
  );
}

function SectionHead({ icon, title, note }: { icon: React.ReactNode; title: string; note?: string }) {
  return (
    <div className="flex items-center gap-2 mb-3.5">
      {icon}
      <h2 className="label-section">
        {title}
        {note && <span className="text-text-dim font-normal normal-case tracking-normal ml-2">({note})</span>}
      </h2>
    </div>
  );
}

// ============================================================================
// 메이커 행동 분석 — 사용자 가설 "RN1 은 진입 후 가격 변동에 맞춰 maker quote
// 를 실시간 갱신한다" 검증용. 마켓당:
//  · 첫 fill 시각 vs 게임 시작 (= 진입 phase)
//  · 가격 stddev / range / inter-fill 시간 (= 갱신 강도)
//  · 양면 여부 / 활성 분간 / gross
// ============================================================================
function MakerBehaviorSection() {
  const [sort, setSort] = useState<MakerSort>("gross");
  const [hours, setHours] = useState<number>(24);
  const [phaseFilter, setPhaseFilter] = useState<"all" | EntryPhase>("all");
  const [pageSize, setPageSize] = useState<number>(30);
  const [page, setPage] = useState<number>(1);
  useEffect(() => { setPage(1); }, [sort, hours, phaseFilter, pageSize]);

  const q = useQuery({
    queryKey: ["maker-behavior", { sort, hours, page, page_size: pageSize }],
    queryFn: () => api.makerBehavior({ sort, hours, page, page_size: pageSize }),
    refetchInterval: 60_000,
    staleTime: 50_000,
    placeholderData: keepPreviousData,
  });

  const d = q.data;
  const filtered = useMemo(() => {
    if (!d) return [];
    if (phaseFilter === "all") return d.markets;
    return d.markets.filter(m => m.entry_phase === phaseFilter);
  }, [d, phaseFilter]);

  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <Crosshair className="size-4 text-accent" />
          <h2 className="label-section">
            메이커 행동 시그너처 · 진입 타이밍 & 가격 추적 갱신
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              ({d?.total_markets ?? 0} markets · 최근 {hours}h)
            </span>
          </h2>
        </div>
        <div className="seg">
          {([1, 6, 24, 72] as const).map(h => (
            <button key={h} onClick={() => setHours(h)}
              className={cn("seg-btn whitespace-nowrap", hours === h && "seg-btn-active")}>
              {h < 24 ? `${h}h` : `${h/24}d`}
            </button>
          ))}
        </div>
      </div>

      {/* Aggregate KPIs */}
      {d && (
        <div className="px-5 py-3 border-b border-line bg-bg-soft/20 grid grid-cols-2 md:grid-cols-5 gap-3">
          <MbKpi label="Two-sided" value={`${d.aggregate.two_sided_pct.toFixed(0)}%`}
            sub="양 outcome 동시 매수" />
          <MbKpi label="평균 inter-fill" value={fmtInterFill(d.aggregate.avg_inter_fill_sec)}
            sub="quote 갱신 빈도 proxy" />
          <MbKpi label="평균 가격 σ" value={d.aggregate.avg_price_stddev.toFixed(4)}
            sub="가격 변동 폭" />
          <MbKpi label="평균 price range" value={d.aggregate.avg_price_range.toFixed(3)}
            sub="min↔max within market" />
          <MbKpi label="중앙 진입 시점" value={
            d.aggregate.median_entry_min_vs_game == null ? "—"
              : fmtEntryMin(d.aggregate.median_entry_min_vs_game)
          } sub="vs game_start (음수=전)" />
        </div>
      )}

      {/* Phase distribution chart */}
      {d && d.aggregate.phase_counts && (
        <div className="px-5 py-3 border-b border-line">
          <div className="label-eyebrow mb-2">진입 phase 분포</div>
          <PhaseBar counts={d.aggregate.phase_counts} selected={phaseFilter} onSelect={setPhaseFilter} />
        </div>
      )}

      {/* Toolbar */}
      <div className="px-5 py-3 border-b border-line flex flex-wrap items-center gap-3 bg-bg-soft/20">
        <label className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
          <span className="flex items-center gap-1"><ArrowDownUp className="size-3" /> 정렬</span>
          <select value={sort} onChange={e => setSort(e.target.value as MakerSort)} className="select">
            <option value="gross">Gross USDC ↓</option>
            <option value="fills">체결 수 ↓</option>
            <option value="entry">진입 이른 순 (게임 전)</option>
            <option value="spread">Price σ ↓ (변동 큼)</option>
            <option value="range">Price range ↓</option>
            <option value="update">갱신 빠른 순 (inter-fill ↑)</option>
          </select>
        </label>
        <label className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
          <span>페이지</span>
          <select value={pageSize} onChange={e => setPageSize(Number(e.target.value))} className="select">
            <option value="15">15</option>
            <option value="30">30</option>
            <option value="60">60</option>
            <option value="120">120</option>
          </select>
        </label>
        <div className="ml-auto text-[11px] text-text-muted">
          {filtered.length} 표시 / {d?.total_markets ?? 0} total
        </div>
      </div>

      {q.isLoading && !d ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : filtered.length === 0 ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">해당 조건의 마켓 없음</div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b border-line bg-bg-soft/30">
                <th className="th text-left">Market</th>
                <th className="th text-center">Phase</th>
                <th className="th text-right" title="첫 fill vs game_start (분)">진입</th>
                <th className="th text-right">활성</th>
                <th className="th text-right">체결</th>
                <th className="th text-right" title="평균 inter-fill 시간 — quote 갱신 빈도 proxy">갱신 ⏱</th>
                <th className="th text-right">Gross</th>
                <th className="th text-right" title="min ↔ max">Price 범위</th>
                <th className="th text-right" title="standard deviation">σ</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map(m => <MakerRow key={m.condition_id} m={m} />)}
            </tbody>
          </table>
        </div>
      )}

      {d && d.total_pages > 1 && (
        <div className="border-t border-line">
          <PageBar page={page} totalPages={d.total_pages} setPage={setPage} />
        </div>
      )}
    </section>
  );
}

function MbKpi({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div>
      <div className="label-eyebrow">{label}</div>
      <div className="text-[16px] font-semibold num tracking-tightish mt-1">{value}</div>
      {sub && <div className="text-[10px] text-text-dim mt-0.5">{sub}</div>}
    </div>
  );
}

function PhaseBar({ counts, selected, onSelect }: {
  counts: Record<string, number>;
  selected: "all" | EntryPhase;
  onSelect: (p: "all" | EntryPhase) => void;
}) {
  const order: EntryPhase[] = ["pre-game-1h+", "pre-game", "live-1h", "live-3h", "post-game", "no-meta"];
  const total = Object.values(counts).reduce((a, b) => a + b, 0) || 1;
  const colors: Record<EntryPhase, string> = {
    "pre-game-1h+": "bg-accent",
    "pre-game":     "bg-accent/70",
    "live-1h":      "bg-buy",
    "live-3h":      "bg-buy/60",
    "post-game":    "bg-warn/60",
    "no-meta":      "bg-line-strong",
  };
  const labels: Record<EntryPhase, string> = {
    "pre-game-1h+": "게임 1h+ 전",
    "pre-game":     "게임 직전",
    "live-1h":      "라이브 0-1h",
    "live-3h":      "라이브 1-3h",
    "post-game":    "종료 후",
    "no-meta":      "메타 없음",
  };
  return (
    <div className="space-y-1.5">
      <div className="flex h-2 rounded-full overflow-hidden bg-line/40">
        {order.map(p => {
          const n = counts[p] ?? 0;
          if (n === 0) return null;
          return <div key={p} className={cn("h-full", colors[p])} style={{ width: `${(n/total) * 100}%` }} title={`${labels[p]} · ${n}`} />;
        })}
      </div>
      <div className="flex flex-wrap items-center gap-2 text-[10.5px]">
        <button onClick={() => onSelect("all")}
          className={cn("chip transition-colors", selected === "all" ? "ring-1 ring-accent text-text" : "")}>
          전체 {total}
        </button>
        {order.map(p => {
          const n = counts[p] ?? 0;
          if (n === 0) return null;
          return (
            <button key={p} onClick={() => onSelect(selected === p ? "all" : p)}
              className={cn("chip inline-flex items-center gap-1.5 transition-colors",
                selected === p ? "ring-1 ring-accent text-text" : "")}>
              <span className={cn("inline-block size-1.5 rounded-full", colors[p])} />
              {labels[p]} <span className="num text-text">{n}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function MakerRow({ m }: { m: MakerMarket }) {
  const router = useRouter();
  const phaseColor: Record<EntryPhase, string> = {
    "pre-game-1h+": "chip-accent",
    "pre-game":     "chip-blue",
    "live-1h":      "chip-green",
    "live-3h":      "chip-green",
    "post-game":    "chip-amber",
    "no-meta":      "chip",
  };
  const phaseLabel: Record<EntryPhase, string> = {
    "pre-game-1h+": "pre+1h",
    "pre-game":     "pre",
    "live-1h":      "live ≤1h",
    "live-3h":      "live ≤3h",
    "post-game":    "post",
    "no-meta":      "—",
  };
  return (
    <tr
      onClick={() => router.push(`/rn1/market/${m.condition_id}`)}
      title="lifecycle replay 페이지로 이동"
      className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors cursor-pointer"
    >
      <td className="td">
        <div className="flex items-start gap-2.5 max-w-[360px]">
          {m.icon && <img src={m.icon} className="size-7 rounded-md object-cover shrink-0 bg-bg-panel ring-1 ring-line/60" alt="" loading="lazy" />}
          <div className="min-w-0">
            <div className="text-text text-[12px] truncate leading-snug">{m.title || m.condition_id.slice(0,14)}</div>
            <div className="flex items-center gap-1 mt-1 text-[10px] flex-wrap">
              {m.two_sided && <span className="chip-purple">two-sided</span>}
              {m.uma_status && <span className="chip">{m.uma_status}</span>}
            </div>
          </div>
        </div>
      </td>
      <td className="td text-center">
        <span className={phaseColor[m.entry_phase]}>{phaseLabel[m.entry_phase]}</span>
      </td>
      <td className="td text-right num">
        {m.first_vs_game_min != null ? (
          <span className={m.first_vs_game_min < 0 ? "text-accent" : "text-buy"}>
            {fmtEntryMin(m.first_vs_game_min)}
          </span>
        ) : <span className="text-text-dim">—</span>}
      </td>
      <td className="td text-right num text-text-muted">
        {m.active_minutes < 60 ? `${m.active_minutes.toFixed(0)}m` : `${(m.active_minutes/60).toFixed(1)}h`}
      </td>
      <td className="td text-right num">{fmtN(m.fills)}</td>
      <td className="td text-right num text-text-muted">{fmtInterFill(m.inter_fill_sec)}</td>
      <td className="td text-right num">{fmt$(m.gross_usdc, 0)}</td>
      <td className="td text-right num text-text-muted">
        {m.min_price.toFixed(2)}–{m.max_price.toFixed(2)}
      </td>
      <td className="td text-right num text-text-muted">{m.price_stddev.toFixed(4)}</td>
    </tr>
  );
}

function fmtEntryMin(m: number): string {
  const abs = Math.abs(m);
  const sign = m < 0 ? "-" : "+";
  if (abs < 60) return `${sign}${abs.toFixed(0)}m`;
  if (abs < 1440) return `${sign}${(abs/60).toFixed(1)}h`;
  return `${sign}${(abs/1440).toFixed(1)}d`;
}
function fmtInterFill(sec: number): string {
  if (!sec || !isFinite(sec)) return "—";
  if (sec < 1) return `${(sec*1000).toFixed(0)}ms`;
  if (sec < 60) return `${sec.toFixed(1)}s`;
  if (sec < 3600) return `${(sec/60).toFixed(1)}m`;
  return `${(sec/3600).toFixed(1)}h`;
}

// ============================================================================
// 정산 P&L 검증 — Forensic baseline +$75K/일 재현 여부 정량 분석
// ============================================================================
function RealizedPnlSection() {
  const [days, setDays] = useState<number>(30);
  const q = useQuery({
    queryKey: ["realized-pnl", days],
    queryFn: () => api.realizedPnl(days),
    refetchInterval: 60_000,
  });

  const d = q.data;
  // chart data + cumulative
  const chart = useMemo(() => {
    if (!d) return [];
    let cum = 0;
    return d.daily.map(row => {
      cum += row.day_net_est;
      return {
        day: row.day,
        merge:  row.merge_payout,
        redeem: row.redeem_payout,
        buy:    -row.gross_buy,  // negative for cost outflow
        rebate: row.rebate_est,
        net:    row.day_net_est,
        cum:    cum,
      };
    });
  }, [d]);

  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <CircleDollarSign className="size-4 text-buy" />
          <h2 className="label-section">
            정산 P&amp;L 검증
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              · Forensic baseline (+$75K/일) 재현 분석 · observed {d?.observed_days ?? 0}d / asked {days}d
            </span>
          </h2>
        </div>
        <div className="seg">
          {([7, 14, 30, 60] as const).map(h => (
            <button key={h} onClick={() => setDays(h)}
              className={cn("seg-btn", days === h && "seg-btn-active")}>
              {h}d
            </button>
          ))}
        </div>
      </div>

      {q.isLoading && !d ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !d ? (
        <div className="px-5 py-16 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : (
        <>
          {/* KPI strip */}
          <div className="px-5 py-3 border-b border-line bg-bg-soft/20 grid grid-cols-2 md:grid-cols-5 gap-3">
            <MbKpi label="총 MERGE 페이아웃" value={fmt$(d.totals.merge_payout, 0)}
              sub={`${fmtN(d.totals.merge_count)}건 정산`} />
            <MbKpi label="총 REDEEM 페이아웃" value={fmt$(d.totals.redeem_payout, 0)}
              sub={`${fmtN(d.totals.redeem_count)}건 정산`} />
            <MbKpi label="우리 관측 gross buy" value={fmt$(d.totals.gross_buy, 0)}
              sub={`${fmtN(d.totals.n_fills)} fills (collector 시작 이후만)`} />
            <MbKpi label="현재 보유분 MTM" value={fmt$(d.totals.current_holdings_mtm, 0)}
              sub="positions.current_value" />
            <MbKpi label="추정 maker rebate" value={fmt$(d.totals.rebate_estimate, 0)}
              sub={`maker share ${d.verification.maker_share_assumption_pct}% × ${d.verification.rebate_assumption_bps}bps`} />
          </div>

          {/* Forensic baseline 비교 — apples-to-apples 만, window 명시 */}
          <div className="px-5 py-3 border-b border-line">
            <div className="label-eyebrow mb-2 flex items-center justify-between">
              <span>Forensic baseline 대비 — 동일 단위 / 동일 윈도우 정규화</span>
              <span className="text-text-dim normal-case font-normal tracking-normal">
                payouts window <span className="text-text num">{d.actions_days.toFixed(1)}d</span> · fills window <span className="text-text num">{d.fills_days.toFixed(2)}d</span>
              </span>
            </div>
            <div className="overflow-x-auto">
              <table className="w-full text-[12px]">
                <thead>
                  <tr className="border-b border-line/60">
                    <th className="th text-left">Component</th>
                    <th className="th text-right">Forensic 21d</th>
                    <th className="th text-right">Forensic/day</th>
                    <th className="th text-right">관측 total</th>
                    <th className="th text-right">관측/day (window 정규화)</th>
                    <th className="th text-right">재현률</th>
                  </tr>
                </thead>
                <tbody>
                  {/* Gross notional — apples-to-apples, fills window 사용 */}
                  <ComparisonRow
                    label="Gross notional (BUY)"
                    baseline={d.forensic_baseline.gross_notional} baselineDays={21}
                    observed={d.totals.gross_buy} observedDaily={d.daily_averages.gross_buy}
                    note="fills window 정규화 (실시간 누적)" />
                  {/* Maker rebate — apples-to-apples */}
                  <ComparisonRow
                    label="Maker rebate (추정)"
                    baseline={d.forensic_baseline.maker_rebate} baselineDays={21}
                    observed={d.totals.rebate_estimate} observedDaily={d.daily_averages.rebate}
                    note={`gross × ${d.verification.maker_share_assumption_pct}% × ${d.verification.rebate_assumption_bps}bps`} />
                  {/* MERGE-locked — 추정 (gross payout × margin) */}
                  <ComparisonRow
                    label="MERGE-locked (profit, 추정)"
                    baseline={d.forensic_baseline.merge_locked} baselineDays={21}
                    observed={d.totals.merge_locked_est}
                    observedDaily={d.totals.merge_locked_est / Math.max(d.actions_days, 0.001)}
                    note={`gross MERGE × ${d.verification.merge_margin_assumption_pct}% margin 가정`}
                    isEstimate />
                  {/* Projected net — 위 세 가지 합으로 추정 */}
                  <tr className="border-t-2 border-line font-semibold">
                    <td className="td">예상 daily net (= rebate + MERGE-locked − residual loss buffer)</td>
                    <td className="td text-right num">{fmt$(d.forensic_baseline.net, 0)}</td>
                    <td className="td text-right num text-buy">{fmt$(d.forensic_baseline.daily_avg, 0)}</td>
                    <td className="td text-right num text-text-dim">—</td>
                    <td className="td text-right num text-warn">
                      {fmt$(d.daily_averages.projected_net, 0)}
                    </td>
                    <td className="td text-right num">
                      <span className={
                        d.verification.projected_net_vs_baseline_pct >= 70 && d.verification.projected_net_vs_baseline_pct <= 130 ? "text-buy"
                          : d.verification.projected_net_vs_baseline_pct >= 30 ? "text-warn" : "text-sell"}>
                        {d.verification.projected_net_vs_baseline_pct.toFixed(0)}%
                      </span>
                    </td>
                  </tr>
                  {/* REDEEM gross — caveat row, no 재현률 */}
                  <tr className="border-t border-line/40 text-text-dim">
                    <td className="td">REDEEM payout (gross)
                      <div className="text-[9.5px] italic">forensic 의 residual_pnl 은 *net profit* — 직접 비교 불가</div>
                    </td>
                    <td className="td text-right num">{fmt$(d.forensic_baseline.residual_pnl, 0)} <span className="text-[9.5px]">net</span></td>
                    <td className="td text-right num">—</td>
                    <td className="td text-right num">{fmt$(d.totals.redeem_payout, 0)} <span className="text-[9.5px]">gross</span></td>
                    <td className="td text-right num">{fmt$(d.daily_averages.redeem_payout, 0)} <span className="text-[9.5px]">gross/d</span></td>
                    <td className="td text-right text-[10px] italic">N/A</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <div className="mt-2 px-3 py-2 bg-warn/5 border border-warn/20 rounded text-[10.5px] text-text-muted leading-relaxed space-y-1">
              <div>⚠️ <strong>해석 가이드</strong></div>
              <div>· <strong>Gross notional / Maker rebate</strong> 는 우리 fills window ({d.fills_days.toFixed(2)}d) 로 정규화 → daily 비교 정확.</div>
              <div>· <strong>MERGE-locked</strong> 은 MERGE gross 페이아웃 × {d.verification.merge_margin_assumption_pct}% margin 추정. RN1 의 실제 pair sum_avg 측정 시 보정 필요.</div>
              <div>· <strong>REDEEM</strong> 은 forensic 의 residual_pnl (net profit) 과 우리 gross payout 이 다른 metric — 별도 표시.</div>
              <div>· <strong>예상 daily net</strong> = rebate_daily + merge_locked_daily − (loss buffer 미반영). forensic 의 $75K/일 와 비교한 재현률.</div>
              <div>· <strong>cost basis 누적 진행 중</strong>: fills window 가 {d.fills_days.toFixed(2)}d → 21d 로 자연 증가하면 더 정확.</div>
            </div>
          </div>

          {/* 일별 stacked bar — payouts vs gross_buy */}
          <div className="px-5 py-4 border-b border-line">
            <div className="label-eyebrow mb-2">일별 정산 활동 (payouts vs gross_buy)</div>
            <div className="h-[220px]">
              <ResponsiveContainer width="100%" height="100%">
                <ComposedChart data={chart} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                  <CartesianGrid stroke="#1f2935" strokeDasharray="2 4" vertical={false} />
                  <XAxis dataKey="day" tick={{ fill: "#7d8b9c", fontSize: 10 }}
                    tickFormatter={(t: string) => t.slice(5)} />
                  <YAxis tick={{ fill: "#7d8b9c", fontSize: 10 }}
                    tickFormatter={(v: number) => v.toLocaleString("en", { notation: "compact" })} />
                  <Tooltip
                    contentStyle={{ background: "#0a0e14", border: "1px solid #1f2935", borderRadius: 8, fontSize: 11 }}
                    formatter={(v: number, name: string) => [fmt$(Math.abs(v), 0), name === "merge" ? "MERGE" : name === "redeem" ? "REDEEM" : name === "buy" ? "BUY 비용" : name === "rebate" ? "Rebate" : "Net"]}
                  />
                  <ReferenceLine y={0} stroke="#1f2935" />
                  <Bar dataKey="merge" stackId="payout" fill="#9f7aea" />
                  <Bar dataKey="redeem" stackId="payout" fill="#22c55e" />
                  <Bar dataKey="buy" stackId="cost" fill="#ef4444" opacity={0.7} />
                  <Line type="monotone" dataKey="cum" stroke="#5bc0eb" strokeWidth={2} dot={false} />
                </ComposedChart>
              </ResponsiveContainer>
            </div>
            <div className="text-[10.5px] text-text-muted mt-2 flex items-center gap-3">
              <span className="inline-flex items-center gap-1"><span className="size-2 rounded-sm bg-accent-2" /> MERGE 페이아웃</span>
              <span className="inline-flex items-center gap-1"><span className="size-2 rounded-sm bg-buy" /> REDEEM 페이아웃</span>
              <span className="inline-flex items-center gap-1"><span className="size-2 rounded-sm bg-sell/70" /> BUY 비용</span>
              <span className="inline-flex items-center gap-1"><span className="size-2 rounded-sm bg-accent" /> 누적 net (line)</span>
            </div>
          </div>

          {/* Daily detail table */}
          <div className="overflow-x-auto">
            <table className="w-full text-[12px]">
              <thead>
                <tr className="border-b border-line bg-bg-soft/30">
                  <th className="th text-left">Day</th>
                  <th className="th text-right">MERGE 건</th>
                  <th className="th text-right">MERGE $</th>
                  <th className="th text-right">REDEEM 건</th>
                  <th className="th text-right">REDEEM $</th>
                  <th className="th text-right">Fills</th>
                  <th className="th text-right">BUY 비용 $</th>
                  <th className="th text-right">Rebate est $</th>
                  <th className="th text-right">Day net est</th>
                </tr>
              </thead>
              <tbody>
                {d.daily.slice().reverse().map(row => (
                  <tr key={row.day} className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors">
                    <td className="td num">{row.day}</td>
                    <td className="td text-right num text-text-muted">{fmtN(row.merge_n)}</td>
                    <td className="td text-right num">{fmt$(row.merge_payout, 0)}</td>
                    <td className="td text-right num text-text-muted">{fmtN(row.redeem_n)}</td>
                    <td className="td text-right num">{fmt$(row.redeem_payout, 0)}</td>
                    <td className="td text-right num text-text-muted">{fmtN(row.n_fills)}</td>
                    <td className="td text-right num text-sell">{fmt$(row.gross_buy, 0)}</td>
                    <td className="td text-right num text-buy">{fmt$(row.rebate_est, 0)}</td>
                    <td className="td text-right num">
                      <span className={row.day_net_est >= 0 ? "text-buy" : "text-sell"}>
                        {row.day_net_est >= 0 ? "+" : ""}{fmt$(row.day_net_est, 0)}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

function ComparisonRow({ label, baseline, baselineDays, observed, observedDaily, note, isEstimate }: {
  label: string;
  baseline: number;
  baselineDays: number;
  observed: number;
  observedDaily: number;
  note?: string;
  isEstimate?: boolean;
}) {
  const baseline_daily = baseline / baselineDays;
  const pct = baseline_daily > 0 ? (observedDaily / baseline_daily * 100) : 0;
  return (
    <tr className="border-b border-line/40">
      <td className="td">
        <span className="flex items-center gap-1.5">
          {label}
          {isEstimate && <span className="chip-amber text-[9px]">추정</span>}
        </span>
        {note && <div className="text-[9.5px] text-text-dim italic">{note}</div>}
      </td>
      <td className="td text-right num text-text-muted">{fmt$(baseline, 0)}</td>
      <td className="td text-right num">{fmt$(baseline_daily, 0)}</td>
      <td className="td text-right num text-text-muted">{fmt$(observed, 0)}</td>
      <td className="td text-right num">{fmt$(observedDaily, 0)}</td>
      <td className="td text-right num">
        <span className={pct >= 70 && pct <= 130 ? "text-buy" : pct >= 30 ? "text-warn" : "text-sell"}>
          {pct.toFixed(0)}%
        </span>
      </td>
    </tr>
  );
}

// ============================================================================
// Lifecycle pattern aggregate — 여러 마켓 분포로 "표준 패턴" 추출
// ============================================================================
function LifecyclePatternsSection() {
  const router = useRouter();
  const [hours, setHours] = useState<number>(24);
  const q = useQuery({
    queryKey: ["lifecycle-patterns", hours],
    queryFn: () => api.lifecyclePatterns(hours),
    refetchInterval: 60_000,
  });

  const d = q.data;
  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <Workflow className="size-4 text-accent" />
          <h2 className="label-section">
            Lifecycle 표준 패턴
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              ({d?.total_markets ?? 0} markets · {hours}h)
            </span>
          </h2>
        </div>
        <div className="seg">
          {([6, 24, 72, 168] as const).map(h => (
            <button key={h} onClick={() => setHours(h)}
              className={cn("seg-btn", hours === h && "seg-btn-active")}>
              {h < 24 ? `${h}h` : h < 168 ? `${h/24}d` : "7d"}
            </button>
          ))}
        </div>
      </div>

      {q.isLoading && !d ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !d ? (
        <div className="px-5 py-16 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : (
        <>
          {/* KPI strip */}
          <div className="px-5 py-3 border-b border-line bg-bg-soft/20 grid grid-cols-2 md:grid-cols-5 gap-3">
            <MbKpi label="Two-sided %" value={`${d.first_leg_distribution.two_sided_pct.toFixed(0)}%`}
              sub={`${d.first_leg_distribution.two_sided} / ${d.first_leg_distribution.single_leg_only + d.first_leg_distribution.two_sided}`} />
            <MbKpi label="First leg side" value={`${d.first_leg_distribution.outcome_0} : ${d.first_leg_distribution.outcome_1}`}
              sub={`idx 0 vs 1 (${((d.first_leg_distribution.outcome_0 / (d.first_leg_distribution.outcome_0 + d.first_leg_distribution.outcome_1 || 1)) * 100).toFixed(0)}%/${((d.first_leg_distribution.outcome_1 / (d.first_leg_distribution.outcome_0 + d.first_leg_distribution.outcome_1 || 1)) * 100).toFixed(0)}%)`} />
            <MbKpi label="Median time → 2nd leg" value={d.stats.median_time_to_second_min != null ? fmtDur(d.stats.median_time_to_second_min * 60) : "—"}
              sub={`n=${d.stats.n_with_t2s}`} />
            <MbKpi label="Median balance" value={d.stats.median_balance_ratio != null ? `${(d.stats.median_balance_ratio * 100).toFixed(0)}%` : "—"}
              sub="min/max leg ratio" />
            <MbKpi label="첫 leg avg-price" value={
              `${d.first_leg_classification["underdog (avg<0.45)"] ?? 0} / ${d.first_leg_classification["even (0.45-0.55)"] ?? 0} / ${d.first_leg_classification["favorite (avg>0.55)"] ?? 0}`
            } sub="underdog / even / favorite" />
          </div>

          {/* Two histograms side by side */}
          <div className="grid grid-cols-1 md:grid-cols-2 border-b border-line">
            <div className="px-5 py-4 border-r-0 md:border-r border-line">
              <div className="label-eyebrow mb-2">1st → 2nd leg 시간 분포</div>
              <HistogramBars data={d.time_to_second_buckets} color="bg-accent" />
            </div>
            <div className="px-5 py-4 border-t md:border-t-0 border-line">
              <div className="label-eyebrow mb-2">최종 balance 분포 (min/max ratio)</div>
              <HistogramBars data={d.balance_buckets} color="bg-accent-2" />
            </div>
          </div>

          {/* Representative markets table */}
          <div className="overflow-x-auto">
            <div className="px-5 py-2 label-eyebrow border-b border-line bg-bg-soft/20">
              대표 마켓 (gross 상위 30) — 클릭 시 lifecycle replay
            </div>
            <table className="w-full">
              <thead>
                <tr className="border-b border-line bg-bg-soft/30">
                  <th className="th text-left">Market</th>
                  <th className="th text-center">First leg</th>
                  <th className="th text-right">진입가</th>
                  <th className="th text-right">→ 2nd leg</th>
                  <th className="th text-right">Balance</th>
                  <th className="th text-right">Fills</th>
                  <th className="th text-right">Gross</th>
                </tr>
              </thead>
              <tbody>
                {d.representatives.map(r => (
                  <tr key={r.condition_id}
                      onClick={() => router.push(`/rn1/market/${r.condition_id}`)}
                      className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors cursor-pointer">
                    <td className="td">
                      <div className="text-text truncate max-w-[360px]">{r.title || r.condition_id.slice(0, 14)}</div>
                      {r.event_slug && <div className="text-[10px] text-text-dim truncate">{r.event_slug}</div>}
                    </td>
                    <td className="td text-center">
                      <span className={r.first_leg === 0 ? "chip-green" : r.first_leg === 1 ? "chip-red" : "chip"}>
                        [{r.first_leg ?? "?"}]
                      </span>
                    </td>
                    <td className="td text-right num">
                      {r.first_leg_price != null ? `${(r.first_leg_price * 100).toFixed(0)}¢` : "—"}
                    </td>
                    <td className="td text-right num text-text-muted">
                      {r.time_to_second_min != null ? fmtDur(r.time_to_second_min * 60) : <span className="text-warn">single</span>}
                    </td>
                    <td className="td text-right num">
                      {r.balance_ratio != null ? (
                        <span className={r.balance_ratio >= 0.7 ? "text-buy" : r.balance_ratio >= 0.3 ? "text-warn" : "text-sell"}>
                          {(r.balance_ratio * 100).toFixed(0)}%
                        </span>
                      ) : <span className="text-text-dim">—</span>}
                    </td>
                    <td className="td text-right num text-text-muted">{fmtN(r.total_fills)}</td>
                    <td className="td text-right num">{fmt$(r.total_usdc, 0)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

function HistogramBars({ data, color }: { data: { label: string; count: number }[]; color: string }) {
  const maxC = Math.max(1, ...data.map(d => d.count));
  return (
    <div className="space-y-1.5">
      {data.map(d => (
        <div key={d.label} className="grid grid-cols-[80px_1fr_50px] gap-2 items-center text-[11px]">
          <span className="text-text-muted text-right">{d.label}</span>
          <div className="h-3 bg-line/30 rounded-sm overflow-hidden">
            <div className={cn("h-full rounded-sm", color)} style={{ width: `${(d.count / maxC) * 100}%` }} />
          </div>
          <span className="num text-text">{d.count}</span>
        </div>
      ))}
    </div>
  );
}

// ============================================================================
// First-leg context — 첫 leg 의 cheaper/expensive + 양 leg 첫 가격 비교
// ============================================================================
function FirstLegContextSection() {
  const router = useRouter();
  const [hours, setHours] = useState<number>(24);
  const q = useQuery({
    queryKey: ["first-leg-context", hours],
    queryFn: () => api.firstLegContext(hours),
    refetchInterval: 60_000,
  });

  const d = q.data;
  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <MapPin className="size-4 text-accent-2" />
          <h2 className="label-section">
            First-leg 선택 분석
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              ({d?.total_markets ?? 0} markets · {hours}h)
            </span>
          </h2>
        </div>
        <div className="seg">
          {([6, 24, 72, 168] as const).map(h => (
            <button key={h} onClick={() => setHours(h)}
              className={cn("seg-btn", hours === h && "seg-btn-active")}>
              {h < 24 ? `${h}h` : h < 168 ? `${h/24}d` : "7d"}
            </button>
          ))}
        </div>
      </div>

      {q.isLoading && !d ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !d ? (
        <div className="px-5 py-16 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : (
        <>
          <div className="px-5 py-3 border-b border-line bg-bg-soft/20 grid grid-cols-2 md:grid-cols-4 gap-3">
            <MbKpi label="Cheaper 쪽 선택" value={`${d.first_leg_side_selection.cheaper_pct.toFixed(0)}%`}
              sub={`${d.first_leg_side_selection.cheaper_side} / ${d.first_leg_side_selection.cheaper_side + d.first_leg_side_selection.expensive_side}`} />
            <MbKpi label="Expensive 쪽 선택" value={`${d.first_leg_side_selection.expensive_side}`}
              sub={`${(100 - d.first_leg_side_selection.cheaper_pct).toFixed(0)}% (favorite 쪽 시작)`} />
            <MbKpi label="Single-leg only" value={fmtN(d.first_leg_side_selection.single_leg_only)}
              sub="2nd leg 진입 없음" />
            <MbKpi label="Median 2nd leg gap" value={d.second_leg_gap_median_min != null ? fmtDur(d.second_leg_gap_median_min * 60) : "—"}
              sub="첫 fill → 다른 leg 첫 fill" />
          </div>

          <div className="px-5 py-4 border-b border-line">
            <div className="label-eyebrow mb-2">첫 leg 진입 가격 분포 (10¢ buckets)</div>
            <HistogramBars data={d.first_leg_price_distribution} color="bg-accent" />
          </div>

          <div className="overflow-x-auto">
            <div className="px-5 py-2 label-eyebrow border-b border-line bg-bg-soft/20">
              마켓별 진입 순서 (게임 시작 대비 이른 순)
            </div>
            <table className="w-full">
              <thead>
                <tr className="border-b border-line bg-bg-soft/30">
                  <th className="th text-left">Market</th>
                  <th className="th text-center">1st leg</th>
                  <th className="th text-right">진입가</th>
                  <th className="th text-right">반대 leg 가</th>
                  <th className="th text-center">cheaper?</th>
                  <th className="th text-right">합 (binary check)</th>
                  <th className="th text-right">2nd leg gap</th>
                  <th className="th text-right">vs game</th>
                </tr>
              </thead>
              <tbody>
                {d.markets.map(m => (
                  <tr key={m.condition_id}
                      onClick={() => router.push(`/rn1/market/${m.condition_id}`)}
                      className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors cursor-pointer">
                    <td className="td">
                      <div className="flex items-start gap-2 max-w-[360px]">
                        {m.icon && <img src={m.icon} className="size-6 rounded shrink-0 bg-bg-panel" alt="" loading="lazy" />}
                        <div className="min-w-0">
                          <div className="text-text text-[12px] truncate">{m.title || m.condition_id.slice(0, 14)}</div>
                          {m.event_slug && <div className="text-[10px] text-text-dim truncate">{m.event_slug}</div>}
                        </div>
                      </div>
                    </td>
                    <td className="td text-center">
                      <span className={m.first_leg === 0 ? "chip-green" : "chip-red"}>[{m.first_leg}]</span>
                    </td>
                    <td className="td text-right num">{(m.self_first_price * 100).toFixed(0)}¢</td>
                    <td className="td text-right num text-text-muted">
                      {m.other_first_price != null ? `${(m.other_first_price * 100).toFixed(0)}¢` : "—"}
                    </td>
                    <td className="td text-center">
                      {m.first_leg_cheaper === true ? <span className="chip-green">cheaper</span>
                        : m.first_leg_cheaper === false ? <span className="chip-red">expensive</span>
                        : <span className="text-text-dim text-[10px]">—</span>}
                    </td>
                    <td className="td text-right num text-text-muted">
                      {m.implied_sum != null ? m.implied_sum.toFixed(3) : "—"}
                    </td>
                    <td className="td text-right num text-text-muted">
                      {m.gap_min != null ? fmtDur(m.gap_min * 60) : <span className="text-warn">—</span>}
                    </td>
                    <td className="td text-right num">
                      {m.secs_from_game_min != null ? (
                        <span className={m.secs_from_game_min < 0 ? "text-accent" : "text-buy"}>
                          {fmtEntryMin(m.secs_from_game_min)}
                        </span>
                      ) : <span className="text-text-dim">—</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

function fmtDur(sec: number): string {
  const abs = Math.abs(sec);
  if (abs < 60) return `${abs.toFixed(0)}s`;
  if (abs < 3600) return `${(abs/60).toFixed(1)}m`;
  if (abs < 86400) return `${(abs/3600).toFixed(1)}h`;
  return `${(abs/86400).toFixed(1)}d`;
}

// ============================================================================
// Mid-price tracking — RN1 fill 이 maker / taker / inside-spread 분류
// fills × book_events JOIN (asset_to_condition 매핑) 으로 정밀 검증
// ============================================================================
function MidPriceSection() {
  const [hours, setHours] = useState<number>(24);
  const q = useQuery({
    queryKey: ["mid-price", hours],
    queryFn: () => api.midPrice(hours, 5000),
    refetchInterval: 30_000,
  });

  const d = q.data;
  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <ScanLine className="size-4 text-accent" />
          <h2 className="label-section">
            Maker · Taker 정밀 분류
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              fills × book_events · {d?.fills_with_book ?? 0}/{d?.total_fills_scanned ?? 0} 매칭
            </span>
          </h2>
        </div>
        <div className="seg">
          {([1, 6, 24, 72] as const).map(h => (
            <button key={h} onClick={() => setHours(h)}
              className={cn("seg-btn", hours === h && "seg-btn-active")}>
              {h < 24 ? `${h}h` : `${h/24}d`}
            </button>
          ))}
        </div>
      </div>

      {q.isLoading && !d ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중… (LATERAL JOIN 무거움)</div>
      ) : !d ? (
        <div className="px-5 py-16 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : d.fills_with_book === 0 ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">
          book_events 와 매칭되는 fills 없음 — WS 데이터 누적 필요
        </div>
      ) : (
        <>
          {/* 핵심 KPI */}
          <div className="px-5 py-3 border-b border-line bg-bg-soft/20 grid grid-cols-2 md:grid-cols-4 gap-3">
            <MbKpi label="MAKER" value={`${d.classification.maker_pct.toFixed(1)}%`}
              sub={`${fmtN(d.classification.maker_bid)} fills · ${fmt$(d.gross_usdc.maker, 0)}`} />
            <MbKpi label="TAKER" value={`${d.classification.taker_pct.toFixed(1)}%`}
              sub={`${fmtN(d.classification.taker_ask)} fills · ${fmt$(d.gross_usdc.taker, 0)}`} />
            <MbKpi label="INSIDE" value={`${d.classification.inside_pct.toFixed(1)}%`}
              sub={`${fmtN(d.classification.inside_spread)} fills · ${fmt$(d.gross_usdc.inside, 0)}`} />
            <MbKpi label="추정 maker rebate" value={fmt$(d.estimated_rebate_usdc, 0)}
              sub={`maker gross × ${d.rebate_assumption_bps}bps`} />
          </div>

          {/* avg distance + distance distribution */}
          <div className="px-5 py-3 border-b border-line">
            <div className="flex items-center justify-between mb-2">
              <span className="label-eyebrow">Distance from mid 분포 (¢)</span>
              <span className="text-[11px] text-text-muted">
                avg <span className={cn("num", d.avg_distance_from_mid >= 0 ? "text-sell" : "text-buy")}>
                  {d.avg_distance_from_mid >= 0 ? "+" : ""}{(d.avg_distance_from_mid * 100).toFixed(2)}¢
                </span> {d.avg_distance_from_mid >= 0 ? " · 평균 ask 방향 (taker tilt)" : " · 평균 bid 방향 (maker tilt)"}
              </span>
            </div>
            <DistChart dist={d.distance_distribution} />
          </div>

          {/* top markets by gross with maker% */}
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr className="border-b border-line bg-bg-soft/30">
                  <th className="th text-left">Market</th>
                  <th className="th text-right">Fills</th>
                  <th className="th text-right">Maker</th>
                  <th className="th text-right">Maker %</th>
                  <th className="th text-right">Gross</th>
                </tr>
              </thead>
              <tbody>
                {d.top_markets_by_gross.map(m => (
                  <MidPriceMarketRow key={m.condition_id} m={m} />
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

function MidPriceMarketRow({ m }: { m: { condition_id: string; title: string | null; fills: number; maker_fills: number; maker_pct: number; gross_usdc: number } }) {
  const router = useRouter();
  return (
    <tr
      onClick={() => router.push(`/rn1/market/${m.condition_id}`)}
      title="lifecycle replay 페이지로 이동"
      className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors cursor-pointer">
      <td className="td">
        <div className="text-text truncate max-w-[420px]">{m.title || m.condition_id.slice(0, 14)}</div>
      </td>
      <td className="td text-right num">{fmtN(m.fills)}</td>
      <td className="td text-right num text-buy">{fmtN(m.maker_fills)}</td>
      <td className="td text-right num">
        <span className={m.maker_pct >= 70 ? "text-buy" : m.maker_pct >= 40 ? "text-warn" : "text-sell"}>
          {m.maker_pct.toFixed(0)}%
        </span>
      </td>
      <td className="td text-right num">{fmt$(m.gross_usdc, 0)}</td>
    </tr>
  );
}

function DistChart({ dist }: { dist: { cents_from_mid: number; count: number }[] }) {
  const maxCount = Math.max(1, ...dist.map(d => d.count));
  return (
    <div className="flex items-end h-[80px] gap-[3px]">
      {dist.map(d => {
        const h = (d.count / maxCount) * 100;
        const isCenter = d.cents_from_mid === 0;
        const isNegative = d.cents_from_mid < 0;
        return (
          <div key={d.cents_from_mid} className="flex-1 flex flex-col items-center gap-1 min-w-0" title={`${d.cents_from_mid >= 0 ? "+" : ""}${d.cents_from_mid}¢ — ${d.count} fills`}>
            <div className="w-full flex items-end h-[60px]">
              <div className={cn(
                "w-full rounded-t transition-all",
                isCenter ? "bg-accent" : isNegative ? "bg-buy/60" : "bg-sell/60"
              )} style={{ height: `${h}%` }} />
            </div>
            <div className={cn("text-[9px] num", isCenter ? "text-accent font-semibold" : "text-text-dim")}>
              {d.cents_from_mid >= 0 ? "+" : ""}{d.cents_from_mid}
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ============================================================================
// Per-sport breakdown — 종목별 진입 패턴
// ============================================================================
function PerSportSection() {
  const [hours, setHours] = useState<number>(24);
  const q = useQuery({
    queryKey: ["per-sport", hours],
    queryFn: () => api.perSport(hours),
    refetchInterval: 60_000,
  });

  const d = q.data;
  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <Trophy className="size-4 text-warn" />
          <h2 className="label-section">
            종목별 진입 패턴
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              ({d?.total_markets ?? 0} markets · {fmt$(d?.total_gross_usdc ?? 0, 0)} gross)
            </span>
          </h2>
        </div>
        <div className="seg">
          {([1, 6, 24, 72] as const).map(h => (
            <button key={h} onClick={() => setHours(h)}
              className={cn("seg-btn", hours === h && "seg-btn-active")}>
              {h < 24 ? `${h}h` : `${h/24}d`}
            </button>
          ))}
        </div>
      </div>

      {q.isLoading && !d ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !d ? (
        <div className="px-5 py-16 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : d.sports.length === 0 ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">해당 윈도우에 종목 데이터 없음</div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b border-line bg-bg-soft/30">
                <th className="th text-left">Sport</th>
                <th className="th text-right">Markets</th>
                <th className="th text-right">Fills</th>
                <th className="th text-right">Gross</th>
                <th className="th text-right">Two-sided</th>
                <th className="th text-right" title="첫 fill vs game_start (min)">진입 (avg)</th>
                <th className="th text-right" title="평균 inter-fill 시간">갱신</th>
                <th className="th text-right">Avg price</th>
                <th className="th text-right" title="가격 변동 폭">σ</th>
              </tr>
            </thead>
            <tbody>
              {d.sports.map(s => <SportRowComponent key={s.sport} s={s} totalGross={d.total_gross_usdc} />)}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function SportRowComponent({ s, totalGross }: { s: SportRow; totalGross: number }) {
  const sharePct = totalGross > 0 ? (s.gross_usdc / totalGross) * 100 : 0;
  return (
    <tr className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors">
      <td className="td">
        <div className="flex items-center gap-2">
          <span className="chip-blue uppercase">{s.sport}</span>
          <div className="h-1 w-16 bg-line/40 rounded-full overflow-hidden">
            <div className="h-full bg-accent" style={{ width: `${Math.min(sharePct, 100)}%` }} />
          </div>
          <span className="text-[10px] text-text-dim num">{sharePct.toFixed(0)}%</span>
        </div>
      </td>
      <td className="td text-right num">{fmtN(s.markets)}</td>
      <td className="td text-right num text-text-muted">{fmtN(s.fills)}</td>
      <td className="td text-right num">{fmt$(s.gross_usdc, 0)}</td>
      <td className="td text-right num">
        <span className={s.two_sided_pct >= 70 ? "text-buy" : s.two_sided_pct >= 40 ? "text-warn" : "text-text-muted"}>
          {s.two_sided_pct.toFixed(0)}%
        </span>
      </td>
      <td className="td text-right num">
        {s.avg_first_vs_game_min != null ? (
          <span className={s.avg_first_vs_game_min < 0 ? "text-accent" : "text-buy"}>
            {fmtEntryMin(s.avg_first_vs_game_min)}
          </span>
        ) : <span className="text-text-dim">—</span>}
      </td>
      <td className="td text-right num text-text-muted">{fmtInterFill(s.avg_inter_fill_sec)}</td>
      <td className="td text-right num text-text-muted">{s.avg_price_mean.toFixed(2)}</td>
      <td className="td text-right num text-text-muted">{s.avg_price_stddev.toFixed(4)}</td>
    </tr>
  );
}

// ============================================================================
// MERGE / REDEEM / SPLIT 이력 — 정산 페이아웃 추적
// ============================================================================
function ActionsRecentSection() {
  const q = useQuery({
    queryKey: ["actions-recent"],
    queryFn: api.actionsRecent,
    refetchInterval: 30_000,
  });

  return (
    <section className="surface p-5">
      <SectionHead icon={<GitMerge className="size-4 text-accent-2" />} title="MERGE / REDEEM / SPLIT 이력"
        note="정산 페이아웃 — Forensic v0.1 의 MERGE-lock 62% net alpha 검증" />
      {q.isLoading ? (
        <div className="py-8 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !q.data ? (
        <div className="py-8 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : (
        <>
          <div className="grid grid-cols-3 gap-3 mb-4">
            <ActionKpi label="MERGE" count={q.data.summary.merge_count} usdc={q.data.summary.merge_usdc}
              icon={<GitMerge className="size-3.5 text-accent-2" />} />
            <ActionKpi label="REDEEM" count={q.data.summary.redeem_count} usdc={q.data.summary.redeem_usdc}
              icon={<Coins className="size-3.5 text-buy" />} />
            <ActionKpi label="SPLIT" count={q.data.summary.split_count} usdc={q.data.summary.split_usdc}
              icon={<Repeat className="size-3.5 text-warn" />} />
          </div>
          {q.data.actions.length === 0 ? (
            <div className="px-3 py-8 text-center text-text-muted text-[12px] surface-2">
              아직 수집된 액션 없음 — actions_loop 가 5분 주기로 backfill 진행 중
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-[12.5px]">
                <thead>
                  <tr className="border-b border-line">
                    <th className="th text-left">Type</th>
                    <th className="th text-left">Market</th>
                    <th className="th text-right">Size (shares)</th>
                    <th className="th text-right">Realized USDC</th>
                    <th className="th text-right">When</th>
                    <th className="th text-center">Tx</th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.actions.slice(0, 30).map(a => <ActionRowItem key={a.action_id} a={a} />)}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}
    </section>
  );
}

function ActionKpi({ label, count, usdc, icon }: { label: string; count: number; usdc: number; icon: React.ReactNode }) {
  return (
    <div className="surface-2 p-3.5">
      <div className="flex items-center gap-1.5 label-eyebrow">{icon} {label}</div>
      <div className="text-[18px] font-semibold num tracking-tightish mt-1">{fmtN(count)}</div>
      <div className="text-[11px] text-text-muted mt-0.5 num">{fmt$(usdc, 0)} 정산</div>
    </div>
  );
}

function ActionRowItem({ a }: { a: ActionRow }) {
  const typeColor = a.action_type === "MERGE" ? "chip-purple"
    : a.action_type === "REDEEM" ? "chip-green" : "chip-amber";
  return (
    <tr className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors">
      <td className="td"><span className={typeColor}>{a.action_type}</span></td>
      <td className="td">
        <div className="text-text truncate max-w-[320px]">{a.title || a.condition_id.slice(0, 14)}</div>
        {a.event_slug && <div className="text-[10px] text-text-dim truncate">{a.event_slug}</div>}
      </td>
      <td className="td text-right num">{a.size != null ? fmtN(a.size, 1) : "—"}</td>
      <td className="td text-right num">{a.realized_usdc != null ? fmt$(a.realized_usdc, 2) : "—"}</td>
      <td className="td text-right text-[11px] text-text-muted">{relTime(a.triggered_at)}</td>
      <td className="td text-center">
        {a.tx_hash && (
          <a href={`https://polygonscan.com/tx/${a.tx_hash}`} target="_blank" rel="noopener"
            className="text-text-dim hover:text-accent transition-colors inline-block">
            <ExternalLink className="size-3" />
          </a>
        )}
      </td>
    </tr>
  );
}

// ============================================================================
// 데이터 커버리지 진단 — fills 합계 vs positions API total_bought
// ============================================================================
function CoverageSection() {
  const q = useQuery({
    queryKey: ["coverage"],
    queryFn: api.coverage,
    refetchInterval: 60_000,
  });

  const d = q.data;
  return (
    <section className="surface p-5">
      <SectionHead icon={<AlertTriangle className="size-4 text-warn" />} title="데이터 커버리지 진단"
        note="W4 — 우리 fills 합계 vs Polymarket positions API의 total_bought" />
      {q.isLoading ? (
        <div className="py-8 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !d ? (
        <div className="py-8 text-center text-sell text-[13px]">API 연결 실패</div>
      ) : (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-4">
            <MbKpi label="전체 커버리지" value={`${d.overall_coverage_pct.toFixed(1)}%`}
              sub="observed / api_total_bought" />
            <MbKpi label="≥90% 커버 마켓" value={`${d.covered_markets} / ${d.total_positions}`}
              sub="누락 적은 포지션" />
            <MbKpi label="0 fills 관측" value={fmtN(d.zero_observed)}
              sub="관측 시작 이전 매수" />
            <MbKpi label="API total_bought" value={fmtN(d.total_api_bought, 0)}
              sub={`vs 우리 ${fmtN(d.total_observed_shares, 0)} shares`} />
          </div>

          {d.gap_markets.length > 0 ? (
            <div className="surface-2 overflow-hidden">
              <div className="px-4 py-2 label-eyebrow bg-bg-soft/40 border-b border-line">
                상위 갭 마켓 (절대 누락 큰 순 · 최대 50)
              </div>
              <div className="overflow-x-auto max-h-[420px] overflow-y-auto">
                <table className="w-full text-[12px]">
                  <thead className="sticky top-0 bg-bg-elevated">
                    <tr className="border-b border-line">
                      <th className="th text-left">Market</th>
                      <th className="th text-right">API bought</th>
                      <th className="th text-right">관측 shares</th>
                      <th className="th text-right">Gap</th>
                      <th className="th text-right">Coverage</th>
                      <th className="th text-right">관측 fills</th>
                      <th className="th text-right">First obs</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.gap_markets.slice(0, 25).map(g => (
                      <tr key={`${g.condition_id}-${g.outcome_index}`}
                        className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors">
                        <td className="td">
                          <div className="text-text truncate max-w-[280px]">{g.title || g.condition_id.slice(0,14)}</div>
                          <div className="text-[10px] text-text-dim">[{g.outcome_index}] {g.outcome_name}</div>
                        </td>
                        <td className="td text-right num">{fmtN(g.api_total_bought, 0)}</td>
                        <td className="td text-right num text-text-muted">{fmtN(g.observed_shares, 0)}</td>
                        <td className="td text-right num text-warn">{fmtN(g.gap_shares, 0)}</td>
                        <td className="td text-right num">
                          <span className={cn(g.coverage_pct < 50 ? "text-sell" : g.coverage_pct < 90 ? "text-warn" : "text-buy")}>
                            {g.coverage_pct.toFixed(0)}%
                          </span>
                        </td>
                        <td className="td text-right num text-text-muted">{fmtN(g.observed_fills)}</td>
                        <td className="td text-right text-[11px] text-text-muted">
                          {g.first_observed ? relTime(g.first_observed) : "—"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          ) : (
            <div className="text-center text-text-muted text-[12px] py-4">
              ≥50 shares 갭 마켓 없음 — 커버리지 양호
            </div>
          )}
        </>
      )}
    </section>
  );
}

function MarketsDetailSection() {
  const [sort, setSort] = useState<MarketsDetailSort>("activity");
  const [stateFilter, setStateFilter] = useState<"all" | "upcoming" | "live" | "post">("all");
  const [pageSize, setPageSize] = useState<number>(25);
  const [page, setPage] = useState<number>(1);
  const [q, setQ] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(q), 250);
    return () => clearTimeout(t);
  }, [q]);
  useEffect(() => { setPage(1); }, [sort, stateFilter, debouncedQ, pageSize]);

  const params: MarketsDetailQuery = { sort, state: stateFilter, page, page_size: pageSize, q: debouncedQ || undefined };
  const md = useQuery({
    queryKey: ["markets-detail", params],
    queryFn: () => api.marketsDetail(params),
    refetchInterval: 60_000,
    staleTime: 50_000,
    placeholderData: keepPreviousData,
  });

  const markets = md.data?.markets ?? [];
  const totalCount = md.data?.market_count ?? 0;
  const totalPages = md.data?.total_pages ?? 1;
  const showingFrom = totalCount === 0 ? 0 : (page - 1) * pageSize + 1;
  const showingTo = Math.min(page * pageSize, totalCount);

  return (
    <section className="surface-flush">
      <div className="px-5 py-4 border-b border-line flex items-center justify-between flex-wrap gap-3">
        <div className="flex items-center gap-2">
          <Droplets className="size-4 text-info" />
          <h2 className="label-section">
            활동 마켓 상세 · 진입 메트릭
            <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
              ({totalCount > 0 ? `${fmtN(showingFrom)}–${fmtN(showingTo)} / ${fmtN(totalCount)}` : "0"} markets · 24h 활동)
            </span>
          </h2>
        </div>
      </div>

      {/* Toolbar */}
      <div className="px-5 py-3 border-b border-line flex flex-wrap items-center gap-3 bg-bg-soft/20">
        <div className="relative flex-1 min-w-[220px]">
          <Search className="size-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-text-dim pointer-events-none" />
          <input
            placeholder="시장 / 이벤트 검색"
            value={q} onChange={e => setQ(e.target.value)}
            className="input pl-9 pr-8"
          />
          {q && (
            <button onClick={() => setQ("")} className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text transition-colors" aria-label="검색 지우기">
              <X className="size-3.5" />
            </button>
          )}
        </div>
        <MdSelect label={<><ArrowDownUp className="size-3" /> 정렬</>} value={sort} onChange={setSort} options={[
          ["activity",     "활동 강도 ↓"],
          ["fills_asc",    "활동 낮은 순 ↑"],
          ["spread",       "Spread 좁은 순 ↑"],
          ["value",        "Cur Value ↓"],
          ["volume",       "Volume ↓"],
          ["game",         "Game ETA 가까운 순"],
          ["slippage_abs", "Slippage |크기| ↓"],
          ["price_range",  "Price range ↓"],
        ]} />
        <MdSelect label={<><Filter className="size-3" /> Game</>} value={stateFilter} onChange={setStateFilter} options={[
          ["all",      "전체"],
          ["upcoming", "예정"],
          ["live",     "진행 중"],
          ["post",     "종료 후"],
        ]} />
        <MdSelect label="페이지" value={String(pageSize)} onChange={v => setPageSize(Number(v))} options={[
          ["10", "10/page"], ["25", "25/page"], ["50", "50/page"], ["100", "100/page"], ["200", "200/page"],
        ]} />
      </div>

      {/* 헤더 PageBar */}
      {totalPages > 1 && (
        <div className="border-b border-line">
          <PageBar page={page} totalPages={totalPages} setPage={setPage}
            totalLabel={`${fmtN(totalCount)} markets`} />
        </div>
      )}

      {md.isLoading && !md.data ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : markets.length === 0 ? (
        <div className="px-5 py-16 text-center text-text-muted text-[13px]">
          {debouncedQ ? `"${q}" 검색 결과 없음` : "조건에 맞는 활동 마켓 없음"}
        </div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b border-line bg-bg-soft/30">
                <th className="th text-left">Market</th>
                <th className="th text-center">Game</th>
                <th className="th text-right" title="Bid - Ask">Bid / Ask</th>
                <th className="th text-right">Spread</th>
                <th className="th text-right" title="market 24h volume">Volume</th>
                <th className="th text-right" title="24h RN1 fills + gross">RN1 24h</th>
                <th className="th text-right" title="RN1 평균 진입가 vs current mid">Slippage</th>
                <th className="th text-right">Cur Value</th>
                <th className="th text-right">Tick / Max</th>
              </tr>
            </thead>
            <tbody>
              {markets.map(m => <DetailRow key={m.condition_id} m={m} />)}
            </tbody>
          </table>
        </div>
      )}

      {/* 푸터 PageBar */}
      {totalPages > 1 && (
        <div className="border-t border-line">
          <PageBar page={page} totalPages={totalPages} setPage={setPage} />
        </div>
      )}
    </section>
  );
}

function MdSelect({ label, value, onChange, options }: {
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

function DetailRow({ m }: { m: MarketDetail }) {
  const router = useRouter();
  const evLink = m.event_slug ? `https://polymarket.com/event/${m.event_slug}` : null;
  const slip = m.slippage_pct;
  const slipColor = slip == null ? "text-text-muted"
                  : Math.abs(slip) < 1 ? "text-text"
                  : slip > 0 ? "text-sell"
                  : "text-buy";
  const fillsBadge = activityBadge(m.fills_24h);
  return (
    <tr
      onClick={(e) => {
        // 외부 링크 (a 태그) 클릭은 stopPropagation 으로 분리
        if ((e.target as HTMLElement).closest("a")) return;
        router.push(`/rn1/market/${m.condition_id}`);
      }}
      title="lifecycle replay 페이지로 이동"
      className="border-b border-line/40 hover:bg-bg-panel/40 transition-colors cursor-pointer">
      <td className="td">
        <div className="flex items-start gap-2.5 max-w-[420px]">
          {m.icon && <img src={m.icon} className="size-8 rounded-md object-cover shrink-0 bg-bg-panel ring-1 ring-line/60" alt="" loading="lazy" />}
          <div className="min-w-0">
            <div className="text-text text-[12.5px] truncate leading-snug">
              {evLink ? <a href={evLink} target="_blank" rel="noopener" onClick={e => e.stopPropagation()} className="hover:underline hover:text-accent transition-colors">{m.title || m.condition_id.slice(0,14)}</a> : m.title}
            </div>
            <div className="flex items-center gap-1 mt-1 text-[10px] flex-wrap">
              {m.neg_risk && <span className="chip-amber">neg-risk</span>}
              {m.uma_status && <span className="chip">{m.uma_status}</span>}
              <span className="chip-blue">{fillsBadge.dots} {fillsBadge.label}</span>
            </div>
          </div>
        </div>
      </td>
      <td className="td text-center">
        <GameBadge state={m.game_state} eta={m.game_eta_sec} />
      </td>
      <td className="td text-right num">
        {m.best_bid != null && m.best_ask != null ? (
          <div className="leading-tight">
            <div className="text-buy text-[11px]">{fmtPx(m.best_bid)}</div>
            <div className="text-sell text-[11px]">{fmtPx(m.best_ask)}</div>
          </div>
        ) : <span className="text-text-dim">—</span>}
      </td>
      <td className="td text-right num">
        {m.spread != null ? (
          <div className="leading-tight">
            <div className="text-text">{fmtPx(m.spread)}</div>
            <div className="text-[10px] text-text-muted">{m.spread_bps ? `${m.spread_bps.toFixed(0)} bps` : ""}</div>
          </div>
        ) : <span className="text-text-dim">—</span>}
      </td>
      <td className="td text-right num">
        {m.volume_num != null ? fmt$(m.volume_num, 0) : "—"}
      </td>
      <td className="td text-right num">
        <div className="leading-tight">
          <div className="text-text">{fmtN(m.fills_24h)} <span className="text-text-dim font-sans text-[10px]">fills</span></div>
          <div className="text-[10px] text-text-muted">{fmt$(m.gross_24h, 0)}</div>
        </div>
      </td>
      <td className={cn("td text-right num", slipColor)}>
        {slip != null ? (
          <div className="leading-tight">
            <div className="font-medium">{(slip > 0 ? "+" : "") + slip.toFixed(2)}%</div>
            {m.rn1_avg_entry != null && (
              <div className="text-[10px] text-text-muted">entry {fmtPx(m.rn1_avg_entry)}</div>
            )}
          </div>
        ) : "—"}
      </td>
      <td className="td text-right num">
        <div className="leading-tight">
          <div className="text-text">{fmt$(m.rn1_current_value, 0)}</div>
          {m.rn1_total_size > 0 && <div className="text-[10px] text-text-muted">{fmtN(m.rn1_total_size, 0)} sh</div>}
        </div>
      </td>
      <td className="td text-right text-[10px] text-text-muted num">
        <div className="leading-tight">
          <div>{m.tick_size != null ? `tick ${m.tick_size}` : "—"}</div>
          <div>{m.rewards_max_spread != null ? `max ${m.rewards_max_spread}` : ""}</div>
        </div>
      </td>
    </tr>
  );
}

function activityBadge(fills24: number): { dots: string; label: string } {
  if (fills24 >= 50) return { dots: "●●●●", label: "very high" };
  if (fills24 >= 20) return { dots: "●●●", label: "high" };
  if (fills24 >= 5)  return { dots: "●●", label: "medium" };
  if (fills24 >= 1)  return { dots: "●", label: "low" };
  return { dots: "○", label: "idle" };
}

function GameBadge({ state, eta }: { state: string; eta: number | null }) {
  if (eta == null) return <span className="text-text-dim text-[11px]">—</span>;
  if (state === "live") {
    return (
      <span className="inline-flex items-center gap-1 text-[11px] text-buy">
        <Flame className="size-3 animate-pulse" /> LIVE
        <span className="text-text-muted">+{Math.abs(Math.round(eta/60))}m</span>
      </span>
    );
  }
  if (state === "upcoming") {
    const abs = Math.abs(eta);
    const label = abs < 3600 ? `${Math.round(abs/60)}m`
                : abs < 86400 ? `${(abs/3600).toFixed(1)}h`
                : `${(abs/86400).toFixed(1)}d`;
    return (
      <span className="inline-flex items-center gap-1 text-[11px] text-warn">
        <Timer className="size-3" /> in {label}
      </span>
    );
  }
  // post
  return (
    <span className="inline-flex items-center gap-1 text-[11px] text-text-dim">
      ended {Math.round(Math.abs(eta)/60)}m
    </span>
  );
}

function Block({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return (
    <div className="surface-2 p-3.5">
      <div className="flex items-center gap-1.5 label-eyebrow mb-2">{icon} {title}</div>
      {children}
    </div>
  );
}
function ForensicKpi({ label, value, sub, cls }: { label: string; value: string; sub?: string; cls?: string }) {
  return (
    <div className="surface-2 p-3.5">
      <div className="label-eyebrow">{label}</div>
      <div className={`text-[18px] font-semibold tracking-tightish num mt-1 ${cls ?? ""}`}>{value}</div>
      {sub && <div className="text-[10px] text-text-dim mt-0.5">{sub}</div>}
    </div>
  );
}
