"use client";

import { useQuery } from "@tanstack/react-query";
import Image from "next/image";
import { Suspense, useMemo, useState } from "react";
import { useSearchParams, useRouter, usePathname } from "next/navigation";
import { Eye, Filter, X, History as HistoryIcon, BookOpen, Search, Inbox } from "lucide-react";
import { api, type TraderOpenOrder, type TraderClosedOrder } from "@/lib/api";
import { fmt$, fmtN, fmtPx, relTime, fmtDateTime, cn } from "@/lib/fmt";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";

type Tab = "open" | "history";

/** /live/orders — 폴리마켓 사이트 양식의 flat 테이블 (Open Orders / History 탭). */
export default function LiveOrdersPage() {
  return (
    <Suspense fallback={<div className="surface px-5 py-10 text-text-dim text-center text-[12px]">로딩…</div>}>
      <LiveOrdersInner />
    </Suspense>
  );
}

function LiveOrdersInner() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const tabParam = sp.get("tab");
  const tab: Tab = tabParam === "history" ? "history" : "open";

  const setTab = (t: Tab) => {
    const params = new URLSearchParams(sp.toString());
    if (t === "open") params.delete("tab"); else params.set("tab", t);
    const qs = params.toString();
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  };

  return (
    <div className="space-y-4">
      <PageHeader
        icon={tab === "open" ? Eye : BookOpen}
        title="My Orders"
        subtitle={tab === "open"
          ? "Polymarket CLOB 에 깔린 트레이더 주문 · 체결/취소되면 즉시 사라짐"
          : "체결 이력 — 폴리마켓 Activity 와 동일 양식"}
        tone="accent"
        actions={<TabBar tab={tab} setTab={setTab} />}
      />

      {tab === "open"    && <OpenOrdersPanel />}
      {tab === "history" && <HistoryPanel />}
    </div>
  );
}

