"use client";
import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { api, FillEntry, HistorySort, HistoryQuery } from "@/lib/api";
import { fmt$, fmtN, relTime, cn } from "@/lib/fmt";
import { ExternalLink, Search, X, ArrowDownUp, Filter, ArrowUpRight } from "lucide-react";
import { PageBar } from "@/components/pagebar";

/** Polymarket portfolio Activity 페이지 미러.
 *  TYPE | MARKET | AMOUNT 3-column grid + 페이지네이션 + 정렬/필터/검색.
 */
export default function ActivityPage() {
  const [sort, setSort] = useState<HistorySort>("newest");
  const [side, setSide] = useState<"ALL" | "BUY" | "SELL">("ALL");
  const [pageSize, setPageSize] = useState<number>(100);
  const [page, setPage] = useState<number>(1);
  const [q, setQ] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(q), 250);
    return () => clearTimeout(t);
  }, [q]);
  // 옵션 변경 시 page reset
  useEffect(() => { setPage(1); }, [sort, side, debouncedQ, pageSize]);

  const params: HistoryQuery = { sort, side, page, page_size: pageSize, q: debouncedQ || undefined };
  const queryRes = useQuery({
    queryKey: ["history", params],
    queryFn: () => api.history(params),
    // RN1 Activity — 기록 용도. 30s 갱신.
    refetchInterval: 30_000,
    staleTime: 25_000,
    placeholderData: keepPreviousData,
  });

  const fills = queryRes.data?.fills ?? [];
  const totalCount = queryRes.data?.total_count ?? 0;
  const totalPages = queryRes.data?.total_pages ?? 1;
  const showingFrom = totalCount === 0 ? 0 : (page - 1) * pageSize + 1;
  const showingTo = Math.min(page * pageSize, totalCount);

  return (
    <div className="space-y-3">
      {/* Toolbar */}
      <div className="surface px-4 py-3 flex flex-wrap items-center gap-3">
        <div className="relative flex-1 min-w-[220px]">
          <Search className="size-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-text-dim pointer-events-none" />
          <label htmlFor="rn1-history-search" className="sr-only">시장 / outcome / event 검색</label>
          <input
            id="rn1-history-search" name="rn1-history-search"
            type="search" autoComplete="off"
            placeholder="시장 / outcome / event 검색"
            value={q} onChange={e => setQ(e.target.value)}
            className="input pl-9 pr-8"
          />
          {q && (
            <button onClick={() => setQ("")} className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text transition-colors" aria-label="검색 지우기">
              <X className="size-3.5" />
            </button>
          )}
        </div>

        <Select label={<><ArrowDownUp className="size-3" /> 정렬</>} value={sort} onChange={setSort} options={[
          ["newest",    "최신순"],
          ["oldest",    "오래된순"],
          ["size",      "Size 큰 순"],
          ["usdc",      "USDC 큰 순"],
          ["price",     "Price 높은 순"],
          ["low_price", "Price 낮은 순"],
        ]} />

        <Select label={<><Filter className="size-3" /> Side</>} value={side} onChange={setSide} options={[
          ["ALL",  "전체"],
          ["BUY",  "BUY 만"],
          ["SELL", "SELL 만"],
        ]} />

        <Select label="페이지" value={String(pageSize)} onChange={v => setPageSize(Number(v))} options={[
          ["50", "50/page"], ["100", "100/page"], ["250", "250/page"], ["500", "500/page"], ["1000", "1000/page"],
        ]} />

        <div className="ml-auto text-[11px] text-text-muted num">
          {totalCount > 0
            ? <><span>{fmtN(showingFrom)}–{fmtN(showingTo)}</span> <span className="text-text-faint">/</span> <span>{fmtN(totalCount)}</span> <span className="font-sans text-text-dim">체결</span></>
            : "0 체결"}
        </div>
      </div>

      {/* 헤더 PageBar */}
      {totalPages > 1 && (
        <PageBar page={page} totalPages={totalPages} setPage={setPage}
          totalLabel={`${fmtN(totalCount)} 체결`} />
      )}

      <div className="surface-flush">
        <div className="grid grid-cols-[110px_1fr_180px] px-5 py-2.5 th border-b border-line bg-bg-soft/30">
          <div>Type</div>
          <div>Market</div>
          <div className="text-right">Amount</div>
        </div>

        {queryRes.isLoading && !queryRes.data ? (
          <div className="px-5 py-16 text-center text-text-muted text-[13px]">데이터 로드 중…</div>
        ) : fills.length === 0 ? (
          <div className="px-5 py-16 text-center text-text-muted text-[13px]">
            {debouncedQ ? `"${q}" 검색 결과 없음` : "체결 이력 없음"}
          </div>
        ) : (
          <ul className="divide-y divide-line/40">
            {fills.map((f, i) => <Row key={`${f.tx_hash}-${f.condition_id}-${f.outcome_index}-${i}`} f={f} />)}
          </ul>
        )}
      </div>

      {/* 푸터 PageBar */}
      {totalPages > 1 && (
        <PageBar page={page} totalPages={totalPages} setPage={setPage} />
      )}
    </div>
  );
}

