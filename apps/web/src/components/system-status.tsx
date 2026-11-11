"use client";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { relTime, cn } from "@/lib/fmt";

/** Sidebar 하단 — collector / api / ws lifecycle 살아있는지 점등.
 *  점·이름·상대시간 한 줄 — 각 component 별 heartbeat.
 *
 *  Freshness 판정 임계는 component 의 실제 주기에 맞춤. 일률 60s 로 잡으면
 *  pnl_snap (5min) / universe (5min) 가 정상 동작 중에도 "주의" 로 잘못 표시됨.
 *  임계 = 주기 × 2 정도 (한 사이클 놓쳐도 다음 사이클 내에 회복).
 */
const FRESH_THRESHOLD_MS: Record<string, number> = {
  "lifecycle.book_sink":     60_000,   // WS 실시간
  "lifecycle.fills":         60_000,   // 30s 주기
  "lifecycle.positions":     60_000,   // 30s 주기
  "lifecycle.markets_meta": 150_000,   // 60s 주기 × 2.5
  "lifecycle.pnl_snap":     600_000,   // 5min 주기 × 2
  "lifecycle.universe":     600_000,   // 5min 주기 × 2
};
const DEFAULT_THRESHOLD_MS = 120_000;

function thresholdFor(component: string): number {
  return FRESH_THRESHOLD_MS[component] ?? DEFAULT_THRESHOLD_MS;
}

function isFresh(ts: string, component: string): boolean {
  return Date.now() - new Date(ts).getTime() < thresholdFor(component);
}

export function SystemStatus() {
  const { data, isError } = useQuery({
    queryKey: ["system-status"],
    queryFn: api.systemStatus,
    // 사이드바 footer — 항상 떠 있음. 30s 갱신.
    refetchInterval: 30_000,
    staleTime: 25_000,
  });

  if (isError) return (
    <Row>
      <Dot tone="sell" />
      <span className="text-sell">api 연결 실패</span>
    </Row>
  );

  const beats = data?.heartbeats ?? [];
  const allFresh = beats.length > 0 && beats.every(b => isFresh(b.ts, b.component));
  const overall: "buy" | "warn" | "sell" =
    beats.length === 0 ? "warn" :
    allFresh ? "buy" : "warn";

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between mb-1">
        <span className="label-eyebrow">시스템</span>
        <span className={cn(
          "inline-flex items-center gap-1 text-[10px] font-medium",
          overall === "buy" ? "text-buy" : overall === "warn" ? "text-warn" : "text-sell",
        )}>
          <Dot tone={overall} pulse={overall === "buy"} />
          {overall === "buy" ? "정상" : overall === "warn" ? "주의" : "오프라인"}
        </span>
      </div>
      {beats.length === 0 && (
        <Row>
          <Dot tone="warn" />
          <span className="text-text-dim">heartbeat 없음</span>
        </Row>
      )}
      {beats.map(hb => {
        const fresh = isFresh(hb.ts, hb.component);
        const periodLabel = formatPeriod(thresholdFor(hb.component));
        return (
          <Row key={hb.component}>
            <Dot tone={fresh ? "buy" : "warn"} pulse={fresh} />
            <span className="truncate flex-1 text-text-muted" title={`임계 ${periodLabel}`}>
              {hb.component.replace("lifecycle.", "")}
            </span>
            <span className="text-text-dim shrink-0 font-mono">{relTime(hb.ts)}</span>
          </Row>
        );
      })}
    </div>
  );
}

function Row({ children }: { children: React.ReactNode }) {
  return <div className="text-[11px] flex items-center gap-1.5">{children}</div>;
}

function Dot({ tone, pulse }: { tone: "buy" | "warn" | "sell"; pulse?: boolean }) {
  const bg = tone === "buy" ? "bg-buy" : tone === "warn" ? "bg-warn" : "bg-sell";
  return (
    <span className="relative inline-flex shrink-0">
      <span className={cn("inline-block w-1.5 h-1.5 rounded-full", bg)} />
      {pulse && <span className={cn("absolute inset-0 inline-block w-1.5 h-1.5 rounded-full animate-ping opacity-60", bg)} />}
    </span>
  );
}

function formatPeriod(ms: number): string {
  if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m`;
  return `${Math.round(ms / 3_600_000)}h`;
}
