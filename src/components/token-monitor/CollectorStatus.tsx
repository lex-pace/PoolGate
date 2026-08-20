import React from "react";
import { StatusDot } from "@/components/ui/StatusDot";
import { Badge } from "@/components/ui/Badge";
import { collectorStatusText, collectorStatusTone, type ToolCollectorState } from "./token-monitor-data";

/** 采集状态清单：开关 + 状态点 + 错误信息（解析失败隔离到单工具）。 */
export function CollectorStatus({ collectors }: { collectors: ToolCollectorState[] }) {
  if (collectors.length === 0) {
    return (
      <div className="py-10 text-center">
        <p className="text-sm text-[var(--text-tertiary)]">尚未扫描本机工具</p>
        <p className="mt-1 text-xs text-[var(--text-tertiary)]">点击「扫描本机」发现 Claude Code / Codex / Cursor 等数据源</p>
      </div>
    );
  }
  return (
    <div className="flex flex-col divide-y divide-[var(--border)]">
      {collectors.map((c) => {
        const tone = collectorStatusTone[c.status];
        return (
          <div key={c.tool_id} className="py-2.5 flex items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <StatusDot status={tone === "ok" ? "ok" : tone === "err" ? "err" : tone === "warn" ? "warn" : "mute"} label={false} pulse={c.status === "active"} />
                <span className="text-sm font-medium">{c.display_name}</span>
                <Badge variant={tone}>{collectorStatusText[c.status]}</Badge>
              </div>
              <div className="mt-0.5 text-[11px] text-[var(--text-tertiary)] truncate max-w-[420px]">
                {c.paths.length > 0 ? c.paths.join("、") : "默认路径未发现"}
                {c.error ? ` · ${c.error}` : ""}
              </div>
            </div>
            <div className="text-right shrink-0 text-[11px] text-[var(--text-tertiary)]">
              {c.enabled ? "已启用" : "已禁用"}
              {c.last_collected_at ? ` · ${new Date(c.last_collected_at).toLocaleTimeString("zh-CN", { hour12: false })}` : ""}
            </div>
          </div>
        );
      })}
    </div>
  );
}
