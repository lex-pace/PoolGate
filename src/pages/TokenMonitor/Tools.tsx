import React, { useState } from "react";
import { Card } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Modal } from "@/components/ui/Modal";
import { Switch } from "@/components/ui/Switch";
import { Plus, RefreshCw, ScanLine, X } from "lucide-react";
import ToolLogo from "@/components/token-monitor/ToolLogo";
import { useCollectorStatus, scanAllTools, rescanTool } from "@/components/token-monitor/token-monitor-data";
import { addCustomApp, detectLocalAgents, enableToolMonitoring, removeCustomApp, setToolCollection, type CustomFields, type DetectedAgent, type ToolCollectorState } from "@/lib/token-monitor-commands";
import type { CollectorStatus } from "@/lib/token-monitor-commands";

// ── 采集状态 → 圆点/短标签 ──
const STATUS_TONE: Record<CollectorStatus, "on" | "warn" | "err" | ""> = {
  active: "on", idle: "", waiting: "warn", permission: "warn",
  path_missing: "warn", format_changed: "warn", partial: "warn", error: "err",
};
const STATUS_LABEL: Record<CollectorStatus, string> = {
  active: "监控中", idle: "空闲", waiting: "等待中", permission: "需授权",
  path_missing: "路径缺失", format_changed: "格式变化", partial: "部分异常", error: "异常",
};

// ── 添加自定义应用（Modal 内表单）──
const FIELD_LABELS: { key: keyof CustomFields; label: string }[] = [
  { key: "input", label: "输入 tokens" },
  { key: "output", label: "输出 tokens" },
  { key: "cache", label: "缓存 tokens" },
  { key: "model", label: "模型名" },
  { key: "ts", label: "时间戳" },
  { key: "cost", label: "成本" },
];

