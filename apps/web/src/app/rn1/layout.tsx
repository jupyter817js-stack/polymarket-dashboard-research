import { PortfolioHeader } from "@/components/portfolio-header";
import { PortfolioTabs } from "@/components/portfolio-tabs";

export default function Rn1Layout({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-3 sm:px-5 lg:px-6 py-4 sm:py-5 max-w-[1400px] mx-auto">
      <PortfolioHeader />
      <PortfolioTabs />
      <div className="mt-5 animate-fade-in-up">{children}</div>
    </div>
  );
}
