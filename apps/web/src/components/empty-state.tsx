"use client";
import { cn } from "@/lib/fmt";
import type { LucideIcon } from "lucide-react";

type Tone = "accent" | "buy" | "warn" | "neutral";

const toneClass: Record<Tone, { wrap: string; icon: string }> = {
  accent:  { wrap: "bg-accent/8 border-accent/20", icon: "text-accent/70" },
  buy:     { wrap: "bg-buy/8 border-buy/20",       icon: "text-buy/80" },
  warn:    { wrap: "bg-warn/8 border-warn/20",     icon: "text-warn/80" },
  neutral: { wrap: "bg-bg-panel border-line",      icon: "text-text-dim" },
};

/** 빈 상태 / 안내 카드 — 아이콘 + 제목 + 본문 + 선택적 action. */
export function EmptyState({ icon: Icon, title, message, tone = "neutral", action }: {
  icon: LucideIcon;
  title: string;
  message?: React.ReactNode;
  tone?: Tone;
  action?: React.ReactNode;
}) {
  const t = toneClass[tone];
  return (
    <div className="surface px-6 py-10 sm:py-12 text-center">
      <div className={cn(
        "size-12 sm:size-14 mx-auto mb-3 rounded-2xl border flex items-center justify-center",
        t.wrap
      )}>
        <Icon className={cn("size-5 sm:size-6", t.icon)} />
      </div>
      <div className="text-[14px] sm:text-[15px] font-semibold tracking-tightish text-text mb-1">
        {title}
      </div>
      {message && (
        <div className="text-[12px] sm:text-[12.5px] text-text-muted max-w-md mx-auto leading-relaxed">
          {message}
        </div>
      )}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}
