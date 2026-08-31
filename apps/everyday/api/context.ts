import type { FetchCreateContextFnOptions } from "@trpc/server/adapters/fetch";

function cookieValue(header: string | null, name: string): string | null {
  if (!header) return null;
  for (const part of header.split(";")) {
    const [k, ...rest] = part.trim().split("=");
    if (k === name) return decodeURIComponent(rest.join("="));
  }
  return null;
}

export type TrpcContext = {
  req: Request;
  resHeaders: Headers;
  userId: number | null;
};

export async function createContext(opts: FetchCreateContextFnOptions): Promise<TrpcContext> {
  const header = opts.req.headers.get("x-user-id");
  const cookie = cookieValue(opts.req.headers.get("cookie"), "mk_user");
  const raw = header || cookie;
  const parsed = raw ? Number(raw) : NaN;
  return {
    req: opts.req,
    resHeaders: opts.resHeaders,
    userId: Number.isFinite(parsed) && parsed > 0 ? parsed : null,
  };
}
