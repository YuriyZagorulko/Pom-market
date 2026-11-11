export type PaperExecutionMode = "clob" | "synthetic_source_price" | null | undefined;

export function paperExecutionLabel(mode: PaperExecutionMode): string {
  switch (mode) {
    case "clob":
      return "CLOB book (counts toward PAPER P&L)";
    case "synthetic_source_price":
      return "Paper-only @ leader price (not CLOB-executable)";
    default:
      return "Legacy / unlabeled";
  }
}

export function paperExecutionShort(mode: PaperExecutionMode): string {
  switch (mode) {
    case "clob":
      return "CLOB";
    case "synthetic_source_price":
      return "Paper-only";
    default:
      return "Legacy";
  }
}

type PaperExecutionContext = {
  source_price?: string;
  copy_limit_price?: string;
  best_clob_price?: string | null;
  best_clob_price_within_limit?: string | null;
  execution_price?: string;
  deviation_from_best_clob_bps?: number | null;
  paper_only?: boolean;
  execution_reason?: string | null;
};

export function formatPaperExecutionDetail(
  mode: PaperExecutionMode,
  ctx: PaperExecutionContext | null | undefined,
): string[] {
  if (!ctx) return [];
  const lines: string[] = [];
  lines.push(`Fill type: ${paperExecutionLabel(mode)}`);
  if (ctx.paper_only) {
    lines.push("This fill is simulated at the leader price; it would not be executable on the live CLOB.");
  }
  if (ctx.execution_reason) lines.push(ctx.execution_reason);
  if (ctx.source_price != null) lines.push(`Leader price: ${ctx.source_price}`);
  if (ctx.best_clob_price != null) lines.push(`Best CLOB ask: ${ctx.best_clob_price}`);
  if (ctx.execution_price != null) lines.push(`Execution price: ${ctx.execution_price}`);
  if (ctx.deviation_from_best_clob_bps != null) {
    lines.push(`Gap vs best CLOB: ${ctx.deviation_from_best_clob_bps} bps`);
  }
  return lines;
}
