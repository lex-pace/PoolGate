import React, { useState } from "react";
import { Card } from "@/components/ui/Card";
import { Modal } from "@/components/ui/Modal";
import { ModeSettings } from "@/components/AppMode";
import { Activity, ArrowLeft, Gauge, Settings2, TrendingUp, Wrench } from "lucide-react";
import Overview from "./Overview";
import Trend from "./Trend";
import Tools from "./Tools";
import ServiceStatusStrip from "./Status";
import { useTokenMonitorRealtime } from "@/components/token-monitor/token-monitor-data";

// 仪表盘视图：总览（含服务状态、统计卡、滚动一年热力图、按模型/按工具）+ 趋势 +
// 工具（采集状态、自定义应用监控——托盘不承载的维护入口）。
// 今日/本月/累计 范围切换在总览页顶部（只影响总览统计，趋势有独立范围）。
export type TmTab = "overview" | "trend" | "tools";

const tabs: Array<{ id: TmTab; label: string; icon: React.ElementType }> = [
  { id: "overview", label: "总览", icon: Gauge },
  { id: "trend", label: "趋势", icon: TrendingUp },
  { id: "tools", label: "工具", icon: Wrench },
];

export default function TokenMonitorPage({
  initialTab,
  onInitialTabConsumed,
  onExit,
}: {
  initialTab?: string;
  onInitialTabConsumed?: () => void;
  /** 桌面端全屏仪表盘模式：提供「返回 PoolGate」入口（菜单内嵌模式不传）。 */
  onExit?: () => void;
}) {
  const [tab, setTab] = useState<TmTab>("overview");
  const [modeSettingsOpen, setModeSettingsOpen] = useState(false);

  // 实时刷新：监听后端 `token-monitor:usage-delta` 事件 → 立即失效用量查询（TOKENS 秒级变化）；
  // 各查询自身的 refetchInterval 兜底（对齐开源 Token Monitor 的文件监听 + 周期刷新两段式）。
  useTokenMonitorRealtime();

  // 深链直达 tab（如托盘跳转）；未知/已移除的 tab 回退到总览。一次性消费后回调清除。
  React.useEffect(() => {
    if (initialTab === "settings") {
      setModeSettingsOpen(true);
      onInitialTabConsumed?.();
    } else if (initialTab && tabs.some((t) => t.id === initialTab)) {
      setTab(initialTab as TmTab);
      onInitialTabConsumed?.();
    } else if (initialTab) {
      // 旧版托盘齿轮指向已移除的 settings tab → 回退总览
      onInitialTabConsumed?.();
    }
  }, [initialTab, onInitialTabConsumed]);

  return (
    <div className="pg-tm-dash flex flex-col h-full min-h-0">
      {/* 工具栏：与 PoolGate 主界面（pg-toolbar）同款 54px 头部，tab 导航居中 */}
      <header
        className="pg-toolbar h-[54px] flex items-center gap-4 px-4 border-b shrink-0"
        style={{ borderColor: "var(--border-default)" }}
        data-tauri-drag-region
      >
        <div className="flex items-center gap-3 flex-1 min-w-0">
          {onExit && (
            <button
              onClick={onExit}
              className="w-7 h-7 flex items-center justify-center rounded-md text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
              title="返回 PoolGate"
              aria-label="返回 PoolGate"
            >
              <ArrowLeft size={15} />
            </button>
          )}
          <div className="w-px h-4 bg-[var(--border-default)]" />
          <div className="min-w-0">
            <h1 className="text-[13px] leading-4 font-semibold tracking-[-0.01em] text-[var(--text-primary)] flex items-center gap-1.5">
              <Activity className="w-3.5 h-3.5 text-[var(--color-brand)]" />
              Token Monitor 仪表盘
            </h1>
            <p className="text-[9px] leading-3 text-[var(--text-dim)] truncate">
              本地 AI 工具用量与额度控制台 · 只读元数据，不持久化 Prompt/代码
            </p>
          </div>
        </div>
        {/* 总览 / 趋势：居中导航（复用 pg-seg 分段样式，占位更小） */}
        <div className="pg-seg pg-seg-md" role="tablist" aria-label="仪表盘视图">
          {tabs.map((t) => {
            const Icon = t.icon;
            const active = tab === t.id;
            return (
              <button
                key={t.id}
                role="tab"
                aria-selected={active}
                onClick={() => setTab(t.id)}
                className={`pg-seg-item ${active ? "active" : ""}`}
              >
                <Icon className="w-3.5 h-3.5" />
                {t.label}
              </button>
            );
          })}
        </div>
        <div className="flex items-center justify-end gap-3 flex-1">
          <button
            type="button"
            onClick={() => setModeSettingsOpen(true)}
            className="h-7 px-2 rounded-md inline-flex items-center gap-1.5 text-[11px] text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
            title="产品模式"
          >
            <Settings2 size={13} /> 模式
          </button>
          <ServiceStatusStrip />
        </div>
      </header>
      <Modal open={modeSettingsOpen} onClose={() => setModeSettingsOpen(false)} title="产品模式">
        <ModeSettings />
      </Modal>

      <main className="pg-content flex-1 min-h-0 overflow-auto px-5 py-5 select-text">
        <Card className="p-0 overflow-hidden">
          <div className="p-4">
            {tab === "overview" && <Overview />}
            {tab === "trend" && <Trend />}
            {tab === "tools" && <Tools range="day" />}
          </div>
        </Card>
      </main>
    </div>
  );
}
