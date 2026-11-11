"use client";
import dynamic from "next/dynamic";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/lib/api";
import { fmt$, fmtN, cn } from "@/lib/fmt";
import type { TF } from "@/components/portfolio-pnl-card";

/** PnL 차트 (recharts ~30 KB) 는 dynamic import — RN1 페이지에서만 다운로드.
 *  ssr:false 로 클라이언트 hydration 후 lazy-load. */
const PortfolioPnLCard = dynamic(() => import("@/components/portfolio-pnl-card"), {
  ssr: false,
  loading: () => (
    <div className="surface px-5 py-4 flex flex-col">
      <div className="flex items-center gap-1.5 text-[12px] text-text-muted font-medium">Profit/Loss</div>
      <div className="mt-2 text-[32px] leading-none font-semibold text-text-dim num">—</div>
      <div className="mt-3 flex-1 min-h-[160px] flex items-center justify-center text-[11px] text-text-dim">
        차트 로딩 중…
      </div>
    </div>
  ),
});

type Scope = "recent" | "all";

/** 폴리마켓 portfolio 페이지 상단의 프로필 카드 + PnL 그래프 미러. */
export function PortfolioHeader() {
  const [tf, setTf] = useState<TF>("1D");
  const [scope, setScope] = useState<Scope>("recent");
  const summary = useQuery({
    queryKey: ["portfolio-summary", scope],
    queryFn: () => api.summary(scope),
    refetchInterval: 30_000,
    staleTime: 25_000,
  });
  const series = useQuery({
    queryKey: ["pnl-series", tf, scope],
    queryFn: () => api.pnlSeries(tf, scope),
    refetchInterval: 60_000,
    staleTime: 50_000,
  });

  const s = summary.data;
  const positionsValue = s?.positions_value ?? 0;
  const biggestWin     = s?.biggest_win ?? null;
  const predictions    = s?.predictions ?? 0;

  return (
    <div className="grid grid-cols-1 lg:grid-cols-[1fr_1.1fr] gap-3 sm:gap-4 mb-4 animate-fade-in-up">
      <ProfileCard
        alias="RN1"
        joined={s?.joined_at ?? "2024-12-12"}
        positionsValue={positionsValue}
        biggestWin={biggestWin}
        predictions={predictions}
        scope={scope}
        setScope={setScope}
      />
      <PortfolioPnLCard tf={tf} setTf={setTf} series={series.data} scope={scope} />
    </div>
  );
}

function ProfileCard({ alias, joined, positionsValue, biggestWin, predictions, scope, setScope }: {
  alias: string; joined: string;
  positionsValue: number; biggestWin: number | null; predictions: number;
  scope: "recent" | "all"; setScope: (v: "recent" | "all") => void;
}) {
  return (
    <div className="surface-hero px-5 py-4">
      <div className="flex items-start gap-4">
        <div className="size-12 rounded-2xl shrink-0 bg-gradient-to-br from-accent-2/30 via-accent/20 to-warn/15
                        border border-accent-2/35 flex items-center justify-center shadow-glow">
          <div className="text-[20px] font-semibold tracking-tightish text-gradient-accent">R</div>
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-[22px] font-semibold tracking-tightish leading-tight text-text">{alias}</div>
          <div className="text-[11.5px] text-text-muted mt-1 flex items-center gap-2 flex-wrap">
            <span>Joined {formatJoined(joined)}</span>
            <span className="text-text-faint">·</span>
            <code className="font-mono text-[10.5px] bg-chip/80 px-2 py-0.5 rounded-md text-text-muted border border-line/60">
              0x2005…75ea
            </code>
          </div>
        </div>

        <div className="seg shrink-0">
          {(["recent", "all"] as const).map(s => (
            <button key={s} onClick={() => setScope(s)}
              className={cn("seg-btn whitespace-nowrap", scope === s && "seg-btn-active")}>
              {s === "recent" ? "최근 24h" : "전체"}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-5 grid grid-cols-3 gap-3 sm:gap-4 pt-4 border-t border-line/60">
        <Stat label="Positions Value" value={fmt$(positionsValue, 0)} sub={scope === "recent" ? "24h 활동 마켓" : "전체 보유"} />
        <Stat label="Biggest Win" value={biggestWin != null ? fmt$(biggestWin, 0) : "—"} sub={scope === "recent" ? "24h 안 resolved" : "전체 resolved"} positive={biggestWin != null && biggestWin > 0} />
        <Stat label="Predictions" value={fmtN(predictions, 0)} sub={scope === "recent" ? "24h 체결" : "누적 관찰"} />
      </div>
    </div>
  );
}

function Stat({ label, value, sub, positive }: { label: string; value: string; sub?: string; positive?: boolean }) {
  return (
    <div className="space-y-0.5 min-w-0">
      <div className={cn(
        "text-[18px] sm:text-[19px] font-semibold tracking-tightish num leading-none truncate",
        positive && "text-buy"
      )}>{value}</div>
      <div className="text-[10.5px] sm:text-[11px] text-text-muted mt-1.5 truncate">{label}</div>
      {sub && <div className="text-[9.5px] sm:text-[10px] text-text-dim truncate">{sub}</div>}
    </div>
  );
}

function formatJoined(d: string): string {
  try {
    const dt = new Date(d);
    return dt.toLocaleDateString("en-US", { month: "short", year: "numeric" });
  } catch { return d; }
}
