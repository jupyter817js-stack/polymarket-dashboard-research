"use client";
import { cn } from "@/lib/fmt";
import type { LucideIcon } from "lucide-react";

type Tone = "accent" | "buy" | "sell" | "warn" | "neutral";

const toneClass: Record<Tone, { wrap: string; icon: string }> = {
  accent:  { wrap: "bg-accent/12 border-accent/30 shadow-glow text-accent",
             icon: "text-accent" },
  buy:     { wrap: "bg-buy/10 border-buy/30 shadow-glow-buy text-buy",
             icon: "text-buy" },
  sell:    { wrap: "bg-sell/10 border-sell/30 shadow-glow-sell text-sell",
             icon: "text-sell" },
  warn:    { wrap: "bg-warn/10 border-warn/30 text-warn",
             icon: "text-warn" },
  neutral: { wrap: "bg-bg-panel border-line text-text-muted",
             icon: "text-text-muted" },
};

/** 일관된 페이지 헤더 — 큰 아이콘 배지 + 제목 + 부제 + 우측 actions. */
export function PageHeader({ icon: Icon, title, subtitle, tone = "accent", actions }: {
  icon: LucideIcon;
  title: string;
  subtitle?: React.ReactNode;
  tone?: Tone;
  actions?: React.ReactNode;
}) {
  const t = toneClass[tone];
  return (
    <header className="flex items-start gap-3 sm:gap-4 mb-4 sm:mb-5">
      <div className={cn(
        "size-10 sm:size-11 rounded-xl border shrink-0 flex items-center justify-center transition-all",
        t.wrap
      )}>
        <Icon className={cn("size-5", t.icon)} />
      </div>
      <div className="flex-1 min-w-0">
        <h1 className="text-[20px] sm:text-[22px] font-semibold tracking-tightish leading-tight text-text">
          {title}
        </h1>
        {subtitle && (
          <div className="mt-1 text-[12px] sm:text-[12.5px] text-text-muted leading-snug">
            {subtitle}
          </div>
        )}
      </div>
      {actions && <div className="shrink-0 flex items-center gap-2">{actions}</div>}
    </header>
  );
}
