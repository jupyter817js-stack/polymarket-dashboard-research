"use client";
import { fmtN } from "@/lib/fmt";
import { cn } from "@/lib/fmt";
import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from "lucide-react";

/** 페이지네이션 바 — Positions/Activity/Markets-detail 공통.
 *  variant="full" : « 처음 ‹ 이전 [page n/N] 다음 › 끝 »
 *  variant="compact": ‹ 이전 [page n/N] 다음 ›
 */
export function PageBar({
  page, totalPages, setPage, variant = "full", totalLabel,
}: {
  page: number;
  totalPages: number;
  setPage: (n: number) => void;
  variant?: "full" | "compact";
  totalLabel?: string;
}) {
  if (totalPages <= 1 && variant !== "compact") return null;
  return (
    <div className={cn(
      "flex items-center justify-center gap-1.5",
      variant === "compact" ? "py-1.5" : "py-2.5"
    )}>
      {variant === "full" && (
        <PageBtn onClick={() => setPage(1)} disabled={page === 1} label="처음">
          <ChevronsLeft className="size-3.5" />
        </PageBtn>
      )}
      <PageBtn onClick={() => setPage(Math.max(1, page - 1))} disabled={page === 1} label="이전">
        <ChevronLeft className="size-3.5" />
      </PageBtn>
      <div className="px-3 py-1 text-[12px] text-text-muted">
        <span className="text-text num font-medium">{page}</span>
        <span className="text-text-dim mx-1.5">/</span>
        <span className="num">{fmtN(totalPages)}</span>
        {totalLabel && <span className="text-text-dim ml-2.5">· {totalLabel}</span>}
      </div>
      <PageBtn onClick={() => setPage(Math.min(totalPages, page + 1))} disabled={page === totalPages} label="다음">
        <ChevronRight className="size-3.5" />
      </PageBtn>
      {variant === "full" && (
        <PageBtn onClick={() => setPage(totalPages)} disabled={page === totalPages} label="끝">
          <ChevronsRight className="size-3.5" />
        </PageBtn>
      )}
    </div>
  );
}

function PageBtn({ onClick, disabled, label, children }: {
  onClick: () => void; disabled?: boolean; label: string; children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex items-center justify-center size-7 rounded-md border transition-all",
        "bg-bg-panel border-line text-text-muted hover:text-text hover:border-line-strong hover:bg-bg-soft",
        "disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-bg-panel disabled:hover:border-line",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/30",
      )}
    >
      {children}
    </button>
  );
}
