import React from "react";
import { Badge } from "@/components/ui/Badge";
import ToolLogo from "./ToolLogo";
import { formatTokens, type ToolUsageRow } from "./token-monitor-data";

/** 工具用量表：占比条 + 能力/状态徽标。空数据显示「暂无数据」。 */
export function ToolUsageList({ rows }: { rows: ToolUsageRow[] }) {
  if (rows.length === 0) {
    return <div className="py-8 text-center text-sm text-[var(--text-tertiary)]">暂无用量数据（请先扫描本机工具）</div>;
  }
  const max = Math.max(...rows.map((r) => r.total_tokens), 1);
  return (
    <div className="flex flex-col gap-2">
      {rows.map((row) => (
        <div key={row.tool_id} className="rounded-xl border border-[var(--border)] bg-[var(--card-bg)] px-3 py-2.5">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 min-w-0">
              <ToolLogo toolId={row.tool_id} displayName={row.display_name} size={18} />
              <span className="text-sm font-medium truncate">{row.display_name}</span>
              <Badge variant={row.support_level === "full" ? "ok" : row.support_level === "standard" ? "info" : "mute"}>
                {row.support_level}
              </Badge>
            </div>
            <span className="text-sm tabular-nums font-semibold">{formatTokens(row.total_tokens)}</span>
          </div>
          <div className="mt-1.5 flex items-center gap-2">
            <div className="flex-1 h-1.5 rounded-full bg-[var(--mute-bg)] overflow-hidden">
              <div className="h-full rounded-full bg-[var(--color-brand)]" style={{ width: `${(row.total_tokens / max) * 100}%` }} />
            </div>
            <span className="text-[11px] tabular-nums text-[var(--text-secondary)] w-14 text-right">{row.share_percent.toFixed(1)}%</span>
          </div>
          <div className="mt-1.5 text-[11px] text-[var(--text-tertiary)] tabular-nums">
            in {formatTokens(row.input_tokens)} · out {formatTokens(row.output_tokens)} · cache {formatTokens(row.cache_tokens)}
            {row.cost_amount != null ? ` · $${row.cost_amount.toFixed(2)}` : " · 价格未知"}
          </div>
        </div>
      ))}
    </div>
  );
}
