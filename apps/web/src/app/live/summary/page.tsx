"use client";

import { useQuery } from "@tanstack/react-query";
import {
  Activity, AlertTriangle, Cpu, Shield, Target, Wallet as WalletIcon, Zap,
  TrendingUp, Receipt, Gauge, Radar, ChevronRight, Layers
} from "lucide-react";
import { api, type TraderBot, type StrategyView } from "@/lib/api";
import { fmt$, fmtN, fmtPx, relTime, cn } from "@/lib/fmt";
import { PageHeader } from "@/components/page-header";
import { StatCard } from "@/components/stat-card";
import { EmptyState } from "@/components/empty-state";

/** /live/summary — 트레이더 라이브 상태 대시보드. */
export default function LiveSummary() {
  const q = useQuery({
    queryKey: ["trader-state"],
    queryFn: api.traderState,
    refetchInterval: 2000,
  });

  const bot = q.data?.bot;
  const daily = q.data?.daily;
  const ks = daily?.kill_switch ?? bot?.kill_switch ?? false;
  const running = bot?.running ?? false;

  return (
    <div className="space-y-4">
      <PageHeader
        icon={Zap}
        title="라이브 트레이더"
        subtitle={<>RN1-style pair-completion · paper 모드 · 자동 universe scan (ATP/WTA · vol ≥ $25k)</>}
        tone={ks ? "sell" : running ? "buy" : "warn"}
        actions={
          <span className={cn("badge",
            ks ? "bg-sell/15 text-sell border-sell/30" :
            running ? "bg-buy/15 text-buy border-buy/30" :
                      "bg-warn/15 text-warn border-warn/30")}>
            {ks ? "KILL" : running ? "RUNNING" : "IDLE"}
          </span>
        }
      />

      <StatusStripe bot={bot} loading={q.isLoading} />

      <StrategyCard strategy={bot?.strategy ?? null} />

      {/* KPI grid — daily + risk + wallet 요약 */}
      <div>
        <div className="label-section mb-2.5 px-1">현재 상태</div>
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-3">
          <StatCard
            icon={TrendingUp}
            label="실현 손익 (오늘)"
            value={fmt$(daily?.realized_pnl)}
            sub="MERGE/REDEEM 통합 전 — 리베이트 기준"
            tone={(daily?.realized_pnl ?? 0) >= 0 ? "buy" : "sell"}
          />
          <StatCard
            icon={Receipt}
            label="체결 건수"
            value={daily?.fills_count ?? 0}
            sub={`체결액 ${fmt$(daily?.gross_filled)}`}
            tone="accent"
          />
          <StatCard
            icon={Gauge}
            label="누적 vwap 목표"
            value={"≤ " + fmtPx(bot?.config?.cumulative_aim)}
            sub={"한도 " + fmtPx(bot?.config?.cumulative_target_max)}
            tone="accent"
          />
          <StatCard
            icon={Shield}
            label="이벤트당 상한"
            value={fmt$(bot?.config?.per_event_cap_usdc)}
            sub={`일일 손실 ${fmt$(bot?.config?.daily_loss_cap_usdc)} · 거래 ${fmt$(bot?.config?.daily_gross_cap_usdc)}`}
            tone={ks ? "sell" : "neutral"}
          />
          <StatCard
            icon={WalletIcon}
            label="CLOB 예치금"
            value={fmt$(bot?.wallet?.clob_usdc_available)}
            sub={`총 자산 ${fmt$(bot?.wallet?.total_value_usdc)}`}
            tone="accent"
          />
          <StatCard
            icon={Layers}
            label="포지션 평가액"
            value={fmt$(bot?.wallet?.positions_value_usdc)}
            sub={`${bot?.wallet?.positions_count ?? 0}개 leg`}
            tone="neutral"
          />
          <StatCard
            icon={Radar}
            label="후보 마켓"
            value={bot?.candidates ?? 0}
            sub={`tick ${bot?.current_tick ?? "—"} · ${bot?.last_cycle_ms ?? "—"}ms`}
            tone="neutral"
          />
          <StatCard
            icon={ks ? AlertTriangle : Activity}
            label="kill switch"
            value={ks ? "작동" : "정상"}
            sub={ks ? "신규 거래 차단" : `실현 PnL ${fmt$(daily?.realized_pnl)} vs cap ${fmt$(bot?.config?.daily_loss_cap_usdc)}`}
            tone={ks ? "sell" : "buy"}
          />
        </div>
      </div>
    </div>
  );
}

