import React from "react";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import { useServiceStatus } from "@/components/token-monitor/token-monitor-data";
import { ProviderLogo } from "@/components/token-monitor/ToolLogo";
import type { ServiceStatusView } from "@/lib/token-monitor-commands";

const STATUS_TEXT: Record<string, string> = {
  ok: "正常",
  degraded: "降级",
  outage: "故障",
  unknown: "未知",
};

/** 服务状态（不显眼展示）：页面头部右侧一排小图标。
 *  绿边框 = 服务正常，灰 = 状态异常；悬停提示「应用名 · 状态」，点击跳官方状态页。 */
export default function ServiceStatusStrip() {
  const { data: statuses } = useServiceStatus();
  const rows = statuses ?? [];

  if (rows.length === 0) {
    return <span className="text-[10px] text-[var(--text-tertiary)]">服务状态…</span>;
  }
  return (
    <div className="flex items-center gap-1.5" role="group" aria-label="供应商服务状态">
      {rows.map((s: ServiceStatusView) => {
        const ok = s.status === "ok";
        const label = STATUS_TEXT[s.status] ?? "未知";
        return (
          <button
            key={s.provider_id}
            type="button"
            onClick={() => void openExternal(s.page_url).catch(() => undefined)}
            title={`${s.label} · ${label}`}
            aria-label={`${s.label} · ${label}，点击打开官方状态页`}
            className={`w-7 h-7 grid place-items-center rounded-lg border-2 transition-transform hover:scale-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] ${
              ok ? "border-[var(--color-ok)]" : "border-[var(--border-strong)]"
            }`}
            style={{
              background: "var(--mute-bg)",
              filter: ok ? undefined : "grayscale(1)",
            }}
          >
            <ProviderLogo provider={s.provider_id} size={16} />
          </button>
        );
      })}
    </div>
  );
}
