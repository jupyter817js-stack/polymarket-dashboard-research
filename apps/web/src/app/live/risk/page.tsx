"use client";

import { useQuery } from "@tanstack/react-query";
import {
  Shield, AlertTriangle, Wallet, Layers, Gauge, Activity, TrendingDown, Coins,
} from "lucide-react";
import { api } from "@/lib/api";
import { fmt$, fmtN, fmtPx, cn } from "@/lib/fmt";
import { PageHeader } from "@/components/page-header";
import { StatCard } from "@/components/stat-card";

export default function LiveRisk() {
  const q = useQuery({
    queryKey: ["trader-state"],
    queryFn: api.traderState,
    refetchInterval: 3000,
  });
  const c = q.data?.bot?.config;
  const d = q.data?.daily;
  const w = q.data?.bot?.wallet;
  const ks = d?.kill_switch ?? q.data?.bot?.kill_switch ?? false;
  const realized = d?.realized_pnl ?? 0;
  const lossCap = c?.daily_loss_cap_usdc ?? 0;
  const lossPct = lossCap > 0 ? (realized / -lossCap) * 100 : 0;

  return (
    <div className="space-y-4">
      <PageHeader
        icon={ks ? AlertTriangle : Shield}
        title="Risk · 한도"
        subtitle="이벤트당 / 일일 / 지갑 헤드룸 + cumulative-vwap 가드"
        tone={ks ? "sell" : "accent"}
        actions={
          <span className={cn("badge",
            ks ? "bg-sell/15 text-sell border-sell/30" : "bg-buy/15 text-buy border-buy/30")}>
            {ks ? "KILL ACTIVE" : "정상"}
          </span>
        }
      />

      {ks && (
        <div className="surface px-4 py-3.5 border-sell/40 bg-gradient-to-br from-sell/[0.08] via-bg-elevated to-bg-elevated
                        flex items-start gap-3">
          <div className="size-10 rounded-xl bg-sell/15 border border-sell/30 flex items-center justify-center shrink-0
                          shadow-glow-sell">
            <AlertTriangle className="size-5 text-sell" />
          </div>
          <div className="flex-1">
            <div className="text-[14px] font-semibold text-sell">KILL SWITCH 작동 중</div>
            <div className="text-[12px] text-text-muted mt-1 leading-relaxed">
              일일 손실 한도 도달 또는 수동 트리거. <strong className="text-text">신규 주문 차단, 기존 호가는 모두 cancel.</strong>
            </div>
          </div>
        </div>
      )}

      {/* 이벤트 / 일일 한도 */}
      <div>
        <div className="label-section mb-2.5 px-1">이벤트 / 일일 한도</div>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <StatCard icon={Coins} label="per_event_cap"     value={fmt$(c?.per_event_cap_usdc)}  tone="accent" />
          <StatCard icon={TrendingDown} label="daily_loss_cap"  value={fmt$(c?.daily_loss_cap_usdc)} tone="warn"
            sub={lossPct !== 0 ? `현재 ${lossPct.toFixed(1)}% 도달` : "—"} />
          <StatCard icon={Gauge} label="daily_gross_cap" value={fmt$(c?.daily_gross_cap_usdc)} tone="accent" />
          <StatCard icon={Activity} label="오늘 체결액 (gross)"  value={fmt$(d?.gross_filled)}
            sub={c?.daily_gross_cap_usdc ? `${((d?.gross_filled ?? 0) / c.daily_gross_cap_usdc * 100).toFixed(1)}% 도달` : ""}
            tone={ratioTone((d?.gross_filled ?? 0), c?.daily_gross_cap_usdc)} />
        </div>
      </div>

      {/* 전략 가드 */}
      <div>
        <div className="label-section mb-2.5 px-1">전략 가드 (handoff §3)</div>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <StatCard icon={Gauge} label="cumulative_aim"      value={"≤ " + fmtPx(c?.cumulative_aim)} tone="buy"
            sub="정상 ladder/completion 목표" />
          <StatCard icon={Shield} label="cumulative_target_max" value={"≤ " + fmtPx(c?.cumulative_target_max)} tone="warn"
            sub="하드 한도, endgame relax" />
          <StatCard icon={Activity} label="endgame after"        value={(c?.endgame_after_min ?? 0) + " 분"} tone="neutral"
            sub="aim → target 완화 시점" />
          <StatCard icon={Layers} label="ladder × tier"         value={(c?.ladder_depth ?? 0) + " 단"} tone="neutral"
            sub={`base size ${fmtN(c?.quote_shares, 1)} 주 · completion ${fmtN(c?.completion_mult, 1)}×`} />
        </div>
      </div>

      {/* 지갑 헤드룸 + PnL 평가 */}
      <div>
        <div className="label-section mb-2.5 px-1">지갑 · PnL</div>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <StatCard icon={Wallet} label="CLOB 예치금"          value={fmt$(w?.clob_usdc_available)} tone="accent"
            sub={`buffer $100`} />
          <StatCard icon={Coins} label="USDC (on-chain)"       value={fmt$((w?.usdc_e_balance ?? 0) + (w?.usdc_native_balance ?? 0))} tone="neutral"
            sub={w?.address ? `${w.address.slice(0,6)}…${w.address.slice(-4)}` : "—"} />
          <StatCard icon={TrendingDown} label="실현 PnL"        value={fmt$(realized)}
            tone={realized >= 0 ? "buy" : "sell"}
            sub={lossCap > 0 ? `cap 대비 ${lossPct.toFixed(1)}%` : "—"} />
          <StatCard icon={ks ? AlertTriangle : Shield} label="kill switch"
            value={ks ? "🔴 작동" : "정상"} tone={ks ? "sell" : "buy"}
            sub={ks ? "신규 거래 차단" : "임계 미도달"} />
        </div>
      </div>
    </div>
  );
}

function ratioTone(num: number, denom: number | null | undefined): "neutral" | "warn" | "sell" {
  if (!denom) return "neutral";
  const r = num / denom;
  if (r >= 0.9) return "sell";
  if (r >= 0.6) return "warn";
  return "neutral";
}
