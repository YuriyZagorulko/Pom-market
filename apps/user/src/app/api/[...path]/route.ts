import { NextRequest, NextResponse } from "next/server";

const API = process.env.API_INTERNAL_URL || "http://127.0.0.1:4321";
/** Default upstream cap (calculate uses a dedicated route with a longer limit). */
const UPSTREAM_TIMEOUT_MS = 120_000;

const FORWARD_HEADERS = ["authorization", "content-type", "accept", "cookie"] as const;

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

async function proxy(request: NextRequest, pathSegments: string[]): Promise<NextResponse> {
  const path = pathSegments.join("/");
  const url = `${API}/api/${path}${request.nextUrl.search}`;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), UPSTREAM_TIMEOUT_MS);
  try {
    const headers = new Headers();
    for (const name of FORWARD_HEADERS) {
      const v = request.headers.get(name);
      if (v) headers.set(name, v);
    }
    const init: RequestInit = {
      method: request.method,
      headers,
      signal: controller.signal,
    };
    if (request.method !== "GET" && request.method !== "HEAD") {
      init.body = await request.arrayBuffer();
    }
    const upstream = await fetch(url, init);
    const body = await upstream.arrayBuffer();
    return new NextResponse(body, {
      status: upstream.status,
      headers: {
        "content-type": upstream.headers.get("content-type") || "application/json",
      },
    });
  } catch (err) {
    if (controller.signal.aborted) {
      return NextResponse.json({ error: "Upstream request timed out" }, { status: 504 });
    }
    const msg = err instanceof Error ? err.message : "upstream error";
    return NextResponse.json({ error: msg }, { status: 502 });
  } finally {
    clearTimeout(timer);
  }
}

type RouteContext = { params: { path: string[] } };

export async function GET(request: NextRequest, context: RouteContext) {
  return proxy(request, context.params.path);
}

export async function POST(request: NextRequest, context: RouteContext) {
  return proxy(request, context.params.path);
}

export async function PATCH(request: NextRequest, context: RouteContext) {
  return proxy(request, context.params.path);
}

export async function PUT(request: NextRequest, context: RouteContext) {
  return proxy(request, context.params.path);
}

export async function DELETE(request: NextRequest, context: RouteContext) {
  return proxy(request, context.params.path);
}

export async function OPTIONS(request: NextRequest, context: RouteContext) {
  return proxy(request, context.params.path);
}