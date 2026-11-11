"use client";
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import Image from "next/image";
import {
  CalendarClock, Flame, Timer, Search, X, Filter, History as HistoryIcon,
  Trophy, TrendingUp, CheckCircle2, Inbox, ChevronDown, Layers,
} from "lucide-react";
import { api, type LiveUpcomingMarket } from "@/lib/api";
import { fmt$, fmtPx, cn } from "@/lib/fmt";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";

type Universe = "all" | "atp" | "wta";
type StateF   = "all" | "upcoming" | "live" | "post";

/** /live/markets — ATP/WTA 경기 일정 (폴리마켓 sports schedule mirror).
 *
 *  2026-05-24 refactor: 한 경기(matchup)에 속하는 모든 마켓(머니라인 + 핸디캡
 *  + 세트 O/U + …)을 이벤트 단위로 묶는 트리 구조. 베이스 슬러그
 *  (`...-YYYY-MM-DD`)로 그룹화하고 머니라인을 main, 그 외는 sub-markets로 분류.
 */
export default function LiveMarketsPage() {
  const [search, setSearch]     = useState("");
  const [universe, setUniverse] = useState<Universe>("all");
  const [stateF, setStateF]     = useState<StateF>("all");

  const universeParam = universe === "all" ? "atp,wta" : universe;
  const q = useQuery({
    queryKey: ["live-markets-upcoming", universeParam],
    queryFn: () => api.liveMarketsUpcoming({ universe: universeParam, hours_back: 6, hours_ahead: 96 }),
    refetchInterval: 60_000,
    staleTime: 50_000,
  });

  const filtered = useMemo(() => {
    const list = q.data?.markets ?? [];
    return list.filter(m => {
      if (stateF !== "all" && m.game_state !== stateF) return false;
      if (search) {
        const s = search.toLowerCase();
        return (m.question ?? "").toLowerCase().includes(s)
          || m.slug.toLowerCase().includes(s)
          || (m.event_slug ?? "").toLowerCase().includes(s);
      }
      return true;
    });
  }, [q.data, search, stateF]);

  // ── Group by event base-slug ───────────────────────────────────────
  const eventGroups = useMemo(() => groupByEvent(filtered), [filtered]);

  // ── Visual section split by game_state ─────────────────────────────
  const live     = eventGroups.filter(g => g.gameState === "live");
  const upcoming = eventGroups.filter(g => g.gameState === "upcoming");
  const post     = eventGroups.filter(g => g.gameState === "post");

  return (
    <div className="space-y-4">
      <PageHeader
        icon={CalendarClock}
        title="라이브 경기 일정"
        subtitle={<>ATP / WTA 매치 · 이벤트별 트리 그룹화 · 봇이 잡은 마켓은 <span className="text-buy font-medium">●</span> 강조</>}
        tone="accent"
        actions={
          <span className="badge bg-accent/15 text-accent border-accent/30">
            {eventGroups.length} 경기 · {q.data?.market_count ?? 0} markets
          </span>
        }
      />

      {/* Toolbar */}
      <div className="surface p-3 flex flex-wrap items-center gap-2">
        <div className="relative flex-1 min-w-[220px]">
          <Search className="size-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-text-dim pointer-events-none" />
          <label htmlFor="markets-search" className="sr-only">검색</label>
          <input id="markets-search" name="markets-search" type="search" autoComplete="off"
            placeholder="이벤트 / 선수 / slug 검색"
            value={search} onChange={e => setSearch(e.target.value)}
            className="input pl-9 pr-8" />
          {search && <button onClick={() => setSearch("")}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text">
            <X className="size-3.5" /></button>}
        </div>

        <Filter className="size-3.5 text-text-muted" />
        <Pill active={universe === "all"} onClick={() => setUniverse("all")}>전체</Pill>
        <Pill active={universe === "atp"} onClick={() => setUniverse("atp")}>ATP</Pill>
        <Pill active={universe === "wta"} onClick={() => setUniverse("wta")}>WTA</Pill>
        <span className="text-text-faint mx-1">·</span>
        <Pill active={stateF === "all"}      onClick={() => setStateF("all")}>전체</Pill>
        <Pill active={stateF === "live"}     onClick={() => setStateF("live")}>
          <span className="dot-buy mr-0.5" /> Live
        </Pill>
        <Pill active={stateF === "upcoming"} onClick={() => setStateF("upcoming")}>Upcoming</Pill>
        <Pill active={stateF === "post"}     onClick={() => setStateF("post")}>Ended</Pill>
      </div>

      {q.isLoading && (
        <div className="surface px-5 py-10 text-center text-[12px] text-text-dim">로딩…</div>
      )}
      {!q.isLoading && eventGroups.length === 0 && (
        <EmptyState
          icon={search || universe !== "all" || stateF !== "all" ? Search : Inbox}
          title={search ? `"${search}" 검색 결과 없음` : "표시할 경기 없음"}
          message={search
            ? "다른 검색어를 시도해보세요."
            : "lifecycle-collector 가 마켓 메타를 채우는 중입니다 — RN1 거래가 시작되면 자동으로 표시됩니다."}
          tone="neutral"
        />
      )}

      {!q.isLoading && live.length > 0 && (
        <Section title="LIVE" icon={Flame} tone="sell" count={live.length}>
          {live.map(g => <EventCard key={g.baseSlug} g={g} defaultOpen={true} />)}
        </Section>
      )}
      {!q.isLoading && upcoming.length > 0 && (
        <Section title="UPCOMING" icon={Timer} tone="accent" count={upcoming.length}>
          {upcoming.map(g => <EventCard key={g.baseSlug} g={g} defaultOpen={g.hasBotFills} />)}
        </Section>
      )}
      {!q.isLoading && post.length > 0 && (
        <Section title="ENDED" icon={HistoryIcon} tone="neutral" count={post.length}>
          {post.map(g => <EventCard key={g.baseSlug} g={g} defaultOpen={false} />)}
        </Section>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────
//   Event grouping
// ──────────────────────────────────────────────────────────────────

interface EventGroup {
  baseSlug: string;
  title: string | null;
  icon: string | null;
  gameStart: string | null;
  gameState: "live" | "upcoming" | "post";
  secsToStart: number | null;
  totalVolume: number;
  hasBotFills: boolean;
  hasNegRisk: boolean;
  main: LiveUpcomingMarket | null;     // moneyline (slug === baseSlug)
  subs: LiveUpcomingMarket[];          // handicap / O/U / etc.
}

/** Extract the base-event slug shared by all markets of one matchup.
 *  Heuristic: greedy match up to and INCLUDING the first `YYYY-MM-DD` token. */
function eventBaseSlug(slug: string): string {
  const m = slug.match(/^(.+?-\d{4}-\d{2}-\d{2})(?:-|$)/);
  return m ? m[1] : slug;
}

function groupByEvent(markets: LiveUpcomingMarket[]): EventGroup[] {
  const groups = new Map<string, EventGroup>();
  for (const m of markets) {
    const key = eventBaseSlug(m.slug);
    let g = groups.get(key);
    if (!g) {
      g = {
        baseSlug: key,
        title: null,
        icon: null,
        gameStart: null,
        gameState: "upcoming",
        secsToStart: null,
        totalVolume: 0,
        hasBotFills: false,
        hasNegRisk: false,
        main: null,
        subs: [],
      };
      groups.set(key, g);
    }
    if (m.slug === key) {
      g.main = m;
      g.title = m.question;
      g.icon = m.icon;
    } else {
      g.subs.push(m);
    }
    g.totalVolume += m.volume_num ?? 0;
    if (m.has_bot_fills) g.hasBotFills = true;
    if (m.neg_risk) g.hasNegRisk = true;
    // game_start / state — they should all match within an event; pick first non-null.
    if (!g.gameStart && m.game_start) g.gameStart = m.game_start;
    if (g.secsToStart == null && m.secs_to_start != null) g.secsToStart = m.secs_to_start;
    // Promote game_state: live > post > upcoming (so a live sub-market drives the
    // whole event into LIVE section even if main hasn't flipped state yet).
    g.gameState = pickState(g.gameState, m.game_state as EventGroup["gameState"]);
  }

  // If no main moneyline market was found, synthesize a title from the slug.
  for (const g of groups.values()) {
    if (!g.title) {
      // Use the first sub-market's question or fall back to base slug
      g.title = g.subs[0]?.question ?? prettySlug(g.baseSlug);
      g.icon  = g.subs[0]?.icon ?? null;
    }
    // Sort sub-markets by slug for deterministic order.
    g.subs.sort((a, b) => a.slug.localeCompare(b.slug));
  }

  return [...groups.values()].sort((a, b) => {
    // primary: gameStart asc; nulls last
    const ax = a.gameStart ? new Date(a.gameStart).getTime() : Infinity;
    const bx = b.gameStart ? new Date(b.gameStart).getTime() : Infinity;
    if (ax !== bx) return ax - bx;
    // tie-breaker: title
    return (a.title ?? "").localeCompare(b.title ?? "");
  });
}

function pickState(cur: EventGroup["gameState"], next: EventGroup["gameState"]): EventGroup["gameState"] {
  const rank: Record<EventGroup["gameState"], number> = { live: 3, post: 2, upcoming: 1 };
  return rank[next] > rank[cur] ? next : cur;
}

function prettySlug(s: string): string {
  return s.replace(/-/g, " ").replace(/\b\w/g, c => c.toUpperCase());
}

/** Strip the base-event prefix so sub-market labels read cleanly,
 *  e.g. `atp-bonzi-zverev-2026-05-24-set-totals-3pt5` → `Set Totals 3.5`. */
function subLabel(m: LiveUpcomingMarket, baseSlug: string): string {
  if (m.question) {
    // Question often already short, e.g. "Benjamin Bonzi vs. Alexander Zverev: Total Sets O/U 3.5".
    // Trim the "<name> vs. <name>: " prefix to keep just the sub-market label.
    const colon = m.question.indexOf(":");
    if (colon > 0 && colon < m.question.length - 1) {
      return m.question.slice(colon + 1).trim();
    }
    return m.question;
  }
  const rest = m.slug.startsWith(baseSlug + "-")
    ? m.slug.slice(baseSlug.length + 1)
    : m.slug;
  return prettySlug(rest);
}

// ──────────────────────────────────────────────────────────────────
//   UI components
// ──────────────────────────────────────────────────────────────────

function Section({ title, icon: Icon, tone, count, children }: {
  title: string;
  icon: React.ElementType;
  tone: "accent" | "buy" | "sell" | "neutral";
  count: number;
  children: React.ReactNode;
}) {
  const toneCls: Record<string, string> = {
    accent:  "bg-accent/12 text-accent border-accent/30",
    buy:     "bg-buy/12 text-buy border-buy/30",
    sell:    "bg-sell/12 text-sell border-sell/30 shadow-glow-sell",
    neutral: "bg-bg-panel text-text-muted border-line",
  };
  return (
    <div>
      <div className="flex items-center gap-2 mb-2.5 px-1">
        <span className={cn("inline-flex items-center gap-1 px-2 py-0.5 rounded-md border text-[10px] uppercase tracking-wider2 font-semibold",
          toneCls[tone])}>
          <Icon className="size-3" />
          {title}
        </span>
        <span className="text-[11px] text-text-muted">
          <span className="num text-text">{count}</span> 경기
        </span>
      </div>
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
        {children}
      </div>
    </div>
  );
}

function EventCard({ g, defaultOpen }: { g: EventGroup; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const hasSubs = g.subs.length > 0;
  const start = g.gameStart ? new Date(g.gameStart) : null;
  const isLive = g.gameState === "live";
  const isPost = g.gameState === "post";
  const accent = isLive ? "border-sell/30 shadow-glow-sell" :
                 g.hasBotFills ? "border-buy/30 shadow-glow-buy" :
                 "border-line/80";

  // Tennis title abbreviator: drop the "Roland Garros ATP:" / similar prefix.
  const cleanTitle = g.title
    ? g.title.replace(/^[^:]+:\s*/, "")
    : prettySlug(g.baseSlug);

  return (
    <div className={cn(
      "surface px-4 py-3.5 transition-all duration-200 relative overflow-hidden",
      accent
    )}>
      {g.hasBotFills && (
        <span className="absolute top-2 right-2 inline-flex items-center gap-1 text-[9.5px]
                         px-1.5 py-0.5 rounded-full bg-buy/15 text-buy border border-buy/30 font-semibold z-10">
          <CheckCircle2 className="size-2.5" /> bot
        </span>
      )}

      {/* Header — icon + title + tour chip */}
      <div className="flex items-start gap-3">
        {g.icon ? (
          <Image src={g.icon} alt="" width={44} height={44}
            className="size-11 rounded-xl object-cover shrink-0 ring-1 ring-line/60 shadow-soft" unoptimized />
        ) : (
          <div className="size-11 rounded-xl bg-gradient-to-br from-accent/15 via-bg-panel to-bg-panel
                          border border-line/70 shrink-0 flex items-center justify-center">
            <Trophy className="size-4 text-accent/70" />
          </div>
        )}
        <div className="flex-1 min-w-0">
          <div className="text-[13px] text-text font-semibold tracking-tightish leading-snug line-clamp-2 pr-12">
            {cleanTitle}
          </div>
          <div className="mt-1 flex items-center gap-1.5 text-[10px] text-text-dim flex-wrap">
            {g.baseSlug.startsWith("atp-") && <span className="chip-accent !text-[9.5px]">ATP</span>}
            {g.baseSlug.startsWith("wta-") && <span className="chip-purple !text-[9.5px]">WTA</span>}
            {g.hasNegRisk && <span className="font-mono">negRisk</span>}
            {hasSubs && (
              <span className="inline-flex items-center gap-0.5 text-text-muted">
                <Layers className="size-2.5" /> {1 + g.subs.length} 마켓
              </span>
            )}
          </div>
        </div>
      </div>

      {/* Time + total volume */}
      <div className="mt-3 flex items-center justify-between gap-2 text-[11px]">
        <TimeBadge state={g.gameState} secs={g.secsToStart} start={start} />
        {g.totalVolume > 0 && (
          <div className="flex items-center gap-1 text-text-muted">
            <TrendingUp className="size-3" />
            <span className="num text-text">{fmt$(g.totalVolume, 0)}</span>
            <span className="text-text-dim">vol</span>
          </div>
        )}
      </div>

      {/* Main market price line */}
      {g.main && (g.main.best_bid != null || g.main.best_ask != null) && (
        <div className="mt-2.5 pt-2.5 border-t border-line/40">
          <div className="text-[9.5px] uppercase tracking-wider text-text-dim mb-1.5">Moneyline</div>
          <PriceRow bid={g.main.best_bid} mid={g.main.mid} ask={g.main.best_ask} />
        </div>
      )}

      {/* Sub-markets tree */}
      {hasSubs && (
        <div className="mt-3 pt-2.5 border-t border-line/40">
          <button onClick={() => setOpen(!open)}
            className="w-full flex items-center justify-between text-[11px] text-text-muted hover:text-text transition-colors">
            <span className="inline-flex items-center gap-1.5">
              <Layers className="size-3" />
              <span>서브 마켓 <span className="num text-text">{g.subs.length}</span></span>
            </span>
            <ChevronDown className={cn("size-3.5 transition-transform duration-150",
              open ? "rotate-180" : "rotate-0")} />
          </button>

          {open && (
            <div className="mt-2 space-y-1">
              {g.subs.map(s => (
                <SubMarketRow key={s.condition_id} m={s} baseSlug={g.baseSlug} />
              ))}
            </div>
          )}
        </div>
      )}

      {isPost && (
        <div className="mt-2 text-[10px] text-text-dim italic">
          종료된 경기 — 정산 대기 가능
        </div>
      )}
    </div>
  );
}

function SubMarketRow({ m, baseSlug }: { m: LiveUpcomingMarket; baseSlug: string }) {
  const label = subLabel(m, baseSlug);
  return (
    <div className="grid grid-cols-[1fr_auto] gap-2 items-center px-2 py-1.5 rounded-md
                    bg-bg-panel/40 border border-line/40 hover:bg-bg-panel/70 transition-colors">
      <div className="min-w-0 flex items-center gap-1.5">
        {m.has_bot_fills && (
          <span className="inline-flex size-1.5 rounded-full bg-buy shrink-0" title="bot fills" />
        )}
        <span className="text-[11.5px] text-text truncate">{label}</span>
      </div>
      <div className="flex items-center gap-2 text-[10.5px] shrink-0">
        {m.best_bid != null && (
          <span className="num text-buy font-semibold">{fmtPx(m.best_bid)}</span>
        )}
        <span className="text-text-dim">/</span>
        {m.best_ask != null && (
          <span className="num text-sell font-semibold">{fmtPx(m.best_ask)}</span>
        )}
        {m.volume_num != null && m.volume_num > 0 && (
          <span className="text-text-dim text-[9.5px] num ml-1">{fmt$(m.volume_num, 0)}</span>
        )}
      </div>
    </div>
  );
}

function PriceRow({ bid, mid, ask }: { bid: number | null; mid: number | null; ask: number | null }) {
  return (
    <div className="grid grid-cols-3 gap-2 text-[11px]">
      <div className="flex flex-col items-start">
        <span className="text-[9.5px] uppercase tracking-wider text-text-dim">Bid</span>
        <span className="num text-buy font-semibold">{fmtPx(bid)}</span>
      </div>
      <div className="flex flex-col items-center">
        <span className="text-[9.5px] uppercase tracking-wider text-text-dim">Mid</span>
        <span className="num text-text font-medium">{fmtPx(mid)}</span>
      </div>
      <div className="flex flex-col items-end">
        <span className="text-[9.5px] uppercase tracking-wider text-text-dim">Ask</span>
        <span className="num text-sell font-semibold">{fmtPx(ask)}</span>
      </div>
    </div>
  );
}

function TimeBadge({ state, secs, start }: { state: string; secs: number | null; start: Date | null }) {
  if (state === "live") {
    return (
      <span className="inline-flex items-center gap-1.5 text-[11px] text-sell font-semibold">
        <span className="size-1.5 rounded-full bg-sell animate-pulse" />
        LIVE
        {start && <span className="text-text-dim font-normal">({timeShort(start)})</span>}
      </span>
    );
  }
  if (state === "post") {
    if (secs == null) return <span className="text-text-dim text-[11px]">ended</span>;
    const abs = Math.abs(secs);
    const label = abs < 3600 ? `${Math.round(abs / 60)}m ago`
                : abs < 86400 ? `${(abs / 3600).toFixed(1)}h ago`
                : `${(abs / 86400).toFixed(1)}d ago`;
    return <span className="text-text-dim text-[11px]">ended {label}</span>;
  }
  if (secs == null) return <span className="text-text-dim text-[11px]">—</span>;
  if (secs < 0) {
    return <span className="text-warn text-[11px] inline-flex items-center gap-1"><Flame className="size-3" /> started</span>;
  }
  if (secs < 3600) {
    return (
      <span className="inline-flex items-center gap-1.5 text-[11px] text-warn font-semibold">
        <Flame className="size-3 animate-pulse" />
        <span className="num">{Math.round(secs / 60)}m</span>
        <span className="font-normal">later</span>
      </span>
    );
  }
  if (secs < 86400) {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return (
      <span className="inline-flex items-center gap-1.5 text-[11px] text-accent">
        <Timer className="size-3" />
        in <span className="num text-text">{h}h{m > 0 ? ` ${m}m` : ""}</span>
        {start && <span className="text-text-dim font-normal">· {timeShort(start)}</span>}
      </span>
    );
  }
  const days = (secs / 86400).toFixed(1);
  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <Timer className="size-3" />
      in <span className="num text-text">{days}d</span>
      {start && <span className="text-text-dim font-normal">· {dateShort(start)}</span>}
    </span>
  );
}

function timeShort(d: Date): string {
  return d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}
function dateShort(d: Date): string {
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function Pill({ active, onClick, children }:
  { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick}
      className={cn("px-2 py-1 rounded-md text-[11px] font-medium inline-flex items-center gap-1 transition-colors",
        active ? "bg-accent/15 text-accent border border-accent/30"
               : "bg-bg-panel/70 text-text-muted border border-line/80 hover:text-text")}>
      {children}
    </button>
  );
}
