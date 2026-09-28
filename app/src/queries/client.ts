import { QueryClient } from "@tanstack/react-query";
import { shouldRetry } from "@/lib/errors";

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,
        retry: shouldRetry,
        retryDelay: (attempt) => Math.min(4000, 600 * 2 ** attempt),
        // A desktop app: window focus says nothing about data freshness; backend events do.
        refetchOnWindowFocus: false,
      },
      mutations: { retry: false },
    },
  });
}
