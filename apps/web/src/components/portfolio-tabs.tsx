"use client";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/fmt";

export function PortfolioTabs() {
  const pathname = usePathname() || "";
  const tabs = [
    { href: "/rn1/positions", label: "Positions", note: "tree" },
    { href: "/rn1/history",   label: "Activity" },
    { href: "/rn1/analysis",  label: "Analysis" },
  ];
  return (
    <div className="flex items-center border-b border-line/70 overflow-x-auto no-scrollbar -mx-1 px-1">
      {tabs.map(t => {
        const active = pathname.startsWith(t.href);
        return (
          <Link key={t.href} href={t.href} prefetch
            className={cn("tab whitespace-nowrap", active ? "tab-active" : "tab-inactive", "flex items-center gap-2")}>
            <span>{t.label}</span>
            {t.note && (
              <span className="text-[9.5px] text-text-dim font-mono uppercase tracking-wider opacity-60">
                {t.note}
              </span>
            )}
          </Link>
        );
      })}
    </div>
  );
}
