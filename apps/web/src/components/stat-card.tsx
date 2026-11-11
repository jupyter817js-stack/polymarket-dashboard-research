"use client";
import { cn } from "@/lib/fmt";
import type { LucideIcon } from "lucide-react";
import { TrendingUp, TrendingDown, Minus } from "lucide-react";

type Tone = "accent" | "buy" | "sell" | "warn" | "neutral";

const toneStyles: Record<Tone, { glow: string; icon: string; value: string }> = {
  accent:  { glow: "hover:shadow-glow",        icon: "text-accent",      value: "text-text" },
  buy:     { glow: "hover:shadow-glow-buy",    icon: "text-buy",         value: "text-buy"  },
  sell:    { glow: "hover:shadow-glow-sell",   icon: "text-sell",        value: "text-sell" },
  warn:    { glow: "",                          icon: "text-warn",        value: "text-warn" },
  neutral: { glow: "",                          icon: "text-text-muted",  value: "text-text" },
};

/** 아이콘 + 큰 값 + 라벨 + (선택) 보조 메트릭. */
export function StatCard({ icon: Icon, label, value, sub, tone = "neutral", delta, hint }: {
  icon: LucideIcon;
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
  tone?: Tone;
  /** 직전 값 대비 변화량. > 0 이면 ▲ buy, < 0 이면 ▼ sell. */
  delta?: number | null;
  hint?: string;
}) {
  const t = toneStyles[tone];
  return (
    <div className={cn(
      "surface px-4 py-3.5 hover-lift transition-all duration-200",
      t.glow
    )}>
      <div className="flex items-start gap-2.5">
        <div className="size-7 rounded-lg bg-bg-panel/80 border border-line/70 flex items-center justify-center shrink-0">
          <Icon className={cn("size-3.5", t.icon)} />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-[10.5px] uppercase tracking-wider2 text-text-muted font-medium truncate">
            {label}
          </div>
          <div className="mt-1 flex items-baseline gap-2 flex-wrap">
            <div className={cn(
              "text-[20px] sm:text-[22px] font-semibold tracking-tightish num leading-none truncate",
              t.value
            )}>
              {value}
            </div>
            {delta != null && <DeltaPill delta={delta} />}
          </div>
          {sub && (
            <div className="mt-1.5 text-[10.5px] text-text-dim truncate" title={hint}>
              {sub}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function DeltaPill({ delta }: { delta: number }) {
  const up = delta > 0, down = delta < 0;
  const Icon = up ? TrendingUp : down ? TrendingDown : Minus;
  return (
    <span className={cn(
      "inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] font-semibold tabular-nums",
      up ? "bg-buy/10 text-buy" : down ? "bg-sell/10 text-sell" : "bg-bg-panel text-text-muted"
    )}>
      <Icon className="size-2.5" />
      {(delta >= 0 ? "+" : "") + delta.toFixed(2) + "%"}
    </span>
  );
}
