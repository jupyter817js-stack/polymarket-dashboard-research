"use client";
import { usePathname } from "next/navigation";
import Link from "next/link";
import { useEffect, useState } from "react";
import { Activity, BarChart3, Zap, Cpu, Eye, Layers3, Menu, X, Shield, CalendarClock } from "lucide-react";
import { cn } from "@/lib/fmt";
import { SystemStatus } from "./system-status";

/** App shell — responsive sidebar (collapsible on mobile) + main content.
 *  Sections:
 *   · RN1 모니터  — 폴리마켓 portfolio 미러 (forensic + 라이브 모니터링)
 *   · LIVE 트레이딩 — 자체 봇 */
export function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname() || "/";
  const [mobileOpen, setMobileOpen] = useState(false);

  // 닫기: 라우트 변경 시 / esc / 바깥 클릭
  useEffect(() => { setMobileOpen(false); }, [pathname]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setMobileOpen(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="min-h-screen lg:grid lg:grid-cols-[260px_1fr]">
      {/* Mobile top bar — only visible <lg */}
      <div className="lg:hidden sticky top-0 z-30 flex items-center gap-3 px-4 py-3
                      bg-bg-elevated/80 backdrop-blur-md border-b border-line/80">
        <button onClick={() => setMobileOpen(o => !o)}
          aria-label="메뉴 열기/닫기"
          className="size-9 rounded-lg bg-bg-panel border border-line/80 flex items-center justify-center
                     text-text-muted hover:text-text active:scale-95 transition-all">
          {mobileOpen ? <X className="size-4" /> : <Menu className="size-4" />}
        </button>
        <div className="flex items-center gap-2">
          <div className="size-7 rounded-md bg-gradient-to-br from-accent/30 via-accent-2/20 to-accent/15
                          border border-accent/30 flex items-center justify-center">
            <Cpu className="size-3.5 text-accent" />
          </div>
          <div className="text-[14px] font-semibold tracking-tightish">rn1-bot</div>
        </div>
      </div>

      {/* Mobile overlay */}
      {mobileOpen && (
        <div onClick={() => setMobileOpen(false)}
          className="lg:hidden fixed inset-0 z-40 bg-bg/80 backdrop-blur-sm animate-fade-in" />
      )}

      {/* Sidebar */}
      <aside className={cn(
        "fixed lg:sticky inset-y-0 left-0 z-50 w-[260px]",
        "bg-bg-elevated/95 backdrop-blur-md border-r border-line/80",
        "flex flex-col h-screen overflow-y-auto",
        "transition-transform duration-300 ease-out",
        mobileOpen ? "translate-x-0" : "-translate-x-full lg:translate-x-0",
      )}>
        {/* Brand */}
        <div className="px-5 pt-5 pb-4 border-b border-line/70 relative">
          {/* Subtle sheen behind brand */}
          <div className="absolute inset-x-0 top-0 h-24 bg-sheen pointer-events-none" />
          <div className="relative flex items-center gap-3">
            <div className="size-10 rounded-xl bg-gradient-to-br from-accent/30 via-accent-2/20 to-accent/10
                            border border-accent/35 flex items-center justify-center shadow-glow">
              <Cpu className="size-5 text-accent" />
            </div>
            <div>
              <div className="text-[16px] font-semibold tracking-tightish leading-tight text-gradient-accent">
                rn1-bot
              </div>
              <div className="text-[10px] text-text-dim font-mono mt-0.5 tracking-wider2 uppercase">
                console · M0.5
              </div>
            </div>
          </div>
        </div>

        {/* RN1 Monitor */}
        <nav className="px-3 pt-5 space-y-0.5">
          <SectionHeader>RN1 모니터</SectionHeader>
          <NavLink href="/rn1/positions"
            icon={<BarChart3 className="size-4" />}
            active={pathname.startsWith("/rn1/positions") || pathname === "/" || pathname === "/rn1"}
            meta="tree">
            Positions
          </NavLink>
          <NavLink href="/rn1/history"
            icon={<Activity className="size-4" />}
            active={pathname.startsWith("/rn1/history")}>
            Activity
          </NavLink>
          <NavLink href="/rn1/analysis"
            icon={<Layers3 className="size-4" />}
            active={pathname.startsWith("/rn1/analysis")}>
            Analysis
          </NavLink>
        </nav>

        {/* LIVE Trading */}
        <nav className="px-3 pt-5 space-y-0.5">
          <SectionHeader>
            LIVE 트레이딩
            <span className="badge-accent ml-1.5 normal-case tracking-normal">PAPER</span>
          </SectionHeader>
          <NavLink href="/live/summary"
            icon={<Zap className="size-4" />}
            active={pathname.startsWith("/live/summary")}>
            요약
          </NavLink>
          <NavLink href="/live/markets"
            icon={<CalendarClock className="size-4" />}
            active={pathname.startsWith("/live/markets")}>
            경기 일정
          </NavLink>
          <NavLink href="/live/positions"
            icon={<BarChart3 className="size-4" />}
            active={pathname.startsWith("/live/positions")}>
            내 Positions
          </NavLink>
          <NavLink href="/live/orders"
            icon={<Eye className="size-4" />}
            active={pathname.startsWith("/live/orders")}>
            내 Open Orders
          </NavLink>
          <NavLink href="/live/pnl"
            icon={<Activity className="size-4" />}
            active={pathname.startsWith("/live/pnl")}>
            PnL
          </NavLink>
          <NavLink href="/live/risk"
            icon={<Shield className="size-4" />}
            active={pathname.startsWith("/live/risk")}>
            Risk
          </NavLink>
        </nav>

        {/* System status footer */}
        <div className="mt-auto p-3 border-t border-line/70">
          <SystemStatus />
        </div>
      </aside>

      <main className="overflow-x-hidden min-w-0">
        {children}
      </main>
    </div>
  );
}

function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2.5 pt-1 pb-2 label-eyebrow flex items-center gap-1.5">
      {children}
    </div>
  );
}

function NavLink({ href, icon, active, disabled, meta, children }: {
  href: string;
  icon: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
  meta?: string;
  children: React.ReactNode;
}) {
  return (
    <Link
      href={disabled ? "#" : href}
      aria-disabled={disabled}
      tabIndex={disabled ? -1 : undefined}
      prefetch={!disabled}
      className={cn(
        "group relative flex items-center gap-2.5 pl-3 pr-2 py-2 rounded-lg text-[13px] font-medium",
        "transition-all duration-150 ease-out",
        active
          ? "bg-gradient-to-r from-accent/10 via-accent/5 to-transparent text-text shadow-inner-stroke"
          : "text-text-muted hover:text-text hover:bg-bg-panel/70",
        disabled && "opacity-40 pointer-events-none",
      )}
    >
      {active && (
        <span className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r-full
                         bg-gradient-to-b from-accent via-accent to-accent-3 shadow-[0_0_8px_rgba(91,192,235,0.6)]" />
      )}
      <span className={cn(
        "shrink-0 transition-colors",
        active ? "text-accent" : "text-text-muted group-hover:text-text"
      )}>
        {icon}
      </span>
      <span className="flex-1 truncate">{children}</span>
      {meta && (
        <span className="text-[9.5px] text-text-dim font-mono uppercase tracking-wider opacity-60">
          {meta}
        </span>
      )}
    </Link>
  );
}
