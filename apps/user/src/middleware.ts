import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";

/**
 * Next.js rewrites forward incoming request headers to the Rust API unchanged.
 * Browsers can set X-Forwarded-For; strip spoofable forwarding headers on
 * internal API proxy paths so the API never treats them as client identity
 * (API uses TCP peer unless TRUSTED_PROXIES matches the immediate peer).
 */
const STRIP_FORWARDING = [
  "x-forwarded-for",
  "x-real-ip",
  "forwarded",
  "x-forwarded-host",
  "x-forwarded-proto",
] as const;

export function middleware(request: NextRequest) {
  const headers = new Headers(request.headers);
  for (const name of STRIP_FORWARDING) {
    headers.delete(name);
  }
  return NextResponse.next({
    request: { headers },
  });
}

export const config = {
  matcher: ["/api/:path*", "/health"],
};
