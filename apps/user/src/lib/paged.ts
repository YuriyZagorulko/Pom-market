export type Paged<T> = {
  items: T[];
  total: number;
  limit: number;
  offset: number;
};

export function parsePaged<T>(body: unknown): Paged<T> {
  if (body && typeof body === "object" && "items" in body) {
    const p = body as Paged<T>;
    return {
      items: Array.isArray(p.items) ? p.items : [],
      total: Number(p.total) || 0,
      limit: Number(p.limit) || 25,
      offset: Number(p.offset) || 0,
    };
  }
  if (Array.isArray(body)) {
    return { items: body as T[], total: body.length, limit: body.length, offset: 0 };
  }
  return { items: [], total: 0, limit: 25, offset: 0 };
}

export function pageRange(offset: number, limit: number, total: number): string {
  if (total === 0) return "No results";
  const from = offset + 1;
  const to = Math.min(offset + limit, total);
  return `Showing ${from}–${to} of ${total.toLocaleString()}`;
}
