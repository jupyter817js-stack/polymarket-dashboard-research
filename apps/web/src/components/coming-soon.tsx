import { Hourglass } from "lucide-react";

export function ComingSoon({ title, since }: { title: string; since: string }) {
  return (
    <div className="surface px-5 py-16 text-center">
      <div className="size-12 rounded-xl bg-warn/10 border border-warn/25 flex items-center justify-center mx-auto mb-4">
        <Hourglass className="size-5 text-warn" />
      </div>
      <div className="text-[15px] text-text font-semibold tracking-tightish mb-1.5">{title}</div>
      <div className="text-[12px] text-text-muted">
        {since} 단계 진입 후 활성화됩니다.
      </div>
    </div>
  );
}