function Select({ label, value, onChange, options }: {
  label: React.ReactNode; value: string; onChange: (v: any) => void;
  options: [string, string][];
}) {
  return (
    <label className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="flex items-center gap-1">{label}</span>
      <select value={value} onChange={e => onChange(e.target.value)} className="select">
        {options.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
      </select>
    </label>
  );
}

function Row({ f }: { f: FillEntry }) {
  const router = useRouter();
  const isBuy = f.side === "BUY";
  const priceCents = Math.round(f.price * 100);
  const outcomeChip = f.outcome_index === 0 ? "chip-green" : "chip-red";

  // 행 클릭 → 해당 마켓을 Positions 에 deep link (이벤트 + 마켓 자동 펼침)
  const goToPosition = () => {
    const params = new URLSearchParams();
    if (f.event_slug) params.set("ev", f.event_slug);
    params.set("cid", f.condition_id);
    router.push(`/rn1/positions?${params.toString()}`);
  };

  return (
    <li
      onClick={goToPosition}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); goToPosition(); } }}
      title="클릭하면 Positions 의 해당 마켓으로 이동"
      className="group grid grid-cols-[110px_1fr_180px] px-5 py-3.5 hover:bg-bg-panel/40 transition-colors items-center cursor-pointer focus-visible:outline-none focus-visible:bg-bg-panel/40"
    >
      <div className="flex items-center gap-1.5 text-[14px] font-medium">
        <span className={cn("inline-block size-1.5 rounded-full", isBuy ? "bg-buy" : "bg-sell")} />
        <span className="text-text">{isBuy ? "Buy" : "Sell"}</span>
      </div>
      <div className="flex items-start gap-3 min-w-0">
        {f.icon ? (
          <img src={f.icon} alt="" className="size-10 rounded-md object-cover shrink-0 bg-bg-panel ring-1 ring-line/60" loading="lazy" />
        ) : (
          <div className="size-10 rounded-md bg-bg-panel border border-line shrink-0" />
        )}
        <div className="flex-1 min-w-0">
          <div className="text-[13px] text-text truncate leading-snug group-hover:text-accent transition-colors">
            {f.title || f.condition_id.slice(0, 14)}
          </div>
          <div className="flex items-center gap-1.5 mt-1.5 text-[11px]">
            <span className={cn(outcomeChip, "py-0.5 px-2")}>
              {f.outcome_name || `[${f.outcome_index}]`} <span className="num">{priceCents}¢</span>
            </span>
            <span className="text-text-muted num">{fmtN(f.size, 1)} <span className="font-sans text-text-dim">shares</span></span>
          </div>
        </div>
      </div>
      <div className="text-right flex flex-col items-end gap-0.5">
        <div className="text-[14px] num font-semibold text-text flex items-center gap-1.5">
          <ArrowUpRight className="size-3 text-text-dim opacity-0 group-hover:opacity-100 transition-opacity" />
          {fmt$(f.usdc_size, 2)}
        </div>
        <div className="text-[11px] text-text-muted flex items-center gap-1.5">
          <span>{relTime(f.ts)}</span>
          {f.tx_hash && (
            <a
              href={`https://polygonscan.com/tx/${f.tx_hash}`}
              target="_blank"
              rel="noopener"
              onClick={(e) => e.stopPropagation()}
              className="text-text-dim hover:text-accent transition-colors"
              title="polygonscan">
              <ExternalLink className="size-3" />
            </a>
          )}
        </div>
      </div>
    </li>
  );
}
