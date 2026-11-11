"use client";
import { useQuery } from "@tanstack/react-query";
import { useParams } from "next/navigation";
import Link from "next/link";
import { useMemo } from "react";
import { api, LifecycleFill, LifecycleResp, QuoteTimelineResp } from "@/lib/api";
import { fmt$, fmtN, fmtPx, cn, relTime } from "@/lib/fmt";
import { ArrowLeft, Clock, Flag, Layers, Activity, Zap, ExternalLink, GitMerge, Crosshair, Timer, BookOpen } from "lucide-react";
import {
  ResponsiveContainer, LineChart, Line, XAxis, YAxis, Tooltip, ReferenceLine,
  ScatterChart, Scatter, ZAxis, CartesianGrid, ComposedChart, Area,
} from "recharts";

/** Single market lifecycle replay — RN1 가 이 시장의 어떤 leg 부터, 언제, 어떻게
 *  two-side 를 완성해 나가는지 마이크로 시계열. */
export default function MarketLifecyclePage() {
  const params = useParams();
  const cid = (params?.cid as string) ?? "";

  const q = useQuery({
    queryKey: ["lifecycle", cid],
    queryFn: () => api.marketLifecycle(cid),
    enabled: !!cid,
    refetchInterval: 30_000,
  });

  if (q.isLoading) return <div className="surface px-5 py-16 text-center text-text-muted text-[13px]">로드 중…</div>;
  if (q.isError || !q.data) return <div className="surface px-5 py-16 text-center text-sell text-[13px]">api 연결 실패</div>;

  const d = q.data;
  const m = d.meta;
  const fills = d.fills;
  if (fills.length === 0) {
    return (
      <div className="space-y-3">
        <BackButton />
        <div className="surface px-5 py-16 text-center text-text-muted text-[13px]">
          이 마켓에 관측된 fills 없음
        </div>
      </div>
    );
  }

  const outcomes: string[] = Array.isArray(m.outcomes) ? m.outcomes as string[]
    : (m.outcomes && typeof m.outcomes === "string" ? (() => { try { return JSON.parse(m.outcomes as unknown as string); } catch { return ["", ""]; } })() : ["", ""]);

  // 차트 데이터 — 시간 vs 누적 shares (두 라인, 색별)
  const chartData = useMemo(() => fills.map(f => ({
    ts: new Date(f.ts).getTime(),
    cum0: f.cum_shares_0,
    cum1: f.cum_shares_1,
    balance: f.balance_ratio * 100,
    price: f.price * 100,  // ¢
    outcome: f.outcome_index,
  })), [fills]);

  const firstFillTs = new Date(fills[0].ts).getTime();
  const lastFillTs = new Date(fills[fills.length - 1].ts).getTime();
  const gameStartTs = m.game_start ? new Date(m.game_start).getTime() : null;

  // First leg 라벨
  const firstLeg = d.lifecycle.first_leg;
  const firstLegName = firstLeg != null ? (outcomes[firstLeg] ?? `[${firstLeg}]`) : "—";
  const timeToSecondMin = d.lifecycle.time_to_second_leg_min;

  return (
    <div className="space-y-3">
      <BackButton />

      {/* Header */}
      <header className="surface px-5 py-4">
        <div className="flex items-start gap-4">
          {m.icon && <img src={m.icon} className="size-12 rounded-lg object-cover shrink-0 bg-bg-panel ring-1 ring-line/60" alt="" />}
          <div className="flex-1 min-w-0">
            <h1 className="heading-2 truncate">{m.question || cid.slice(0, 20)}</h1>
            <div className="text-[12px] text-text-muted mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1">
              {m.event_slug && <span className="font-mono text-[11px]">{m.event_slug}</span>}
              <span className="text-text-faint">·</span>
              <span><code className="font-mono text-[10.5px] bg-chip px-1.5 py-0.5 rounded-sm">{cid.slice(0, 12)}…</code></span>
              {m.neg_risk && <span className="chip-amber">neg-risk</span>}
              {m.uma_status && <span className="chip">{m.uma_status}</span>}
              {m.game_start && (
                <span className="text-text-muted inline-flex items-center gap-1">
                  <Flag className="size-3" />
                  game {new Date(m.game_start).toLocaleString("en-GB", { dateStyle: "short", timeStyle: "short" })}
                </span>
              )}
            </div>
          </div>
        </div>

        {/* Lifecycle KPIs */}
        <div className="mt-4 pt-4 border-t border-line grid grid-cols-2 md:grid-cols-5 gap-3">
          <Kpi label="첫 leg" value={firstLegName}
            sub={`outcome ${firstLeg ?? "—"} · ${fills[0] ? relTime(fills[0].ts) : ""}`}
            accent={firstLeg === 0 ? "buy" : "sell"} />
          <Kpi label="2nd leg 까지" value={timeToSecondMin != null ? fmtDur(timeToSecondMin * 60) : "—"}
            sub={d.lifecycle.two_sided ? "two-sided 완성" : "single-leg 만"} />
          <Kpi label="총 fills" value={fmtN(d.total_fills)}
            sub={d.active_minutes != null ? `${fmtDur(d.active_minutes * 60)} 활성` : ""} />
          <Kpi label="최종 balance" value={d.lifecycle.final_balance_ratio != null
            ? `${(d.lifecycle.final_balance_ratio * 100).toFixed(0)}%` : "—"}
            sub="min/max leg shares" />
          <Kpi label="게임 시작 대비"
            value={fills[0].secs_from_game_min != null ? fmtEntryMin(fills[0].secs_from_game_min) : "—"}
            sub="첫 fill 시점" />
        </div>
      </header>

      {/* Per-leg summary cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {d.legs.map(leg => (
          <div key={leg.outcome_index} className={cn("surface px-5 py-4",
            leg.outcome_index === 0 ? "border-l-4 border-l-buy/60" : "border-l-4 border-l-sell/60")}>
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2">
                <span className={cn("size-2.5 rounded-full",
                  leg.outcome_index === 0 ? "bg-buy" : "bg-sell")} />
                <span className="text-[13px] font-semibold">
                  [{leg.outcome_index}] {outcomes[leg.outcome_index] ?? "—"}
                </span>
                {firstLeg === leg.outcome_index && (
                  <span className="chip-accent">FIRST</span>
                )}
              </div>
              <span className="text-[11px] text-text-muted">{fmtN(leg.fills)} fills</span>
            </div>
            <div className="grid grid-cols-3 gap-3 mt-3">
              <SubKpi label="Shares" value={fmtN(leg.shares, 1)} />
              <SubKpi label="USDC" value={fmt$(leg.usdc, 0)} />
              <SubKpi label="Avg price" value={leg.avg_price.toFixed(4)} />
            </div>
            <div className="text-[10px] text-text-dim mt-2">
              첫 fill: {relTime(leg.first_ts)}
            </div>
          </div>
        ))}
      </div>

      {/* Cumulative shares chart — 두 leg 라인 */}
      <section className="surface px-5 py-4">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <Activity className="size-4 text-accent" />
            <h2 className="label-section">Leg 누적 매수 곡선</h2>
          </div>
          <div className="text-[10.5px] text-text-muted flex items-center gap-3">
            <span className="inline-flex items-center gap-1"><span className="size-2 rounded-full bg-buy" /> [0] {outcomes[0]}</span>
            <span className="inline-flex items-center gap-1"><span className="size-2 rounded-full bg-sell" /> [1] {outcomes[1]}</span>
          </div>
        </div>
        <div className="h-[260px]">
          <ResponsiveContainer width="100%" height="100%">
            <ComposedChart data={chartData} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="#1f2935" strokeDasharray="2 4" vertical={false} />
              <XAxis dataKey="ts" type="number" scale="time"
                domain={[firstFillTs, lastFillTs]}
                tick={{ fill: "#7d8b9c", fontSize: 10 }}
                tickFormatter={(t: number) => new Date(t).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })} />
              <YAxis tick={{ fill: "#7d8b9c", fontSize: 10 }} />
              <Tooltip
                contentStyle={{ background: "#0a0e14", border: "1px solid #1f2935", borderRadius: 8, fontSize: 11 }}
                labelFormatter={(t: number) => new Date(t).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
                formatter={(v: number, name: string) => {
                  if (name === "cum0") return [fmtN(v, 0), `[0] ${outcomes[0] ?? "0"}`];
                  if (name === "cum1") return [fmtN(v, 0), `[1] ${outcomes[1] ?? "1"}`];
                  return [v, name];
                }}
              />
              {gameStartTs && gameStartTs >= firstFillTs && gameStartTs <= lastFillTs && (
                <ReferenceLine x={gameStartTs} stroke="#f59e0b" strokeDasharray="3 3"
                  label={{ value: "game", position: "top", fill: "#f59e0b", fontSize: 10 }} />
              )}
              <Area type="monotone" dataKey="cum0" stroke="#22c55e" strokeWidth={2} fill="#22c55e" fillOpacity={0.15} isAnimationActive={false} />
              <Area type="monotone" dataKey="cum1" stroke="#ef4444" strokeWidth={2} fill="#ef4444" fillOpacity={0.15} isAnimationActive={false} />
            </ComposedChart>
          </ResponsiveContainer>
        </div>
      </section>

      {/* Quote timeline — book_events + fills 시계열 overlay */}
      <QuoteTimelineSection cid={cid} outcomes={outcomes} />

      {/* Fill-by-fill timeline */}
      <section className="surface-flush">
        <div className="px-5 py-3 border-b border-line flex items-center gap-2">
          <Crosshair className="size-4 text-accent-2" />
          <h2 className="label-section">시간순 체결 시퀀스 · {fmtN(d.total_fills)} fills</h2>
          <span className="ml-auto text-[10.5px] text-text-muted">
            <span className="text-buy mr-2">M maker</span>
            <span className="text-warn mr-2">I inside</span>
            <span className="text-sell">T taker</span>
          </span>
        </div>
        <div className="max-h-[600px] overflow-y-auto">
          <table className="w-full">
            <thead className="sticky top-0 bg-bg-elevated z-10">
              <tr className="border-b border-line">
                <th className="th text-left w-10">#</th>
                <th className="th text-left w-[100px]">Time</th>
                <th className="th text-left">Leg</th>
                <th className="th text-center w-12">Cls</th>
                <th className="th text-right">Price</th>
                <th className="th text-right" title="vs mid 시점">Δmid</th>
                <th className="th text-right">Size</th>
                <th className="th text-right">USDC</th>
                <th className="th text-right">Cum 0</th>
                <th className="th text-right">Cum 1</th>
                <th className="th text-right">Bal</th>
              </tr>
            </thead>
            <tbody>
              {fills.map((f, i) => <FillRow key={i} f={f} idx={i+1} outcomes={outcomes} prev={i > 0 ? fills[i-1] : null} />)}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

/** Quote timeline — book_events (bid/ask 변동) 와 RN1 fills 시계열 overlay.
 *  사용자 가설 "가격 변동에 맞춰 quote 갱신" 의 *마이크로* 검증. */
function QuoteTimelineSection({ cid, outcomes }: { cid: string; outcomes: string[] }) {
  const q = useQuery({
    queryKey: ["quote-timeline", cid],
    queryFn: () => api.quoteTimeline(cid),
    refetchInterval: 30_000,
  });

  const d = q.data;

  // 차트 데이터 변환 — 두 outcome 의 bid/ask 라인 + fill 마커
  const chartData = useMemo(() => {
    if (!d) return { rows: [], firstTs: 0, lastTs: 0 };
    // book events 와 fills 를 ts 기준 통합 → 각 ts 별 bid_0, ask_0, bid_1, ask_1, fill_0_price, fill_1_price
    const eventMap = new Map<number, any>();
    // 누적 bid/ask state per outcome
    let curr0 = { bid: null as number | null, ask: null as number | null };
    let curr1 = { bid: null as number | null, ask: null as number | null };
    const allEvents: { ts: number; type: "book" | "fill"; outcome: number; bid?: number | null; ask?: number | null; price?: number; size?: number }[] = [];

    for (const b of d.book_events) {
      allEvents.push({
        ts: new Date(b.ts).getTime(),
        type: "book",
        outcome: b.outcome_index,
        bid: b.best_bid,
        ask: b.best_ask,
      });
    }
    for (const f of d.fills) {
      allEvents.push({
        ts: new Date(f.ts).getTime(),
        type: "fill",
        outcome: f.outcome_index,
        price: f.price,
        size: f.size,
      });
    }
    allEvents.sort((a, b) => a.ts - b.ts);

    const rows: any[] = [];
    for (const e of allEvents) {
      if (e.type === "book") {
        const target = e.outcome === 0 ? curr0 : curr1;
        if (e.bid != null) target.bid = e.bid;
        if (e.ask != null) target.ask = e.ask;
      }
      // row 추가: 현재 state + 이 이벤트의 fill (있다면)
      rows.push({
        ts: e.ts,
        bid0: curr0.bid,
        ask0: curr0.ask,
        bid1: curr1.bid,
        ask1: curr1.ask,
        fill_0: e.type === "fill" && e.outcome === 0 ? e.price : null,
        fill_1: e.type === "fill" && e.outcome === 1 ? e.price : null,
      });
    }
    const firstTs = rows.length > 0 ? rows[0].ts : 0;
    const lastTs = rows.length > 0 ? rows[rows.length - 1].ts : 0;
    return { rows, firstTs, lastTs };
  }, [d]);

  return (
    <section className="surface px-5 py-4">
      <div className="flex items-center gap-2 mb-3">
        <BookOpen className="size-4 text-info" />
        <h2 className="label-section">
          Quote 갱신 timeline · book × fills
          <span className="text-text-dim font-normal normal-case tracking-normal ml-2">
            ({d?.book_event_count ?? 0} book events · {d?.fill_count ?? 0} fills)
          </span>
        </h2>
      </div>
      {q.isLoading ? (
        <div className="py-12 text-center text-text-muted text-[13px]">로드 중…</div>
      ) : !d || chartData.rows.length === 0 ? (
        <div className="py-12 text-center text-text-muted text-[13px]">book 데이터 없음</div>
      ) : (
        <>
          <div className="text-[10.5px] text-text-muted flex items-center gap-3 mb-2">
            <span className="inline-flex items-center gap-1"><span className="size-2 rounded-full bg-buy" /> [0] {outcomes[0]} bid/ask</span>
            <span className="inline-flex items-center gap-1"><span className="size-2 rounded-full bg-sell" /> [1] {outcomes[1]} bid/ask</span>
            <span className="inline-flex items-center gap-1"><span className="size-2 rounded-full bg-accent" /> RN1 fill</span>
          </div>
          <div className="h-[320px]">
            <ResponsiveContainer width="100%" height="100%">
              <ComposedChart data={chartData.rows} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <CartesianGrid stroke="#1f2935" strokeDasharray="2 4" vertical={false} />
                <XAxis dataKey="ts" type="number" scale="time"
                  domain={[chartData.firstTs, chartData.lastTs]}
                  tick={{ fill: "#7d8b9c", fontSize: 10 }}
                  tickFormatter={(t: number) => new Date(t).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })} />
                <YAxis tick={{ fill: "#7d8b9c", fontSize: 10 }} domain={[0, 1]}
                  tickFormatter={(v: number) => `${(v*100).toFixed(0)}¢`} />
                <Tooltip
                  contentStyle={{ background: "#0a0e14", border: "1px solid #1f2935", borderRadius: 8, fontSize: 11 }}
                  labelFormatter={(t: number) => new Date(t).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
                  formatter={(value, name) => {
                    const v = typeof value === "number" ? value : null;
                    if (v == null) return ["", ""];
                    const labels: Record<string, string> = {
                      bid0: `[0] bid`, ask0: `[0] ask`,
                      bid1: `[1] bid`, ask1: `[1] ask`,
                      fill_0: `[0] fill`, fill_1: `[1] fill`,
                    };
                    return [`${(v*100).toFixed(1)}¢`, labels[String(name)] ?? String(name)];
                  }}
                />
                {/* outcome 0 — buy 색 */}
                <Line type="stepAfter" dataKey="bid0" stroke="#22c55e" strokeWidth={1.2} dot={false} isAnimationActive={false} connectNulls />
                <Line type="stepAfter" dataKey="ask0" stroke="#22c55e" strokeWidth={1.2} strokeDasharray="3 3" dot={false} isAnimationActive={false} connectNulls />
                {/* outcome 1 — sell 색 */}
                <Line type="stepAfter" dataKey="bid1" stroke="#ef4444" strokeWidth={1.2} dot={false} isAnimationActive={false} connectNulls />
                <Line type="stepAfter" dataKey="ask1" stroke="#ef4444" strokeWidth={1.2} strokeDasharray="3 3" dot={false} isAnimationActive={false} connectNulls />
                {/* fills — accent 점 */}
                <Line dataKey="fill_0" stroke="none" dot={{ r: 3, fill: "#22c55e", stroke: "#0a0e14", strokeWidth: 1 }} isAnimationActive={false} />
                <Line dataKey="fill_1" stroke="none" dot={{ r: 3, fill: "#ef4444", stroke: "#0a0e14", strokeWidth: 1 }} isAnimationActive={false} />
              </ComposedChart>
            </ResponsiveContainer>
          </div>
          <div className="mt-2 text-[10.5px] text-text-dim">
            실선 = bid · 점선 = ask · 점 = RN1 fill. fill 점이 그 outcome 의 bid 라인 위에 있으면 maker rest 매수 (RN1 의 호가가 hit 됨).
          </div>
        </>
      )}
    </section>
  );
}

