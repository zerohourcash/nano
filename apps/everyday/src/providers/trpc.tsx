import { createTRPCReact } from "@trpc/react-query";
import { httpBatchLink } from "@trpc/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import superjson from "superjson";
import type { AppRouter } from "../../api/router";
import type { ReactNode } from "react";
import { trpcUrl } from "@/lib/app-mode";
import { requiresDeviceSignature, signedDeviceHeaders } from "@/lib/device-signing";

export const trpc = createTRPCReact<AppRouter>();

const queryClient = new QueryClient();
const trpcClient = trpc.createClient({
  links: [
    httpBatchLink({
      url: trpcUrl(),
      transformer: superjson,
      async fetch(input, init) {
        const url = typeof input === "string" ? input : input.toString();
        const deviceHeaders = requiresDeviceSignature(url)
          ? await signedDeviceHeaders(url, init?.body)
          : {};
        return globalThis.fetch(input, {
          ...(init ?? {}),
          headers: { ...(init?.headers ?? {}), ...deviceHeaders },
          credentials: "include",
        });
      },
    }),
  ],
});

export function TRPCProvider({ children }: { children: ReactNode }) {
  return (
    <trpc.Provider client={trpcClient} queryClient={queryClient}>
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    </trpc.Provider>
  );
}
