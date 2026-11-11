"use client";

import { useQuery } from "@tanstack/react-query";
import { Activity, TrendingUp, Receipt, Gauge, AlertTriangle } from "lucide-react";
import { api } from "@/lib/api";
import { fmt$, cn } from "@/lib/fmt";
import { PageHeader } from "@/components/page-header";
import { StatCard } from "@/components/stat-card";

export default function LivePnl() {
  const q = useQuery({
    queryKey: ["trader-state"],
    queryFn: api.traderState,
    refetchInterval: 3000,
  });
  const d = q.data?.daily;
  const realized = d?.realized_pnl ?? 0;
  const ks = d?.kill_switch ?? false;
  return (
    <div className="space-y-4">
      <PageHeader
        icon={Activity}
        title="PnL · 오늘"
        subtitle="체결 + 메이커 리베이트 기준. MERGE/REDEEM/settlement 통합은 다음 단계."
        tone={realized >= 0 ? "buy" : "sell"}
      />

      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <StatCard
          icon={TrendingUp}
          label="실현 손익"
          value={fmt$(realized)}
          sub="rebate-only 기준"
          tone={realized >= 0 ? "buy" : "sell"}
        />
        <StatCard
          icon={Receipt}
          label="메이커 리베이트"
          value={fmt$(d?.rebate, 4)}
          sub="누적 fill 의 0.75% 추정 (실제 dust)"
          tone="accent"
        />
        <StatCard
          icon={Gauge}
          label="체결 건수"
          value={d?.fills_count ?? 0}
          sub={`총 체결액 ${fmt$(d?.gross_filled)}`}
          tone="neutral"
        />
        <StatCard
          icon={ks ? AlertTriangle : Activity}
          label="kill switch"
          value={ks ? "🔴 작동" : "정상"}
          sub={ks ? "신규 거래 차단" : "정상 운영"}
          tone={ks ? "sell" : "buy"}
        />
      </div>

      <div className="surface px-4 py-3.5 text-[12px] text-text-muted leading-relaxed">
        <div className="text-text font-medium mb-1.5">참고</div>
        누적 PnL 시계열은 settlement / MERGE / REDEEM 통합 후 추가됩니다.
        현재는 트레이더의 일별 fill 합산 기준만 표시.
        <span className={cn("ml-2 font-semibold", ks ? "text-sell" : "text-buy")}>
          {ks ? "🔴 KILL SWITCH 작동 중" : "● 정상 운영 중"}
        </span>
      </div>
    </div>
  );
}
