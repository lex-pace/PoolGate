import React from "react";

const dotMap: Record<string, string> = {
  ok: "status-dot ok",
  warn: "status-dot warn",
  err: "status-dot err",
  mute: "status-dot mute",
  pulse_ok: "status-dot ok animate-pulse-green",
  pulse_err: "status-dot err animate-pulse-red",
};

const labelMap: Record<string, string> = {
  ok: "正常", healthy: "正常", active: "活跃", running: "运行中", success: "成功",
  warn: "警告", limited: "受限", warning: "警告",
  err: "异常", error: "错误", failed: "失败", exhausted: "耗尽", stopped: "已停止",
  mute: "离线", disabled: "已禁用", unchecked: "未检查", offline: "离线",
};

export function StatusDot({ status, label, pulse, className = "" }: {
  status: string; label?: boolean; pulse?: boolean; className?: string;
}) {
  const key = pulse ? `pulse_${status}` : status;
  const cls = dotMap[key] || dotMap[status] || "status-dot mute";
  const lb = labelMap[status] || status;
  return (
    <span className={`inline-flex items-center gap-1.5 ${className}`}>
      <span className={cls} />
      {label && <span className="text-xs text-[var(--text-secondary)]">{lb}</span>}
    </span>
  );
}
