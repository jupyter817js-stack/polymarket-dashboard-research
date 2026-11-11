"use client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";

/** 전역 React Query 기본값.
 *
 *  설계:
 *    · 폴링은 default OFF — 각 query 가 명시적으로 refetchInterval 을 설정.
 *      이전엔 5초 default 였음 → 모든 페이지에서 모든 query 가 5초마다 큰 JSON
 *      을 다시 fetch 하여 SPA 가 항상 무거웠음.
 *    · staleTime 30초 — 페이지 전환·재마운트 시 캐시된 데이터 즉시 표시,
 *      네트워크 호출 없음.
 *    · gcTime 5분 — 백그라운드 캐시 보관.
 *    · refetchOnWindowFocus OFF — 탭 전환마다 재호출 안 함.
 *
 *  실시간성이 필요한 query (live trader, open orders) 는 그 query 의
 *  useQuery 옵션에서 `refetchInterval: N` 으로 명시적 opt-in. */
export function Providers({ children }: { children: React.ReactNode }) {
  const [client] = useState(() => new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false,
        refetchOnMount: false,
        refetchOnReconnect: false,
        staleTime: 30_000,
        gcTime: 5 * 60_000,
        retry: 1,
        // refetchInterval intentionally OMITTED — opt-in per query.
      },
    },
  }));
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
