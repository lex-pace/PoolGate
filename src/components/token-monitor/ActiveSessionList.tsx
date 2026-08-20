import React from "react";
import { StatusDot } from "@/components/ui/StatusDot";
import ToolLogo, { ProviderLogo } from "./ToolLogo";
import {
  formatTokens, formatMessageCount, MULTI_MODEL_MSG_HINT, type SessionSummary,
} from "./token-monitor-data";

/** 会话列表：摘要只读（不读正文），标题为脱敏生成。
 *  消息数对齐开源 Token Monitor：千分位 + 单复数（1 msg / N msgs）；
 *  多模型会话的消息数为各模型分组之和（悬停提示口径）。 */
export function ActiveSessionList({ sessions }: { sessions: SessionSummary[] }) {
  if (sessions.length === 0) {
    return <div className="py-8 text-center text-sm text-[var(--text-tertiary)]">暂无会话记录</div>;
  }
  return (
    <div className="flex flex-col divide-y divide-[var(--border)]">
      {sessions.map((s) => {
        const msgLabel = formatMessageCount(s.message_count);
        const multiModel = s.model_set.length > 1;
        return (
          <div key={s.session_id} className="py-2.5 flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <ToolLogo toolId={s.tool_id} size={16} />
                <StatusDot status={s.status === "active" ? "active" : "idle"} pulse={s.status === "active"} />
                <span className="text-sm font-medium truncate">{s.title_redacted ?? s.session_id}</span>
              </div>
              <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-[var(--text-tertiary)] truncate">
                <ProviderLogo model={s.model_set[0]} size={12} />
                <span className="truncate">
                  {s.tool_id} · {s.model_set.join("、") || "模型不可用"}
                  {msgLabel && (
                    <>
                      {" · "}
                      <span title={multiModel ? MULTI_MODEL_MSG_HINT : undefined}>{msgLabel}</span>
                    </>
                  )}
                </span>
              </div>
            </div>
            <div className="text-right shrink-0">
              <div className="text-sm tabular-nums font-semibold">{formatTokens(s.total_tokens)}</div>
              <div className="text-[11px] text-[var(--text-tertiary)] tabular-nums">
                in {formatTokens(s.input_tokens)} · out {formatTokens(s.output_tokens)}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