function FillRow({ f, idx, outcomes, prev }: { f: LifecycleFill; idx: number; outcomes: string[]; prev: LifecycleFill | null }) {
  const isFirstOfLeg = !prev || prev.outcome_index !== f.outcome_index;
  // inter-fill 시간
  const interSec = prev ? (new Date(f.ts).getTime() - new Date(prev.ts).getTime()) / 1000 : null;
  const clsBadge = f.classification === "maker" ? <span className="text-[9px] font-bold text-buy" title="maker rest hit">M</span>
                 : f.classification === "taker" ? <span className="text-[9px] font-bold text-sell" title="taker crossed">T</span>
                 : f.classification === "inside" ? <span className="text-[9px] font-bold text-warn" title="inside spread">I</span>
                 : <span className="text-[9px] text-text-dim">–</span>;
  const legColor = f.outcome_index === 0 ? "text-buy" : "text-sell";
  const legBg = f.outcome_index === 0 ? "bg-buy/40" : "bg-sell/40";

  return (
    <tr className="border-b border-line/30 hover:bg-bg-panel/40 transition-colors text-[12px]">
      <td className="td text-text-dim num">{idx}</td>
      <td className="td">
        <div className="num text-text">{new Date(f.ts).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</div>
        {interSec != null && interSec > 0 && (
          <div className="text-[9.5px] text-text-dim num">+{fmtDur(interSec)}</div>
        )}
      </td>
      <td className="td">
        <div className="flex items-center gap-1.5">
          <span className={cn("inline-block size-1.5 rounded-full", legBg)} />
          <span className={cn("font-medium", legColor)}>
            [{f.outcome_index}] {f.outcome_name || outcomes[f.outcome_index] || "—"}
          </span>
          {isFirstOfLeg && <span className="chip-accent ml-1">first</span>}
        </div>
      </td>
      <td className="td text-center">{clsBadge}</td>
      <td className="td text-right num">{(f.price * 100).toFixed(1)}¢</td>
      <td className="td text-right num">
        {f.dist_from_mid != null ? (
          <span className={f.dist_from_mid >= 0 ? "text-sell" : "text-buy"}>
            {f.dist_from_mid >= 0 ? "+" : ""}{(f.dist_from_mid * 100).toFixed(2)}¢
          </span>
        ) : <span className="text-text-dim">—</span>}
      </td>
      <td className="td text-right num text-text-muted">{fmtN(f.size, 1)}</td>
      <td className="td text-right num">{fmt$(f.usdc_size, 2)}</td>
      <td className="td text-right num text-buy">{fmtN(f.cum_shares_0, 0)}</td>
      <td className="td text-right num text-sell">{fmtN(f.cum_shares_1, 0)}</td>
      <td className="td text-right num text-text-muted">{(f.balance_ratio * 100).toFixed(0)}%</td>
    </tr>
  );
}

function BackButton() {
  return (
    <Link href="/rn1/analysis" className="inline-flex items-center gap-1.5 text-[12px] text-text-muted hover:text-text transition-colors">
      <ArrowLeft className="size-3.5" /> Analysis
    </Link>
  );
}

function Kpi({ label, value, sub, accent }: { label: string; value: string; sub?: string; accent?: "buy" | "sell" }) {
  return (
    <div>
      <div className="label-eyebrow">{label}</div>
      <div className={cn("text-[18px] font-semibold tracking-tightish mt-1",
        accent === "buy" ? "text-buy" : accent === "sell" ? "text-sell" : "text-text")}>
        {value}
      </div>
      {sub && <div className="text-[10px] text-text-dim mt-0.5">{sub}</div>}
    </div>
  );
}

function SubKpi({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[9.5px] uppercase tracking-wider text-text-dim">{label}</div>
      <div className="text-[14px] font-semibold num mt-0.5">{value}</div>
    </div>
  );
}

function fmtDur(sec: number): string {
  const abs = Math.abs(sec);
  if (abs < 60) return `${abs.toFixed(0)}s`;
  if (abs < 3600) return `${(abs/60).toFixed(1)}m`;
  if (abs < 86400) return `${(abs/3600).toFixed(1)}h`;
  return `${(abs/86400).toFixed(1)}d`;
}

function fmtEntryMin(m: number): string {
  const abs = Math.abs(m);
  const sign = m < 0 ? "-" : "+";
  if (abs < 60) return `${sign}${abs.toFixed(0)}m`;
  if (abs < 1440) return `${sign}${(abs/60).toFixed(1)}h`;
  return `${sign}${(abs/1440).toFixed(1)}d`;
}
