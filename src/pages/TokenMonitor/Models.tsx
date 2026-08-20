import React from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { useModelUsage, formatTokens } from "@/components/token-monitor/token-monitor-data";
import { ProviderLogo } from "@/components/token-monitor/ToolLogo";
import type { Range } from "@/lib/token-monitor-commands";

export default function Models({ range }: { range: Range }) {
  const { data: models } = useModelUsage(range);
  const rows = models ?? [];
  const maxTotal = Math.max(1, ...rows.map((r) => r.total_tokens));

  return (
    <Card>
      <CTitle>模型用量</CTitle>
      <CBody>
        {rows.length === 0 ? (
          <div className="py-12 text-center text-sm text-[var(--text-tertiary)]">暂无模型用量数据</div>
        ) : (
          <div className="flex flex-col divide-y divide-[var(--border)]">
            {rows.map((row) => (
              <div key={row.model} className="py-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="flex items-center gap-2 min-w-0">
                    <ProviderLogo model={row.model} size={16} />
                    <span className="text-sm font-medium truncate">{row.model}</span>
                  </span>
                  <span className="text-sm tabular-nums font-semibold shrink-0">{formatTokens(row.total_tokens)}</span>
                </div>
                <div className="mt-1.5 h-1.5 rounded-full bg-[var(--mute-bg)] overflow-hidden">
                  <div
                    className="h-full rounded-full bg-[var(--color-brand)]"
                    style={{ width: `${(row.total_tokens / maxTotal) * 100}%` }}
                  />
                </div>
                <div className="mt-1.5 flex items-center gap-3 text-[11px] text-[var(--text-tertiary)] tabular-nums">
                  <span>in {formatTokens(row.input_tokens)}</span>
                  <span>out {formatTokens(row.output_tokens)}</span>
                  <span>cache {formatTokens(row.cache_tokens)}</span>
                  {row.cost_amount != null && <span>≈ ¥{row.cost_amount.toFixed(2)}</span>}
                  <span className="ml-auto">{(row.share_percent ?? 0).toFixed(1)}%</span>
                </div>
              </div>
            ))}
          </div>
        )}
      </CBody>
    </Card>
  );
}
