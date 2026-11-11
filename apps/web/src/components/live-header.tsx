"use client";
import Link from "next/link";
import dynamic from "next/dynamic";
import { usePathname } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/lib/api";
import { fmt$, fmtN, cn } from "@/lib/fmt";
import { Zap } from "lucide-react";
import type { TF } from "@/components/live-pnl-card";

/** PnL 차트 (recharts ~30 KB) 는 dynamic import — `/live/*` 페이지에서만 다운로드.
 *  ssr:false 로 클라이언트 hydration 후 lazy-load. */
const LivePnLCard = dynamic(() => import("@/components/live-pnl-card"), {
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

/** rn1-trader 의 polymarket-style portfolio 헤더 + tab bar. */
export function LiveHeader() {
  const [tf, setTf] = useState<TF>("1D");
  const [scope, setScope] = useState<Scope>("recent");

  const summary = useQuery({
    queryKey: ["live-portfolio-summary", scope],
    queryFn: () => api.livePortfolioSummary(scope),
    refetchInterval: 6_000,
    staleTime: 4_000,
  });
  const series = useQuery({
    queryKey: ["live-pnl-series", tf, scope],
    queryFn: () => api.livePnlSeries(tf, scope),
    refetchInterval: 10_000,
    staleTime: 8_000,
  });

  const pathname = usePathname() || "";
  const tabs = [
    { href: "/live/summary",   label: "요약" },
    { href: "/live/markets",   label: "경기 일정" },
    { href: "/live/positions", label: "내 Positions" },
    { href: "/live/orders",    label: "내 Open Orders" },
    { href: "/live/pnl",       label: "PnL" },
    { href: "/live/risk",      label: "Risk" },
  ];

  const s = summary.data;
  const positionsValue = s?.positions_value ?? 0;
  const biggestWin     = s?.biggest_win ?? null;
  const predictions    = scope === "recent" ? (s?.predictions_24h ?? 0) : (s?.predictions ?? 0);
  const alias          = s?.alias ?? "rn1-trader";
  const wallet         = s?.wallet ?? "—";
  const joinedAt       = s?.joined_at;
  const isLive         = alias.includes("LIVE");

  return (
    <>
      <div className="grid grid-cols-1 lg:grid-cols-[1fr_1.1fr] gap-3 sm:gap-4 mb-4">
        <ProfileCard
          alias={alias}
          wallet={wallet}
          isLive={isLive}
          joined={joinedAt}
          positionsValue={positionsValue}
          biggestWin={biggestWin}
          predictions={predictions}
          scope={scope}
          setScope={setScope}
        />
        <LivePnLCard tf={tf} setTf={setTf} series={series.data} />
      </div>

      <div className="flex items-center border-b border-line/70 mb-4 overflow-x-auto no-scrollbar -mx-1 px-1">
        {tabs.map(t => {
          const active = pathname.startsWith(t.href);
          return (
            <Link key={t.href} href={t.href} prefetch
              className={cn("tab whitespace-nowrap", active ? "tab-active" : "tab-inactive")}>
              {t.label}
            </Link>
          );
        })}
      </div>
    </>
  );
}

function ProfileCard({ alias, wallet, isLive, joined, positionsValue, biggestWin, predictions, scope, setScope }: {
  alias: string; wallet: string; isLive: boolean; joined: string | undefined;
  positionsValue: number; biggestWin: number | null; predictions: number;
  scope: Scope; setScope: (s: Scope) => void;
}) {
  const walletShort = wallet.startsWith("0x") && wallet.length > 12
    ? `${wallet.slice(0, 6)}…${wallet.slice(-4)}`
    : wallet;
  return (
    <div className="surface-hero px-5 py-4 animate-fade-in-up">
      <div className="flex items-start gap-4">
        <div className={cn(
          "size-12 rounded-2xl shrink-0 border flex items-center justify-center relative",
          isLive
            ? "bg-gradient-to-br from-sell/30 via-warn/20 to-accent/15 border-sell/40 shadow-glow-sell"
            : "bg-gradient-to-br from-accent/30 via-accent-2/20 to-buy/15 border-accent/35 shadow-glow"
        )}>
          <Zap className={cn("size-5 relative", isLive ? "text-sell" : "text-accent")} />
        </div>
        <div className="flex-1 min-w-0">
          <div className={cn(
            "text-[20px] font-semibold tracking-tightish truncate leading-tight",
            isLive ? "text-text" : "text-gradient-accent"
          )}>{alias}</div>
          <div className="text-[11.5px] text-text-muted mt-1 flex items-center gap-2 flex-wrap">
            {joined && <span>Joined {formatJoined(joined)}</span>}
            <span className="text-text-faint">·</span>
            <code className="font-mono text-[10.5px] bg-chip/80 px-2 py-0.5 rounded-md text-text-muted border border-line/60">
              {walletShort}
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
        <Stat label="Positions Value" value={fmt$(positionsValue, 0)}
              sub={scope === "recent" ? "24h 활동 마켓" : "전체 보유"} />
        <Stat label="Biggest Win"     value={biggestWin != null ? fmt$(biggestWin, 0) : "—"}
              sub={biggestWin != null && biggestWin > 0 ? "현 시점 leg PnL" : "(아직 흑자 leg 없음)"}
              positive={biggestWin != null && biggestWin > 0} />
        <Stat label="Predictions"     value={fmtN(predictions, 0)}
              sub={scope === "recent" ? "24h 체결" : "누적 체결"} />
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