function StatusStripe({ bot, loading }: { bot?: TraderBot; loading: boolean }) {
  const live = bot?.mode === "live";
  const ks = bot?.kill_switch;
  const running = bot?.running;
  return (
    <div className={cn(
      "relative px-4 py-3 flex flex-wrap items-center gap-3 rounded-xl border shadow-card overflow-hidden",
      "transition-all duration-200",
      ks ? "border-sell/40 bg-gradient-to-br from-sell/[0.08] via-bg-elevated to-bg-elevated" :
      running ? "border-buy/30 bg-gradient-to-br from-buy/[0.05] via-bg-elevated to-bg-elevated"
              : "border-line/80 bg-bg-elevated"
    )}>
      {running && !ks && (
        <div className="pointer-events-none absolute inset-y-0 left-0 right-0 opacity-60"
             style={{ background: "linear-gradient(90deg, transparent, rgba(34,197,94,0.08) 50%, transparent)" }} />
      )}
      <div className="flex items-center gap-2 relative">
        <Cpu className="size-4 text-accent" />
        <span className="text-[14px] font-semibold tracking-tightish">rn1-trader</span>
        <span className={cn("badge",
          live ? "bg-sell/15 text-sell border-sell/30 shadow-glow-sell"
               : "bg-accent/15 text-accent border-accent/30")}>
          {live ? "LIVE 실거래" : "PAPER 모의"}
        </span>
      </div>
      <Dot on={!!bot?.ws_market_connected} label="시세 WS" />
      <Dot on={!!bot?.ws_user_connected}   label="체결 WS" />
      <div className="text-[11px] text-text-muted font-mono">
        tick <span className="num text-text">{bot?.current_tick ?? "—"}</span>
        <span className="text-text-faint mx-1">·</span>
        사이클 <span className="num text-text">{bot?.last_cycle_ms ?? "—"}</span>ms
        <span className="text-text-faint mx-1">·</span>
        후보 <span className="num text-text">{bot?.candidates ?? 0}</span>
      </div>
      <div className="ml-auto text-[12px] font-medium">
        {loading ? <span className="text-text-dim">로딩…</span>
         : ks ? <span className="text-sell flex items-center gap-1.5"><span className="dot-sell animate-pulse" /> KILL SWITCH 작동</span>
         : running ? <span className="text-buy flex items-center gap-1.5"><span className="dot-buy" /> 가동 중</span>
         : <span className="text-warn flex items-center gap-1.5"><span className="dot-warn" /> 정지</span>}
      </div>
      {bot?.last_error && (
        <div className="basis-full mt-1 text-[12px] text-sell flex items-center gap-1.5">
          <AlertTriangle className="size-3.5" />
          <span className="font-mono">{bot.last_error}</span>
        </div>
      )}
    </div>
  );
}

function Dot({ on, label }: { on: boolean; label: string }) {
  return (
    <div className="flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className={on ? "dot-buy" : "dot-sell"} />
      {label}
    </div>
  );
}

function StrategyCard({ strategy }: { strategy: StrategyView | null }) {
  if (!strategy) {
    return (
      <EmptyState
        icon={Target}
        title="포커스 이벤트 없음"
        message={<>트레이더가 자동 universe scan 중입니다. ATP/WTA in-play 가 시작되면 즉시 잡습니다 — 5분마다 재시도.</>}
        tone="warn"
      />
    );
  }
  const cum = strategy.cumulative_vwap;
  const aim = strategy.cumulative_target;
  const both = strategy.leg0_shares > 0 && strategy.leg1_shares > 0;
  const cumClass = !both ? "text-text-dim" :
                   cum <= aim ? "text-buy" :
                   cum < aim + 0.05 ? "text-warn" : "text-sell";
  const naked = strategy.naked_shares;

  return (
    <div className="surface-hero px-5 py-4 sm:py-5">
      <div className="flex items-center gap-2 mb-3 flex-wrap">
        <div className="size-7 rounded-lg bg-accent/15 border border-accent/30 flex items-center justify-center">
          <Target className="size-3.5 text-accent" />
        </div>
        <h2 className="label-section">전략 상태</h2>
        <span className="text-[12.5px] text-text font-medium truncate">{strategy.question ?? strategy.focus_slug}</span>
        {strategy.event_slug && (
          <span className="text-[10px] text-text-dim font-mono">{strategy.event_slug}</span>
        )}
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mb-4">
        <Leg name={strategy.leg0_name} shares={strategy.leg0_shares} vwap={strategy.leg0_vwap} idx={0} />
        <Leg name={strategy.leg1_name} shares={strategy.leg1_shares} vwap={strategy.leg1_vwap} idx={1} />
      </div>
      <div className="space-y-1.5 text-[13px]">
        <Row k="누적 vwap (두 평단 합)"        v={both ? fmtPx(cum) : "— (양 레그 체결 전)"}
             big accent={cumClass} />
        <Row k="목표 상한 (이 아래여야 흑자)"   v={"≤ " + fmtPx(aim)} />
        <Row k="매칭 페어 (무위험 부분)"        v={fmtN(strategy.matched_pairs, 1) + " 쌍"} />
        <Row k="미매칭 naked (방향성 노출)"
             v={naked > 0.5 ? `${fmtN(naked, 1)}주 (${strategy.naked_leg_name || "?"})` : "없음 (균형)"}
             accent={naked > 0.5 ? "text-warn" : "text-buy"} />
      </div>
    </div>
  );
}

function Leg({ name, shares, vwap, idx }: { name: string; shares: number; vwap: number; idx: number }) {
  const tone = idx === 0 ? "buy" : "sell";
  return (
    <div className={cn(
      "bg-bg-panel/70 border rounded-lg px-3 py-2.5 transition-colors",
      tone === "buy" ? "border-buy/20 hover:border-buy/35" : "border-sell/20 hover:border-sell/35"
    )}>
      <div className="flex items-center gap-2 mb-1">
        <span className={tone === "buy" ? "dot-buy" : "dot-sell"} />
        <div className="text-[13px] font-semibold tracking-tightish truncate flex-1">{name || "—"}</div>
        <span className="text-[9.5px] text-text-dim font-mono">[{idx}]</span>
      </div>
      <div className="text-[12px] text-text-muted">
        <span className="num text-text">{fmtN(shares, 1)}</span>주
        <span className="text-text-dim mx-1.5">@</span>
        평단 <span className="num text-text">{shares > 0 ? fmtPx(vwap) : "—"}</span>
      </div>
    </div>
  );
}

function Row({ k, v, accent, big }: { k: string; v: React.ReactNode; accent?: string; big?: boolean }) {
  return (
    <div className="flex items-center justify-between text-[12px] py-1.5 border-b border-line/30 last:border-b-0">
      <span className="text-text-muted flex items-center gap-1.5">
        <ChevronRight className="size-3 text-text-faint" />
        {k}
      </span>
      <span className={cn("num", big && "text-[14.5px] font-semibold", accent)}>{v}</span>
    </div>
  );
}
