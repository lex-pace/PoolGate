import React from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { useProjects, formatTokens } from "@/components/token-monitor/token-monitor-data";
import type { Range } from "@/lib/token-monitor-commands";

// 工具拆分配色（与开源项目视图的多彩堆叠一致）
const TOOL_COLORS = ["#d98a5c", "#5b8def", "#37b6a0", "#d56a54", "#9b7fe0", "#c2a24d", "#5cc2d9", "#6b6e73"];

function formatUsd(cost?: number): string | null {
  if (cost == null) return null;
  return cost >= 1 ? `$${cost.toFixed(2)}` : `$${cost.toFixed(4)}`;
}

export default function Projects({ range }: { range: Range }) {
  const { data: projects } = useProjects(range);
  const rows = projects ?? [];
  const maxTotal = Math.max(1, ...rows.map((p) => p.total_tokens));

  return (
    <Card>
      <CTitle>项目用量（路径以不可逆 Hash 存储）</CTitle>
      <CBody>
        {rows.length === 0 ? (
          <div className="py-12 text-center text-sm text-[var(--text-tertiary)]">暂无项目数据</div>
        ) : (
          <div className="flex flex-col divide-y divide-[var(--border)]">
            {rows.map((p) => {
              const cost = formatUsd(p.cost_amount);
              const tools = p.tools ?? [];
              return (
                <div key={p.project_id} className="py-3">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-sm font-medium truncate">{p.display_name || p.project_id}</span>
                    <span className="flex items-center gap-2 shrink-0">
                      <span className="text-sm tabular-nums font-semibold">{formatTokens(p.total_tokens)}</span>
                      {cost && <span className="text-xs text-[var(--text-tertiary)] tabular-nums">{cost}</span>}
                    </span>
                  </div>

                  {/* 项目总量条 */}
                  <div className="mt-1.5 h-1.5 rounded-full bg-[var(--mute-bg)] overflow-hidden">
                    <div
                      className="h-full rounded-full bg-[var(--color-brand)]"
                      style={{ width: `${(p.total_tokens / maxTotal) * 100}%` }}
                    />
                  </div>

                  {/* 按工具堆叠拆分（对齐开源项目视图） */}
                  {tools.length > 0 && (
                    <>
                      <div className="mt-2 flex h-2 rounded-full overflow-hidden bg-[var(--mute-bg)]">
                        {tools.map((t, i) => (
                          <div
                            key={t.tool_id}
                            title={`${t.display_name} ${formatTokens(t.total_tokens)} (${t.share_percent.toFixed(1)}%)`}
                            style={{
                              width: `${t.share_percent}%`,
                              background: TOOL_COLORS[i % TOOL_COLORS.length],
                            }}
                          />
                        ))}
                      </div>
                      <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1">
                        {tools.map((t, i) => (
                          <span key={t.tool_id} className="inline-flex items-center gap-1 text-[11px] text-[var(--text-secondary)]">
                            <i
                              className="w-1.5 h-1.5 rounded-full"
                              style={{ background: TOOL_COLORS[i % TOOL_COLORS.length] }}
                            />
                            {t.display_name}
                            <b className="font-medium tabular-nums">{formatTokens(t.total_tokens)}</b>
                            <em className="not-italic text-[var(--text-tertiary)]">{t.share_percent.toFixed(0)}%</em>
                          </span>
                        ))}
                      </div>
                    </>
                  )}

                  <div className="mt-1.5 text-[11px] text-[var(--text-tertiary)] tabular-nums">
                    {p.session_count} 个会话
                    {p.last_active_at && (
                      <>
                        <span> · 最近活跃 {new Date(p.last_active_at).toLocaleString("zh-CN", { hour12: false })}</span>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </CBody>
    </Card>
  );
}
