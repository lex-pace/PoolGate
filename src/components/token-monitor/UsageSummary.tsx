import React from "react";
import { KPICard } from "@/components/ui/KPICard";
import { formatTokens } from "./token-monitor-data";
import type { TokenMonitorSnapshot } from "./token-monitor-data";

/** 用量总览：输入/输出/缓存/总 Tokens + 费用（金额缺省时显示「未知价格」，不填 0）。 */
export function UsageSummary({ snapshot }: { snapshot: TokenMonitorSnapshot }) {
  const { usage } = snapshot;
  const cost = usage.cost_amount != null ? `¥${usage.cost_amount.toFixed(2)}` : "未知价格";
  return (
    <div className="grid grid-cols-2 md:grid-cols-5 gap-3">
      <KPICard title="输入 Tokens" value={formatTokens(usage.input_tokens)} sub="input" />
      <KPICard title="输出 Tokens" value={formatTokens(usage.output_tokens)} sub="output" />
      <KPICard title="缓存 Tokens" value={formatTokens(usage.cache_tokens)} sub="cache（不计入 total）" />
      <KPICard title="总 Tokens" value={formatTokens(usage.total_tokens)} sub="total（不双算 cache）" />
      <KPICard title="估算费用" value={cost} sub={usage.cost_amount != null ? "USD · 估算" : "能力诚实：源未提供"} />
    </div>
  );
}
