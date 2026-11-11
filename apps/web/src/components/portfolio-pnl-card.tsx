"use client";
import { useMemo } from "react";
import { PnlSeries } from "@/lib/api";
import { fmt$, cls, cn } from "@/lib/fmt";
import { ResponsiveContainer, Tooltip, YAxis, XAxis, Area, AreaChart } from "recharts";

export type TF = "1D" | "1W" | "1M" | "1Y" | "YTD" | "ALL";

/** Polymarket portfolio Profit/Loss 카드 — recharts dynamic-imported. */
export default function PortfolioPnLCard({ tf, setTf, series, scope }:
  { tf: TF; setTf: (v: TF) => void; series: PnlSeries | undefined; scope: "recent" | "all" }) {
  const points = series?.points ?? [];
  const { winStart, winEnd } = useMemo(() => {
    const end = Date.now();
    switch (tf) {
      case "1W":  return { winStart: end -   7 * 24 * 3600 * 1000, winEnd: end };
      case "1M":  return { winStart: end -  30 * 24 * 3600 * 1000, winEnd: end };
      case "1Y":  return { winStart: end - 365 * 24 * 3600 * 1000, winEnd: end };
      case "YTD": {
        const yearStart = new Date(new Date().getFullYear(), 0, 1).getTime();
        return { winStart: yearStart, winEnd: end };
      }
      case "ALL": {
        const firstMs = points.length > 0 ? new Date(points[0].ts).getTime() : end - 30 * 24 * 3600 * 1000;
        return { winStart: Math.min(firstMs, end - 30 * 24 * 3600 * 1000), winEnd: end };
      }
      default:    return { winStart: end - 24 * 3600 * 1000, winEnd: end };
    }
  }, [tf, points]);

  const data = useMemo(() => {
    if (points.length === 0) return [];
    const bucketCnt =
      tf === "1D"  ? 24
    : tf === "1W"  ? 56
    : tf === "1M"  ? 120
    : tf === "1Y"  ? 365
    : tf === "YTD" ? Math.max(60, Math.floor((winEnd - winStart) / (24 * 3600 * 1000)))
    :                Math.max(60, Math.floor((winEnd - winStart) / (24 * 3600 * 1000)));
    const bucketWidth = (winEnd - winStart) / bucketCnt;
    const baseNet = points[0].net;
    const baseRealized = points[0].realized;
    const baseUnreal = points[0].unrealized;
    const snaps = points.map(p => ({
      ms: new Date(p.ts).getTime(),
      net: p.net, realized: p.realized, unrealized: p.unrealized,
    })).sort((a, b) => a.ms - b.ms);

    const result: Array<{ts_ms: number; net_delta: number; realized_delta: number; unrealized_delta: number; net_abs: number}> = [];
    let idx = 0;
    let cNet = baseNet, cReal = baseRealized, cUnreal = baseUnreal;
    for (let i = 0; i <= bucketCnt; i++) {
      const ts_ms = Math.round(winStart + i * bucketWidth);
      while (idx < snaps.length && snaps[idx].ms <= ts_ms) {
        cNet = snaps[idx].net;
        cReal = snaps[idx].realized;
        cUnreal = snaps[idx].unrealized;
        idx++;
      }
      result.push({
        ts_ms,
        net_delta: cNet - baseNet,
        realized_delta: cReal - baseRealized,
        unrealized_delta: cUnreal - baseUnreal,
        net_abs: cNet,
      });
    }
    return result;
  }, [points, tf, winStart, winEnd]);

  const first = points[0];
  const last  = points[points.length - 1];
  const delta = last && first ? last.net - first.net : 0;
  const isUp = delta >= 0;
  const absNet = last?.net ?? 0;
  const stroke = "#5bc0eb";
  const hasData = data.length > 1;

  return (
    <div className="surface px-5 py-4 flex flex-col">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-1.5 text-[12px]">
          {isUp ? (
            <span className="inline-block w-0 h-0 border-x-[4px] border-x-transparent border-b-[7px] border-b-buy" aria-hidden />
          ) : (
            <span className="inline-block w-0 h-0 border-x-[4px] border-x-transparent border-t-[7px] border-t-sell" aria-hidden />
          )}
          <span className="text-text-muted font-medium">Profit/Loss</span>
        </div>
        <TfToggle value={tf} onChange={setTf} />
      </div>

      <div className="mt-2">
        <div className={cn(
          "text-[32px] leading-none font-semibold tracking-tightish num",
          isUp ? "text-buy" : "text-sell"
        )}>
          {delta >= 0 ? "+" : ""}{fmt$(delta, 2)}
        </div>
        <div className="text-[11.5px] text-text-muted mt-2 flex items-center gap-2 flex-wrap">
          <span>{tfLabel(tf)}</span>
          <span className="text-text-faint">·</span>
          <span>
            <span className="text-text-dim">절대 Net</span>{" "}
            <span className={cn("num", cls(absNet))}>{fmt$(absNet, 0)}</span>
          </span>
          {scope === "all" && absNet < -100000 && (
            <span className="text-[10px] text-text-dim italic" title="positions API 의 cash_pnl 은 losing leg 의 미회수 sunk cost 포함. trade-based forensic 분석 (+$1.59M/21d) 와 다른 metric.">
              (incl. sunk cost)
            </span>
          )}
        </div>
      </div>

      <div className="mt-3 flex-1 min-h-[160px]">
        {hasData ? (
          <ResponsiveContainer width="100%" height={160}>
            <AreaChart data={data} margin={{ top: 6, right: 4, left: 0, bottom: 0 }}>
              <defs>
                <linearGradient id="pnlGrad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%"   stopColor={stroke} stopOpacity={0.32} />
                  <stop offset="60%"  stopColor={stroke} stopOpacity={0.08} />
                  <stop offset="100%" stopColor={stroke} stopOpacity={0} />
                </linearGradient>
              </defs>
              <YAxis hide domain={["auto", "auto"]} />
              <XAxis dataKey="ts_ms" type="number" scale="time" domain={[winStart, winEnd]} hide />
              <Tooltip
                contentStyle={{ background: "#0a0e14", border: "1px solid #1f2935", borderRadius: 8, fontSize: 11, padding: "6px 9px" }}
                labelStyle={{ color: "#7d8b9c", fontSize: 10 }}
                itemStyle={{ color: "#e6edf3" }}
                cursor={{ stroke: "#566373", strokeDasharray: "2 3", strokeWidth: 1 }}
                formatter={(value, name, item) => {
                  const v = typeof value === "number" ? value : 0;
                  if (name === "net_delta") {
                    type Pt = { net_abs?: number };
                    const payload = (item as { payload?: Pt }).payload;
                    const abs = payload?.net_abs ?? 0;
                    return [`${v >= 0 ? "+" : ""}${fmt$(v, 2)}  (abs ${fmt$(abs, 0)})`, "ΔNet"];
                  }
                  return [(v >= 0 ? "+" : "") + fmt$(v, 2), String(name)];
                }}
                labelFormatter={(t) => {
                  const ms = typeof t === "number" ? t : Number(t);
                  const d = new Date(ms);
                  return d.toLocaleString("en-GB", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
                }}
              />
              <Area type="monotone" dataKey="net_delta" stroke={stroke} strokeWidth={2.2}
                    fill="url(#pnlGrad)" isAnimationActive={false} dot={false} connectNulls />
            </AreaChart>
          </ResponsiveContainer>
        ) : (
          <div className="flex flex-col items-center justify-center h-[160px] text-[11px] text-text-dim gap-1">
            <span>차트 데이터 누적 중…</span>
            <span>collector 5분 snapshot · {series?.point_count ?? 0} points</span>
          </div>
        )}
      </div>
    </div>
  );
}

function TfToggle({ value, onChange }: { value: TF; onChange: (v: TF) => void }) {
  const opts: TF[] = ["1D", "1W", "1M", "1Y", "YTD", "ALL"];
  return (
    <div className="seg">
      {opts.map(o => (
        <button key={o} onClick={() => onChange(o)}
          className={cn("seg-btn", value === o && "seg-btn-active")}>
          {o}
        </button>
      ))}
    </div>
  );
}

function tfLabel(tf: TF): string {
  return tf === "1D" ? "Past Day"
       : tf === "1W" ? "Past Week"
       : tf === "1M" ? "Past Month"
       : tf === "1Y" ? "Past Year"
       : tf === "YTD" ? "Year-to-Date" : "All-time";
}
