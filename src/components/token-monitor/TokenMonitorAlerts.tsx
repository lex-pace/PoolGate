import { useCallback } from "react";
import { useToast } from "@/components/ui/Toast";
import { useTokenMonitorAlerts, type TmAlert } from "./token-monitor-data";

/** 告警级别 → Toast 类型（remind 信息 / warn 警告 / critical 错误强调）。 */
const LEVEL_TOAST_TYPE: Record<string, "success" | "error" | "warning" | "info"> = {
  remind: "info",
  warn: "warning",
  critical: "error",
};

/**
 * 全局挂载一次（须在 ToastProvider 内）：监听 `token-monitor:alert`，
 * 系统通知由数据层 `useTokenMonitorAlerts` 负责（跨窗口去重），
 * 这里把告警转成应用内 Toast（桌面与托盘各自挂载，各显示各的）。
 */
export default function TokenMonitorAlerts() {
  const { toast } = useToast();
  const handleAlert = useCallback(
    (alert: TmAlert) => {
      toast(LEVEL_TOAST_TYPE[alert.level] ?? "info", alert.message, 6000);
    },
    [toast],
  );
  useTokenMonitorAlerts(handleAlert);
  return null;
}
