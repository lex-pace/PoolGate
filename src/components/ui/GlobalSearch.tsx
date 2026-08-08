import React, { useCallback, useEffect, useRef, useState } from "react";
import { useAccounts, useGroups, useProviders } from "@/hooks/use-tauri";
import { FileText, Layers3, LayoutDashboard, Route, Search, Settings, Waypoints } from "lucide-react";

interface SearchResult {
  type: "resource" | "pool" | "page";
  id: string;
  label: string;
  sub?: string;
  page: string;
}

interface Props {
  open: boolean;
  onClose: () => void;
  onNavigate: (page: string) => void;
}

const pages = [
  { id: "dashboard", label: "指挥中心", icon: LayoutDashboard },
  { id: "resources", label: "模型供应商", icon: Layers3 },
  { id: "groups", label: "路由池", icon: Route },
  { id: "logs", label: "请求流", icon: FileText },
  { id: "analytics", label: "用量分析", icon: Waypoints },
  { id: "settings", label: "设置", icon: Settings },
];

export function GlobalSearch({ open, onClose, onNavigate }: Props) {
  const [query, setQuery] = useState("");
  const [selectedIdx, setSelectedIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const { data: providers = [] } = useProviders();
  const { data: accounts = [] } = useAccounts();
  const { data: groups = [] } = useGroups();

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIdx(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  const connectorNames = new Map(providers.map((provider) => [provider.id, provider.name]));
  const results: SearchResult[] = [];
  const q = query.toLowerCase();

  if (!q) {
    pages.forEach((page) => results.push({ type: "page", id: page.id, label: page.label, page: page.id }));
  } else {
    pages.forEach((page) => {
      if (page.label.toLowerCase().includes(q) || page.id.includes(q)) {
        results.push({ type: "page", id: page.id, label: page.label, page: page.id });
      }
    });
    accounts.forEach((account) => {
      const connectorName = account.provider_id ? connectorNames.get(account.provider_id) : undefined;
      const searchable = [account.name, account.email, account.models, account.source_format, account.credential_type, connectorName]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      if (searchable.includes(q)) {
        results.push({
          type: "resource",
          id: account.id,
          label: account.name || account.email || account.id,
          sub: `${connectorName || "自动连接器"} · ${account.models || account.credential_type || "模型待发现"}`,
          page: "resources",
        });
      }
    });
    groups.forEach((group) => {
      if (`${group.name} ${group.protocol} ${group.strategy || ""}`.toLowerCase().includes(q)) {
        results.push({ type: "pool", id: group.id, label: group.name, sub: `${group.protocol} · ${group.strategy || "轮询"}`, page: "groups" });
      }
    });
  }

  const handleKey = useCallback((event: React.KeyboardEvent) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIdx((index) => Math.min(index + 1, results.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIdx((index) => Math.max(index - 1, 0));
    } else if (event.key === "Enter" && results[selectedIdx]) {
      event.preventDefault();
      onNavigate(results[selectedIdx].page);
      onClose();
    } else if (event.key === "Escape") {
      onClose();
    }
  }, [results, selectedIdx, onNavigate, onClose]);

  useEffect(() => setSelectedIdx(0), [query]);
  useEffect(() => {
    const element = listRef.current?.children[selectedIdx] as HTMLElement;
    element?.scrollIntoView({ block: "nearest" });
  }, [selectedIdx]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-[90] flex items-start justify-center pt-[15vh]" onClick={onClose}>
      <div className="absolute inset-0 bg-black/45 backdrop-blur-sm" />
      <div className="relative w-full max-w-lg mx-4 rounded-xl border overflow-hidden animate-slide-up" style={{ background: "var(--bg-surface-solid)", borderColor: "var(--border-default)", boxShadow: "var(--shadow-elevated)" }} onClick={(event) => event.stopPropagation()}>
        <div className="flex items-center gap-3 px-4 border-b" style={{ borderColor: "var(--border-default)" }}>
          <Search size={16} className="text-[var(--text-dim)]" />
          <input ref={inputRef} value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={handleKey} placeholder="搜索模型供应商、模型能力和路由池..." className="flex-1 h-12 bg-transparent outline-none text-sm text-[var(--text-primary)]" />
          <kbd className="text-[10px] px-1.5 py-0.5 rounded border text-[var(--text-dim)]" style={{ borderColor: "var(--border-default)" }}>ESC</kbd>
        </div>
        <div ref={listRef} className="max-h-72 overflow-auto py-1">
          {results.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-[var(--text-dim)]">没有匹配的模型供应商或路由池</div>
          ) : results.map((result, index) => {
            const Icon = result.type === "resource" ? Layers3 : result.type === "pool" ? Route : pages.find((page) => page.id === result.id)?.icon || FileText;
            return (
              <button key={`${result.type}-${result.id}`} className="w-full flex items-center gap-3 px-4 py-2.5 text-sm text-left transition-colors" style={{ background: index === selectedIdx ? "var(--bg-hover)" : "transparent", color: "var(--text-primary)" }} onMouseEnter={() => setSelectedIdx(index)} onClick={() => { onNavigate(result.page); onClose(); }}>
                <span className="w-7 h-7 rounded-lg bg-[var(--bg-inset)] flex items-center justify-center text-[var(--color-brand)]"><Icon size={14} /></span>
                <div className="flex-1 min-w-0">
                  <div className="truncate">{result.label}</div>
                  {result.sub && <div className="text-[10px] truncate text-[var(--text-dim)]">{result.sub}</div>}
                </div>
                <span className="text-[9px] px-1.5 py-0.5 rounded bg-[var(--bg-inset)] text-[var(--text-dim)]">{result.type === "page" ? "页面" : result.type === "resource" ? "模型供应商" : "路由池"}</span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
