import { NextRequest, NextResponse } from "next/server";

const API = process.env.API_INTERNAL_URL || "http://127.0.0.1:4321";
/** Next rewrite proxy times out at ~30s; heavy calculate runs need a dedicated route. */
const CALCULATE_TIMEOUT_MS = 180_000;

export const maxDuration = 300;
export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function POST(
  request: NextRequest,
  context: { params: { id: string } },
) {
  const { id } = context.params;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), CALCULATE_TIMEOUT_MS);
  try {
    const body = await request.text();
    const headers = new Headers();
    const auth = request.headers.get("authorization");
    if (auth) headers.set("authorization", auth);
    const cookie = request.headers.get("cookie");
    if (cookie) headers.set("cookie", cookie);
    headers.set("content-type", request.headers.get("content-type") || "application/json");

    const upstream = await fetch(`${API}/api/v1/source-traders/${id}/analytics/calculate`, {
      method: "POST",
      headers,
      body,
      signal: controller.signal,
    });
    const text = await upstream.text();
    return new NextResponse(text, {
      status: upstream.status,
      headers: { "content-type": upstream.headers.get("content-type") || "application/json" },
    });
  } catch (err) {
    if (controller.signal.aborted) {
      return NextResponse.json(
        {
          error:
            "Calculation timed out after 3 minutes. Try 30D or a shorter period, then recalculate.",
        },
        { status: 504 },
      );
    }
    const msg = err instanceof Error ? err.message : "upstream error";
    return NextResponse.json({ error: msg }, { status: 502 });
  } finally {
    clearTimeout(timer);
  }
}