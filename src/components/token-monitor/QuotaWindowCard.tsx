import React from "react";
import { Badge } from "@/components/ui/Badge";
import type { QuotaWindowView } from "@/lib/token-monitor-commands";

const unitLabel: Record<string, string> = {
  tokens: "Tokens", requests: "请求", credits: "Credits", currency: "余额", percent: "%",
};

/** 额度窗口卡：剩余百分比条 + 重置时间。缺失字段显示「不可用」，不猜不填。 */
export function QuotaWindowCard({ window }: { window: QuotaWindowView }) {
  const percent = window.remaining_percent;
  const tone = percent == null ? "mute" : percent <= 5 ? "err" : percent <= 10 ? "warn" : percent <= 20 ? "warn" : "ok";
  const showPercent = percent != null;
  const used = window.used_value != null ? window.used_value.toLocaleString() : null;
  const limit = window.limit_value != null ? window.limit_value.toLocaleString() : null;

  return (
    <div className="rounded-xl border border-[var(--border)] bg-[var(--card-bg)] p-3">
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium truncate">{window.label || window.window_key}</span>
        <Badge variant={tone} dot>{showPercent ? `${percent!.toFixed(0)}% 剩余` : "无数据"}</Badge>
      </div>
      <div className="mt-2 h-2 rounded-full bg-[var(--mute-bg)] overflow-hidden">
        <div
          className={`h-full rounded-full ${tone === "err" ? "bg-[var(--color-err)]" : tone === "warn" ? "bg-[var(--color-warn)]" : "bg-[var(--color-ok)]"}`}
          style={{ width: `${showPercent ? Math.max(0, Math.min(100, percent!)) : 0}%` }}
        />
      </div>
      <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--text-tertiary)] tabular-nums">
        <span>{used != null ? `${used} / ${limit ?? "∞"}` : "用量不可用"}</span>
        <span>{window.resets_at ? `重置 ${new Date(window.resets_at).toLocaleString("zh-CN", { hour12: false })}` : "无重置时间"}</span>
      </div>
      <div className="mt-1 flex items-center gap-1.5 text-[10px] text-[var(--text-tertiary)]">
        <span>单位 {unitLabel[window.unit] ?? window.unit}</span>
        <span>·</span>
        <span>来源 {window.source}</span>
        <span>·</span>
        <span>{window.confidence === "stale" ? "数据可能过期" : window.confidence}</span>
      </div>
    </div>
  );
}
