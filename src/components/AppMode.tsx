import { useState } from "react";
import { Check, Gauge, Layers3, RefreshCw, ShieldCheck } from "lucide-react";
import { relaunch } from "@tauri-apps/plugin-process";
import { Card } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { useAppMode, useSetAppMode } from "@/hooks/use-tauri";
import type { AppMode } from "@/lib/tauri-commands";

const modeCopy: Record<AppMode, {
  title: string;
  subtitle: string;
  description: string;
  features: string[];
  icon: typeof Gauge;
}> = {
  gateway: {
    title: "Gateway 完整模式",
    subtitle: "模型网关 + Token Monitor",
    description: "适合需要统一管理模型账号、路由池和本地 Agent 接入的用户。",
    features: ["多供应商与 OAuth 账号池", "OpenAI / Anthropic / Gemini 兼容网关", "路由、故障转移、请求日志", "内置 Token Monitor 本地用量监控"],
    icon: Layers3,
  },
  monitor: {
    title: "Monitor 专注模式",
    subtitle: "仅 Token Monitor",
    description: "适合只想查看本机 AI 编码工具 Tokens、会话、趋势和额度的用户。",
    features: ["Claude Code、Codex、Cursor 等工具用量", "Tokens、模型、会话和项目趋势", "额度告警与后台采集", "系统托盘快速查看，完全不启动网关"],
    icon: Gauge,
  },
};

export function ModeChoiceCards({
  onSelect,
  busy = false,
}: {
  onSelect: (mode: AppMode) => void | Promise<void>;
  busy?: boolean;
}) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4 max-w-3xl w-full">
      {(Object.keys(modeCopy) as AppMode[]).map((mode) => {
        const copy = modeCopy[mode];
        const Icon = copy.icon;
        return (
          <Card key={mode} className="p-5 flex flex-col gap-4 border transition-colors hover:border-[var(--color-brand)]">
            <div className="flex items-start gap-3">
              <div className="w-10 h-10 rounded-xl grid place-items-center bg-[var(--color-brand)]/10 text-[var(--color-brand)]">
                <Icon size={21} />
              </div>
              <div>
                <h2 className="text-base font-semibold text-[var(--text-primary)]">{copy.title}</h2>
                <p className="text-xs mt-1 text-[var(--color-brand)]">{copy.subtitle}</p>
              </div>
            </div>
            <p className="text-sm leading-6 text-[var(--text-secondary)]">{copy.description}</p>
            <ul className="space-y-2 flex-1">
              {copy.features.map((feature) => (
                <li key={feature} className="flex items-start gap-2 text-xs text-[var(--text-secondary)]">
                  <Check size={14} className="mt-0.5 shrink-0 text-[var(--color-ok)]" />
                  <span>{feature}</span>
                </li>
              ))}
            </ul>
            <Button variant={mode === "gateway" ? "primary" : "secondary"} disabled={busy} onClick={() => void onSelect(mode)}>
              {busy ? "正在保存..." : mode === "gateway" ? "使用 Gateway 完整模式" : "使用 Monitor 模式"}
            </Button>
          </Card>
        );
      })}
    </div>
  );
}

export function ModeSelectionScreen({ onSelected }: { onSelected?: () => void }) {
  const setMode = useSetAppMode();
  const [busy, setBusy] = useState(false);
  const select = async (mode: AppMode) => {
    setBusy(true);
    try {
      await setMode.mutateAsync(mode);
      onSelected?.();
      await relaunch();
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="pg-app min-h-screen flex flex-col items-center justify-center px-6 py-10">
      <div className="max-w-3xl w-full mb-8 text-center">
        <img src="/poolgate-icon.png" alt="PoolGate" className="w-12 h-12 mx-auto mb-4" />
        <div className="pg-eyebrow mb-2">Welcome to PoolGate</div>
        <h1 className="text-2xl font-semibold text-[var(--text-primary)]">选择 PoolGate 的使用模式</h1>
        <p className="text-sm leading-6 mt-3 text-[var(--text-dim)]">
          Gateway 是完整产品形态，包含网关与 Token Monitor；Monitor 模式只展示本地工具用量监控，不启动或暴露任何网关功能。
        </p>
      </div>
      <ModeChoiceCards onSelect={select} busy={busy} />
      <p className="mt-6 text-xs text-[var(--text-dim)]">之后可在设置中的「产品模式」切换，切换后 PoolGate 会自动重启；已有数据不会删除。</p>
    </div>
  );
}

export function ModeSettings() {
  const { data: appMode, isLoading } = useAppMode();
  const setMode = useSetAppMode();
  const [busy, setBusy] = useState(false);
  const current = appMode?.mode ?? "gateway";
  const switchMode = async () => {
    const next: AppMode = current === "gateway" ? "monitor" : "gateway";
    const confirmed = window.confirm(
      next === "monitor"
        ? "切换到 Monitor 模式？网关能力会被隐藏并禁止启动，PoolGate 将自动重启。"
        : "切换到 Gateway 完整模式？PoolGate 将自动重启并恢复网关、账号池和路由功能。",
    );
    if (!confirmed) return;
    setBusy(true);
    try {
      await setMode.mutateAsync(next);
      await relaunch();
    } finally {
      setBusy(false);
    }
  };
  if (isLoading) return <div className="text-xs text-[var(--text-dim)]">正在读取产品模式...</div>;
  return (
    <div className="space-y-3">
      <div className="flex items-start gap-3">
        <ShieldCheck size={18} className="mt-0.5 text-[var(--color-brand)]" />
        <div className="flex-1">
          <div className="text-sm font-medium text-[var(--text-primary)]">当前模式：{current === "gateway" ? "Gateway 完整模式" : "Monitor 专注模式"}</div>
          <p className="text-xs leading-5 mt-1 text-[var(--text-dim)]">
            {current === "gateway" ? "包含 Gateway 与 Token Monitor 的全部功能。" : "只展示 Token Monitor；网关命令在 Rust 后端也会被拒绝。"}
          </p>
        </div>
      </div>
      <Button variant="secondary" size="sm" disabled={busy} onClick={() => void switchMode()}>
        <RefreshCw size={13} className={busy ? "animate-spin" : ""} />
        {busy ? "重启中..." : current === "gateway" ? "切换到 Monitor 模式" : "切换到 Gateway 完整模式"}
      </Button>
    </div>
  );
}