function AddCustomAppModal({ open, onClose, onDone }: { open: boolean; onClose: () => void; onDone: () => void }) {
  const [name, setName] = useState("");
  const [pathsText, setPathsText] = useState("");
  const [showFields, setShowFields] = useState(false);
  const [fields, setFields] = useState<Record<keyof CustomFields, string>>({
    input: "", output: "", cache: "", model: "", ts: "", cost: "",
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const reset = () => {
    setName(""); setPathsText("");
    setFields({ input: "", output: "", cache: "", model: "", ts: "", cost: "" });
    setShowFields(false); setError("");
  };

  const handleAdd = async () => {
    setError("");
    const paths = pathsText.split("\n").map((p) => p.trim()).filter(Boolean);
    if (!name.trim()) { setError("请填写应用名"); return; }
    if (paths.length === 0) { setError("请至少填写一个 JSONL 日志路径"); return; }
    setBusy(true);
    try {
      const fieldsOpt: CustomFields | undefined = showFields
        ? Object.fromEntries(
            FIELD_LABELS.map(({ key }) => [key, fields[key].split(",").map((s) => s.trim()).filter(Boolean)]),
          ) as unknown as CustomFields
        : undefined;
      const hasAnyField = fieldsOpt && Object.values(fieldsOpt).some((v) => v.length > 0);
      await addCustomApp({ display_name: name.trim(), paths, fields: hasAnyField ? fieldsOpt : undefined });
      reset();
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal open={open} onClose={onClose} title="添加自定义应用">
      <div className="flex flex-col gap-3">
        <Input label="应用名" placeholder="如：我的 Agent 应用" value={name} onChange={(e) => setName(e.target.value)} />
        <Input
          label="JSONL 日志路径（每行一个，支持通配符）"
          placeholder={"~/logs/myapp/*.jsonl"}
          value={pathsText}
          onChange={(e) => setPathsText(e.target.value)}
        />
        <button
          type="button"
          onClick={() => setShowFields((v) => !v)}
          className="self-start text-xs text-[var(--color-brand)] hover:underline cursor-pointer"
        >
          {showFields ? "收起字段映射 ▲" : "高级：字段映射（可选）▼"}
        </button>
        {showFields && (
          <div className="flex flex-col gap-2 rounded-lg border border-[var(--border-default)] p-3">
            {FIELD_LABELS.map(({ key, label }) => (
              <Input
                key={key}
                label={label}
                placeholder={`/usage/${key === "ts" ? "created_at" : key + "_tokens"}，逗号分隔多个`}
                value={fields[key]}
                onChange={(e) => setFields((f) => ({ ...f, [key]: e.target.value }))}
              />
            ))}
            <p className="text-[11px] text-[var(--text-dim)]">支持 JSON Pointer，如 /usage/input_tokens、/model；留空用内置通用默认字段</p>
          </div>
        )}
        {error && <p className="text-xs text-[var(--color-err)]">{error}</p>}
        <div className="flex justify-end gap-2">
          <Button size="sm" variant="ghost" onClick={onClose}>取消</Button>
          <Button size="sm" onClick={handleAdd} disabled={busy}>{busy ? "添加中…" : "添加并开始监控"}</Button>
        </div>
      </div>
    </Modal>
  );
}

// ── 工具卡片：Logo（悬停→刷新）+ 名称 + 状态 + 开关 +（自定义：删除）──
function ToolCard({
  c,
  busyRescan,
  onRescan,
  onToggle,
  onDelete,
}: {
  c: ToolCollectorState;
  busyRescan: string | null;
  onRescan: (toolId: string) => void;
  onToggle: (toolId: string, enabled: boolean) => void;
  onDelete?: (c: ToolCollectorState) => void;
}) {
  const [forced, setForced] = useState(false);
  const busy = busyRescan === c.tool_id;
  // 状态标签：关闭 → 「已关闭」；开启且异常 → 保留异常提示；开启且健康 → 「监控中」
  const label = c.enabled
    ? c.status === "error" || c.status === "format_changed" || c.status === "permission"
      ? STATUS_LABEL[c.status] ?? c.status
      : "监控中"
    : "已关闭";
  const tone = c.enabled ? STATUS_TONE[c.status] ?? "" : "";
  return (
    <div className={`pg-tool-card ${c.enabled ? "" : "is-off"}`}>
      <div
        className="pg-tool-logo-box"
        onClick={() => setForced((v) => !v)}
        title="点击 Logo 重扫该工具"
        aria-label={`重扫 ${c.display_name}`}
      >
        <ToolLogo toolId={c.tool_id} displayName={c.display_name} size={38} />
        <button
          type="button"
          className={`pg-tool-logo-refresh ${forced ? "forced" : ""}`}
          onClick={(e) => { e.stopPropagation(); onRescan(c.tool_id); }}
          aria-label={`重扫 ${c.display_name}`}
        >
          <RefreshCw size={15} className={busy ? "spin" : ""} />
        </button>
      </div>
      <span className="pg-tool-name" title={c.display_name}>{c.display_name}</span>
      <span className="pg-tool-meta" title={c.enabled ? `采集状态：${STATUS_LABEL[c.status] ?? c.status}` : "已关闭监控，统计中已剔除该工具"}>
        <i className={`pg-tool-dot ${tone}`} />
        {label}
      </span>
      {onDelete && (
        <button type="button" className="pg-tool-del" onClick={() => onDelete(c)} aria-label={`删除 ${c.display_name}`}>
          <X size={12} />
        </button>
      )}
      <Switch
        size="sm"
        checked={c.enabled}
        onChange={(v) => onToggle(c.tool_id, v)}
        ariaLabel={`${c.display_name}${c.enabled ? "关闭" : "加入"} Token 监控`}
      />
    </div>
  );
}

// ── 一键扫描添加：检测到的本机 Agent 工具行 ──
function DetectRow({
  agent,
  checked,
  onToggle,
}: {
  agent: DetectedAgent;
  checked: boolean;
  onToggle: (toolId: string, next: boolean) => void;
}) {
  // 可添加 = 未监控 && 有适配器（或 tokscale 覆盖且引擎可用）&& 本机有安装痕迹
  const addable =
    !agent.monitored &&
    (agent.has_adapter || (agent.covered_by_tokscale && agent.tokscale_available)) &&
    (agent.installed || agent.data_found);
  const badge = agent.monitored
    ? { cls: "on", text: "已监控" }
    : agent.data_found
      ? { cls: "data", text: "发现数据" }
      : agent.installed
        ? { cls: "inst", text: "已安装" }
        : agent.covered_by_tokscale
          ? { cls: "ts", text: "tokscale 覆盖" }
          : { cls: "off", text: "未安装" };
  const hint = agent.data_sources[0] ?? agent.cli ?? agent.vendor ?? "";
  return (
    <div className={`pg-detect-row ${checked ? "is-checked" : ""}`}>
      <ToolLogo toolId={agent.tool_id} displayName={agent.display_name} size={26} />
      <div className="flex flex-col min-w-0">
        <span className="pg-tool-name" title={agent.display_name}>{agent.display_name}</span>
        {hint && <span className="pg-detect-hint" title={hint}>{hint}</span>}
      </div>
      <em className={`pg-detect-badge ${badge.cls}`}>{badge.text}</em>
      <Switch
        size="sm"
        checked={agent.monitored || (addable && checked)}
        disabled={agent.monitored || !addable}
        onChange={(next) => onToggle(agent.tool_id, next)}
        ariaLabel={`${agent.monitored ? "已监控" : addable ? "添加" : "不可添加"} ${agent.display_name}`}
      />
    </div>
  );
}

// ── 分区：标题 + 计数 + 右侧操作 + 卡片网格 ──
function ToolSection({
  title,
  count,
  action,
  tools,
  busyRescan,
  onRescan,
  onToggle,
  onDelete,
  emptyText,
}: {
  title: string;
  count: number;
  action?: React.ReactNode;
  tools: ToolCollectorState[];
  busyRescan: string | null;
  onRescan: (toolId: string) => void;
  onToggle: (toolId: string, enabled: boolean) => void;
  onDelete?: (c: ToolCollectorState) => void;
  emptyText: string;
}) {
  return (
    <section className="pg-tools-section">
      <div className="pg-tools-head">
        <span className="pg-tools-title">
          {title}
          <em className="pg-tools-count">{count}</em>
        </span>
        {action}
      </div>
      {tools.length === 0 ? (
        <p className="text-xs text-[var(--text-dim)] py-4">{emptyText}</p>
      ) : (
        <div className="pg-tools-grid">
          {tools.map((c) => (
            <ToolCard key={c.tool_id} c={c} busyRescan={busyRescan} onRescan={onRescan} onToggle={onToggle} onDelete={onDelete} />
          ))}
        </div>
      )}
    </section>
  );
}

export default function Tools({ range: _range }: { range: "day" }) {
  const { data: collectors, refetch } = useCollectorStatus();
  const [scanning, setScanning] = useState(false);
  const [busyRescan, setBusyRescan] = useState<string | null>(null);
  const [busyToggle, setBusyToggle] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  // 一键扫描添加：检测结果 + 勾选集合
  const [detected, setDetected] = useState<DetectedAgent[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [detecting, setDetecting] = useState(false);
  const [adding, setAdding] = useState(false);

  const list = collectors ?? [];
  const custom = list.filter((c) => c.tool_id.startsWith("custom:"));
  const builtin = list.filter((c) => !c.tool_id.startsWith("custom:"));
  const enabledCount = list.filter((c) => c.enabled).length;

  // 一键扫描：检测本机已安装的 Agent 工具，预选「有数据且可添加」的工具
  const handleDetect = async () => {
    setDetecting(true);
    try {
      const list = await detectLocalAgents();
      setDetected(list);
      setSelected(
        new Set(
          list
            .filter((a) => !a.monitored && (a.has_adapter || (a.covered_by_tokscale && a.tokscale_available)) && a.data_found)
            .map((a) => a.tool_id),
        ),
      );
    } catch {
      // invoke 不可用（预览环境）→ 静默
    } finally {
      setDetecting(false);
    }
  };

  const toggleSelect = (toolId: string, next: boolean) => {
    setSelected((prev) => {
      const nextSet = new Set(prev);
      if (next) nextSet.add(toolId);
      else nextSet.delete(toolId);
      return nextSet;
    });
  };

  // 一键添加所选：注册适配器 + 立即采集，刷新监控列表
  const handleAddSelected = async () => {
    const ids = Array.from(selected);
    if (ids.length === 0) return;
    setAdding(true);
    try {
      await enableToolMonitoring(ids);
      await refetch();
      setDetected(null);
      setSelected(new Set());
    } catch {
      // 静默
    } finally {
      setAdding(false);
    }
  };

  // 本地扫描：重扫本机所有已知工具/日志源，刷新监控列表
  const handleScan = async () => {
    setScanning(true);
    try {
      await scanAllTools();
      await refetch();
    } catch {
      // invoke 不可用（预览环境）→ 静默
    } finally {
      setScanning(false);
    }
  };

  const handleRescan = async (toolId: string) => {
    setBusyRescan(toolId);
    try {
      await rescanTool(toolId);
      await refetch();
    } catch {
      // 静默
    } finally {
      setBusyRescan(null);
    }
  };

  // 勾选 = 加入/移出 Token 监控
  const handleToggle = async (toolId: string, enabled: boolean) => {
    setBusyToggle(toolId);
    try {
      await setToolCollection(toolId, enabled);
      await refetch();
    } catch {
      // 静默
    } finally {
      setBusyToggle(null);
    }
  };

  const handleDelete = async (c: ToolCollectorState) => {
    if (!window.confirm(`删除自定义应用「${c.display_name}」？其用量/会话记录将一并删除。`)) return;
    try {
      await removeCustomApp(c.tool_id);
      await refetch();
    } catch {
      // 静默
    }
  };

  return (
    <div className="flex flex-col gap-6">
      {/* 页头：扫描 + 概览 */}
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-[15px] font-semibold text-[var(--text-primary)] flex items-center gap-2">
            工具监控
            <em className="pg-tools-count">{enabledCount}/{list.length} 个监控中</em>
          </h2>
          <p className="text-[11px] text-[var(--text-dim)] mt-0.5">
            扫描本机日志源，勾选开关即可加入/移出 Token 监控；鼠标悬停 Logo 可单独重扫
          </p>
        </div>
        <Button size="sm" onClick={handleScan} disabled={scanning}>
          <ScanLine size={13} className={scanning ? "spin" : ""} />
          {scanning ? "扫描中…" : "扫描本机"}
        </Button>
      </div>

      {/* 一键扫描添加：通用本机 Agent 工具扫描 → 勾选 → 加入 TOKENS 监控 */}
      <section className="pg-tools-section">
        <div className="pg-tools-head">
          <span className="pg-tools-title">
            一键扫描添加
            <em className="pg-tools-count">
              {detected ? `${detected.filter((a) => !a.monitored && (a.has_adapter || (a.covered_by_tokscale && a.tokscale_available)) && (a.installed || a.data_found)).length} 个可添加` : "检测本机安装的 Agent 工具"}
            </em>
          </span>
          <div className="flex items-center gap-2">
            {detected && selected.size > 0 && (
              <Button size="sm" onClick={handleAddSelected} disabled={adding}>
                <Plus size={13} />
                {adding ? "添加中…" : `添加所选（${selected.size}）`}
              </Button>
            )}
            <Button size="sm" variant={detected ? "outline" : "default"} onClick={handleDetect} disabled={detecting}>
              <ScanLine size={13} className={detecting ? "spin" : ""} />
              {detecting ? "扫描中…" : detected ? "重新扫描" : "扫描本机 Agent"}
            </Button>
          </div>
        </div>
        {!detected ? (
          <p className="text-xs text-[var(--text-dim)] py-3">
            扫描本机已安装的 Agent 工具（CLI 二进制 + 数据目录），勾选后一键加入 TOKENS 监控；
            新增工具的用量自动计入今日/托盘 TOKENS 汇总（tokscale 聚合 + 自定义应用同一链路）。
          </p>
        ) : detected.length === 0 ? (
          <p className="text-xs text-[var(--text-dim)] py-3">未检测到已知 Agent 工具。</p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {detected.map((agent) => (
              <DetectRow
                key={agent.tool_id}
                agent={agent}
                checked={selected.has(agent.tool_id)}
                onToggle={toggleSelect}
              />
            ))}
          </div>
        )}
      </section>

      <ToolSection
        title="Token 监控应用"
        count={builtin.length}
        tools={builtin}
        busyRescan={busyRescan}
        onRescan={handleRescan}
        onToggle={handleToggle}
        emptyText="未发现内置工具，点击右上角「扫描本机」重新检测。"
      />

      <ToolSection
        title="自定义应用"
        count={custom.length}
        tools={custom}
        busyRescan={busyRescan}
        onRescan={handleRescan}
        onToggle={handleToggle}
        onDelete={handleDelete}
        emptyText="尚未添加自定义应用。点击「添加应用」注册你自己的 JSONL 日志源。"
        action={
          <Button size="sm" variant="outline" onClick={() => setAddOpen(true)}>
            <Plus size={13} />添加应用
          </Button>
        }
      />

      <AddCustomAppModal open={addOpen} onClose={() => setAddOpen(false)} onDone={() => { setAddOpen(false); void refetch(); }} />
    </div>
  );
}
