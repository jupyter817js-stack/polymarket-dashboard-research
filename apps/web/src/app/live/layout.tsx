import { LiveHeader } from "@/components/live-header";

export default function LiveLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-3 sm:px-5 lg:px-6 py-4 sm:py-5 max-w-[1400px] mx-auto">
      <LiveHeader />
      <div className="animate-fade-in-up">{children}</div>
    </div>
  );
}