function TabBar({ tab, setTab }: { tab: Tab; setTab: (t: Tab) => void }) {
  const openQ = useQuery({
    queryKey: ["trader-open-orders-count"],
    queryFn: api.traderOpenOrders,
    refetchInterval: 2000,
    staleTime: 1500,
  });
  return (
    <div className="ml-auto inline-flex rounded-md border border-line/80 bg-bg-panel p-0.5">
      <TabBtn active={tab === "open"} onClick={() => setTab("open")}>
        <Eye className="size-3.5" /> Open Orders
        {openQ.data && openQ.data.total_open > 0 && (
          <span className="ml-1 text-[10px] num text-accent font-semibold">{openQ.data.total_open}</span>
        )}
      </TabBtn>
      <TabBtn active={tab === "history"} onClick={() => setTab("history")}>
        <HistoryIcon className="size-3.5" /> History
      </TabBtn>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
//   OPEN ORDERS PANEL — flat (Polymarket style)
// ─────────────────────────────────────────────────────────────────────────

function OpenOrdersPanel() {
  const [tagFilter, setTagFilter] = useState<string>("all");
  const [search, setSearch] = useState("");

  const openQ = useQuery({
    queryKey: ["trader-open-orders"],
    queryFn: api.traderOpenOrders,
    refetchInterval: 2000,
    staleTime: 1500,
  });

  const filtered = useMemo(() => {
    const list = openQ.data?.orders ?? [];
    return list.filter(o => {
      if (tagFilter !== "all" && o.tag !== tagFilter) return false;
      if (search) {
        const s = search.toLowerCase();
        return (o.title ?? "").toLowerCase().includes(s)
          || (o.event_slug ?? "").toLowerCase().includes(s)
          || (o.outcome_name ?? "").toLowerCase().includes(s)
          || o.condition_id.toLowerCase().includes(s);
      }
      return true;
    });
  }, [openQ.data, tagFilter, search]);

  return (
    <>
      <Toolbar search={search} setSearch={setSearch}
        tagFilter={tagFilter} setTagFilter={setTagFilter}
        right={
          <div className="text-[11px] text-text-muted">
            <span className="num text-text">{openQ.data?.total_open ?? 0}</span>건
            <span className="text-text-faint mx-1.5">·</span>
            깔린 총액 <span className="num text-text">{fmt$(openQ.data?.total_remaining_usdc)}</span>
          </div>
        }
      />

      <div className="surface-flush">
        <div className="hidden md:grid grid-cols-[1fr_60px_110px_75px_85px_110px_90px_60px] gap-3 px-4 py-2.5
                        text-[10px] uppercase tracking-wider2 text-text-dim font-medium border-b border-line/70">
          <div>Market</div>
          <div>Side</div>
          <div>Outcome</div>
          <div className="text-right">Price</div>
          <div className="text-right">Cur</div>
          <div className="text-right">Filled / Size</div>
          <div className="text-right">Total</div>
          <div className="text-right">Age</div>
        </div>

        {openQ.isLoading && <Empty msg="로딩…" />}
        {!openQ.isLoading && filtered.length === 0 && (
          <div className="px-4 py-6">
            <EmptyState
              icon={search || tagFilter !== "all" ? Search : Inbox}
              title={search || tagFilter !== "all" ? "필터 조건에 맞는 주문 없음" : "깔린 주문 없음"}
              message={search || tagFilter !== "all"
                ? "검색어/태그 필터를 해제해보세요."
                : "봇이 사다리 호가를 깔거나 포커스 이벤트가 부재합니다."}
              tone="neutral"
            />
          </div>
        )}
        {!openQ.isLoading && filtered.map(o => <OpenRow key={o.order_id} o={o} />)}
      </div>
    </>
  );
}

function OpenRow({ o }: { o: TraderOpenOrder }) {
  const filledPct = o.size > 0 ? Math.min(100, (o.fill_size_shares / o.size) * 100) : 0;
  const sideClass = o.side === "BUY" ? "text-buy" : "text-sell";
  const isIdx0 = o.outcome_index === 0;
  // Per-outcome current price: bid for BUY orders (we'd hit it as taker exit),
  // ask for SELL orders. For BUY maker, distance bid−price = how far from
  // top of book we are; closer = more likely to fill. For SELL maker, the
  // distance to ask shows the same idea on the sell side.
  const cur = o.side === "SELL" ? o.best_ask : o.best_bid;
  // Distance vs our price (¢): negative = market moved AWAY from us
  // (BUY maker below bid = waiting for dip; SELL maker above ask = waiting for spike).
  const distCents = cur != null
    ? Math.round((o.side === "SELL" ? (cur - o.price) : (o.price - cur)) * 100)
    : null;
  const stale = (o.quote_age_s ?? 0) > 60;
  return (
    <div className="grid grid-cols-1 md:grid-cols-[1fr_60px_110px_75px_85px_110px_90px_60px] gap-3 px-4 py-3
                    items-center border-b border-line/30 last:border-b-0 hover:bg-bg-panel/40 transition-colors">
      <MarketCell title={o.title} cid={o.condition_id} icon={o.icon} tag={o.tag} kind={o.kind} negRisk={o.neg_risk} />

      <div className="md:contents grid grid-cols-2 gap-2 text-[12px]">
        <div className="md:block">
          <span className="md:hidden text-[9.5px] uppercase tracking-wider text-text-dim mr-1.5">Side</span>
          <span className={cn("font-semibold", sideClass)}>{o.side}</span>
        </div>
        <div className="md:block">
          <span className="md:hidden text-[9.5px] uppercase tracking-wider text-text-dim mr-1.5">Outcome</span>
          <span className={cn("text-text", isIdx0 ? "" : "")}>{o.outcome_name ?? `leg ${o.outcome_index}`}</span>
        </div>
        <div className="md:text-right">
          <span className="md:hidden text-[9.5px] uppercase tracking-wider text-text-dim mr-1.5">Price</span>
          <span className="num">{fmtPx(o.price)}</span>
        </div>
        <div className="md:text-right">
          <span className="md:hidden text-[9.5px] uppercase tracking-wider text-text-dim mr-1.5">Cur</span>
          <span className={cn("num text-[12px]",
            cur == null ? "text-text-dim" :
            distCents != null && distCents >= 0 ? "text-buy" :   // market favorable to us
            "text-text-muted")}>
            {fmtPx(cur)}
          </span>
          {distCents != null && Math.abs(distCents) > 0 && (
            <span className={cn("ml-1 text-[10px] num",
              distCents > 0 ? "text-buy" : "text-sell")}
              title={o.side === "SELL"
                ? `ask vs our SELL: ${distCents > 0 ? "above target" : "below target"}`
                : `bid vs our BUY: ${distCents > 0 ? "above bid (good)" : "below bid"}`}>
              {distCents > 0 ? "+" : ""}{distCents}¢
            </span>
          )}
          {stale && (
            <span className="ml-1 px-1 rounded text-[9px] bg-warn/20 text-warn font-medium"
                  title={`Quote ${Math.floor((o.quote_age_s ?? 0) / 60)}분 stale — 가격이 부정확할 수 있음`}>
              stale
            </span>
          )}
        </div>
        <div className="md:text-right">
          <div className="text-[12px] num">{fmtN(o.fill_size_shares, 1)} / {fmtN(o.size, 1)}</div>
          <div className="h-1 mt-1 bg-bg-panel rounded-full overflow-hidden md:w-20 md:ml-auto">
            <div className="h-full bg-buy transition-all" style={{ width: `${filledPct}%` }} />
          </div>
        </div>
        <div className="md:text-right num text-[12px]">{fmt$(o.size_usdc)}</div>
        <div className="md:text-right text-[11px] text-text-muted">{relTime(o.signed_at)}</div>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
//   HISTORY PANEL
// ─────────────────────────────────────────────────────────────────────────

function HistoryPanel() {
  const [tagFilter, setTagFilter] = useState<string>("all");
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<"filled" | "all">("filled");

  const closedQ = useQuery({
    queryKey: ["trader-closed-orders"],
    queryFn: api.traderClosedOrders,
    refetchInterval: 4_000,
    staleTime: 3_000,
  });

  const filtered = useMemo(() => {
    const list = closedQ.data?.orders ?? [];
    return list.filter(o => {
      if (statusFilter === "filled" && o.status !== "filled" && o.status !== "partially_filled") return false;
      if (tagFilter !== "all" && o.tag !== tagFilter) return false;
      if (search) {
        const s = search.toLowerCase();
        return (o.title ?? "").toLowerCase().includes(s)
          || (o.event_slug ?? "").toLowerCase().includes(s)
          || (o.outcome_name ?? "").toLowerCase().includes(s)
          || o.condition_id.toLowerCase().includes(s);
      }
      return true;
    });
  }, [closedQ.data, tagFilter, search, statusFilter]);

  const stats = useMemo(() => {
    const list = filtered.filter(o => o.fill_size_shares > 0);
    const total_usdc = list.reduce((s, o) => s + o.fill_usdc, 0);
    const total_shares = list.reduce((s, o) => s + o.fill_size_shares, 0);
    const avg_price = total_shares > 0 ? total_usdc / total_shares : null;
    return { count: list.length, total_usdc, avg_price };
  }, [filtered]);

  return (
    <>
      <Toolbar search={search} setSearch={setSearch}
        tagFilter={tagFilter} setTagFilter={setTagFilter}
        prefix={
          <>
            <Filter className="size-3.5 text-text-muted" />
            <TagPill active={statusFilter === "filled"} onClick={() => setStatusFilter("filled")}>Filled</TagPill>
            <TagPill active={statusFilter === "all"}    onClick={() => setStatusFilter("all")}>+Cancelled</TagPill>
            <span className="text-text-faint mx-1">·</span>
          </>
        }
        right={
          <div className="text-[11px] text-text-muted">
            <span className="num text-text">{stats.count}</span>건
            <span className="text-text-faint mx-1.5">·</span>
            체결 <span className="num text-text">{fmt$(stats.total_usdc)}</span>
            {stats.avg_price != null && (<>
              <span className="text-text-faint mx-1.5">·</span>
              평단 <span className="num text-text">{fmtPx(stats.avg_price)}</span>
            </>)}
          </div>
        }
      />

      <div className="surface-flush">
        <div className="hidden md:grid grid-cols-[1fr_70px_120px_90px_110px_100px_90px_70px] gap-3 px-4 py-2.5
                        text-[10px] uppercase tracking-wider2 text-text-dim font-medium border-b border-line/70">
          <div>Market</div>
          <div>Side</div>
          <div>Outcome</div>
          <div className="text-right">Price</div>
          <div className="text-right">Filled / Size</div>
          <div className="text-right">Total</div>
          <div>Status</div>
          <div className="text-right">When</div>
        </div>

        {closedQ.isLoading && <Empty msg="로딩…" />}
        {!closedQ.isLoading && filtered.length === 0 && (
          <div className="px-4 py-6">
            <EmptyState
              icon={search || tagFilter !== "all" || statusFilter !== "filled" ? Search : BookOpen}
              title={search || tagFilter !== "all" || statusFilter !== "filled" ? "필터 조건에 맞는 이력 없음" : "체결 이력 없음"}
              message="트레이더가 첫 체결을 받으면 여기에 표시됩니다."
              tone="neutral"
            />
          </div>
        )}
        {!closedQ.isLoading && filtered.map(o => <ClosedRow key={o.order_id} o={o} />)}
      </div>
    </>
  );
}

function ClosedRow({ o }: { o: TraderClosedOrder }) {
  const stamp = o.filled_at ?? o.cancelled_at ?? o.signed_at;
  const sideClass = o.side === "BUY" ? "text-buy" : "text-sell";
  const statusInfo: Record<string, [string, string]> = {
    filled:            ["text-buy",        "Filled"],
    partially_filled:  ["text-buy/80",     "Partial"],
    cancelled:         ["text-text-muted", "Cancelled"],
    rejected:          ["text-sell",       "Rejected"],
    expired:           ["text-text-muted", "Expired"],
  };
  const [statusClass, statusLabel] = statusInfo[o.status] ?? ["text-text-muted", o.status];
  return (
    <div className="grid grid-cols-1 md:grid-cols-[1fr_70px_120px_90px_110px_100px_90px_70px] gap-3 px-4 py-3
                    items-center border-b border-line/30 last:border-b-0 hover:bg-bg-panel/40 transition-colors">
      <MarketCell title={o.title} cid={o.condition_id} icon={o.icon} tag={o.tag} kind={o.kind} />

      <div className="md:contents grid grid-cols-2 gap-2 text-[12px]">
        <div className="md:block">
          <span className="md:hidden text-[9.5px] uppercase tracking-wider text-text-dim mr-1.5">Side</span>
          <span className={cn("font-semibold", sideClass)}>{o.side}</span>
        </div>
        <div className="md:block">
          <span className="md:hidden text-[9.5px] uppercase tracking-wider text-text-dim mr-1.5">Outcome</span>
          <span className="text-text">{o.outcome_name ?? `leg ${o.outcome_index}`}</span>
        </div>
        <div className="md:text-right num">{fmtPx(o.price)}</div>
        <div className="md:text-right num">{fmtN(o.fill_size_shares, 1)} / {fmtN(o.size, 1)}</div>
        <div className="md:text-right num">{fmt$(o.fill_usdc > 0 ? o.fill_usdc : o.size_usdc)}</div>
        <div className={cn("text-[11px] font-medium", statusClass)} title={o.cancelled_reason ?? ""}>{statusLabel}</div>
        <div className="md:text-right text-[11px] text-text-muted" title={fmtDateTime(stamp)}>{relTime(stamp)}</div>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
//   Shared bits
// ─────────────────────────────────────────────────────────────────────────

function Toolbar({ search, setSearch, tagFilter, setTagFilter, prefix, right }: {
  search: string; setSearch: (s: string) => void;
  tagFilter: string; setTagFilter: (s: string) => void;
  prefix?: React.ReactNode;
  right?: React.ReactNode;
}) {
  return (
    <div className="surface p-3 flex flex-wrap items-center gap-2">
      <SearchBox value={search} onChange={setSearch} placeholder="제목 / 이벤트 / outcome / cid 검색" />
      {prefix}
      <Filter className="size-3.5 text-text-muted" />
      <TagPill active={tagFilter === "all"}        onClick={() => setTagFilter("all")}>전체</TagPill>
      <TagPill active={tagFilter === "new_pair"}   onClick={() => setTagFilter("new_pair")}>NewPair</TagPill>
      <TagPill active={tagFilter === "completion"} onClick={() => setTagFilter("completion")}>Completion</TagPill>
      <TagPill active={tagFilter === "sweep"}      onClick={() => setTagFilter("sweep")}>Sweep</TagPill>
      {right && <div className="ml-auto">{right}</div>}
    </div>
  );
}

function MarketCell({ title, cid, icon, tag, kind, negRisk }: {
  title: string | null; cid: string; icon: string | null; tag: string; kind: string; negRisk?: boolean;
}) {
  return (
    <div className="flex items-center gap-2.5 min-w-0">
      {icon ? (
        <Image src={icon} alt="" width={32} height={32}
          className="size-8 rounded-md object-cover shrink-0 ring-1 ring-line/60" unoptimized />
      ) : <div className="size-8 rounded-md bg-bg-panel border border-line shrink-0" />}
      <div className="min-w-0">
        <div className="text-[12.5px] text-text font-medium tracking-tightish truncate">
          {title ?? cid.slice(0, 14)}
        </div>
        <div className="flex items-center gap-1.5 mt-0.5">
          <TagChip tag={tag} />
          <KindChip kind={kind} />
          {negRisk && <span className="text-[9px] text-text-dim font-mono">negRisk</span>}
        </div>
      </div>
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return <div className="px-5 py-12 text-center text-[12px] text-text-dim">{msg}</div>;
}

function SearchBox({ value, onChange, placeholder, id = "live-orders-search" }:
  { value: string; onChange: (s: string) => void; placeholder: string; id?: string }) {
  return (
    <div className="relative flex-1 min-w-[200px]">
      <label htmlFor={id} className="sr-only">{placeholder}</label>
      <input id={id} name={id} type="search" autoComplete="off"
        value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder}
        className="w-full bg-bg-panel border border-line/80 rounded-md pl-3 pr-8 py-1.5 text-[12px]
                   focus:outline-none focus:border-accent transition-colors" />
      {value && <button onClick={() => onChange("")}
        className="absolute right-2 top-1/2 -translate-y-1/2 text-text-muted hover:text-text">
        <X className="size-3.5" /></button>}
    </div>
  );
}

function TabBtn({ active, onClick, children }:
  { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick}
      className={cn("px-2.5 py-1 rounded-[5px] text-[12px] flex items-center gap-1 transition-colors",
        active ? "bg-bg-elevated text-text shadow-sm" : "text-text-muted hover:text-text")}>
      {children}
    </button>
  );
}

function TagPill({ active, onClick, children }:
  { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick}
      className={cn("px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
        active ? "bg-accent/15 text-accent border border-accent/30"
               : "bg-bg-panel/70 text-text-muted border border-line/80 hover:text-text")}>
      {children}
    </button>
  );
}

function TagChip({ tag }: { tag: string }) {
  const map: Record<string, [string, string]> = {
    new_pair:   ["bg-accent/10 text-accent border-accent/25", "NewPair"],
    completion: ["bg-warn/10   text-warn   border-warn/25",   "Completion"],
    sweep:      ["bg-sell/10   text-sell   border-sell/25",   "Sweep"],
    reconcile:  ["bg-bg-panel  text-text-muted border-line",  "Reconcile"],
  };
  const [cls, label] = map[tag] ?? ["bg-bg-panel text-text-muted border-line", tag];
  return (
    <span className={cn("inline-flex items-center px-1.5 py-px text-[9.5px] rounded border", cls)}>
      {label}
    </span>
  );
}

function KindChip({ kind }: { kind: string }) {
  return (
    <span className={cn("text-[9.5px] px-1.5 py-px rounded border font-mono",
      kind === "taker" ? "bg-sell/10 text-sell border-sell/25"
                       : "bg-bg-panel text-text-muted border-line")}>
      {kind}
    </span>
  );
}
