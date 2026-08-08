import React, { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Spinner } from "@/components/ui/Spinner";
import { useAppLogs, useAppLogInfo } from "@/hooks/use-tauri";
import { Search, Download, RefreshCw, ChevronLeft, ChevronRight, ArrowDownToLine, Terminal } from "lucide-react";

// ─── Constants ───────────────────────────────────────────────────────────────

const refreshIntervals = [
  { value: 0, label: "关闭" },
  { value: 1, label: "1秒" },
  { value: 3, label: "3秒" },
  { value: 5, label: "5秒" },
  { value: 10, label: "10秒" },
  { value: 30, label: "30秒" },
];

const AUTO_REFRESH_KEY = "pg_applog_auto_refresh";

const levelFilters = [
  { id: "all", label: "全部", match: null },
  { id: "error", label: "ERROR", match: "ERROR" },
  { id: "warn", label: "WARN", match: "WARN" },
  { id: "info", label: "INFO", match: "INFO" },
];

// Fixed dark terminal palette (mirrors a real console; independent of the
// app light/dark theme so the viewer always reads like a terminal).
const TERMINAL = {
  bg: "#0d1117",
  bgHeader: "#161b22",
  border: "#21262d",
  text: "#e6edf3",
  lineNo: "#484f58",
  error: "#f85149",
  warn: "#d29922",
  info: "#58a6ff",
  debug: "#8b949e",
  time: "#7d8590",
  highlight: "#ffa657",
};

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / 1024 ** i).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

// ─── Terminal line rendering ─────────────────────────────────────────────────

/** Extract the leading RFC3339 timestamp so it can be tinted dimly. */
function splitTimestamp(line: string): { head: string; rest: string } {
  const m = line.match(/^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?)\s*/);
  return m ? { head: m[1], rest: line.slice(m[0].length) } : { head: "", rest: line };
}

function levelColor(rest: string): { color: string; label?: string } {
  if (rest.includes("ERROR") || rest.includes("panic")) return { color: TERMINAL.error };
  if (rest.includes("WARN")) return { color: TERMINAL.warn };
  if (rest.includes("DEBUG")) return { color: TERMINAL.debug };
  return { color: TERMINAL.text };
}

/** Split a line into segments, highlighting keywords in an accent colour. */
function highlightKeywords(text: string): React.ReactNode {
  const parts = text.split(/(request_id=\S+|model=\S+|status_code=\d+|\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b)/g);
  return parts.map((part, i) => {
    if (!part) return null;
    const isKeyword =
      part.startsWith("request_id=") ||
      part.startsWith("model=") ||
      part.startsWith("status_code=") ||
      /^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(part);
    return (
      <span key={i} style={isKeyword ? { color: TERMINAL.highlight, fontWeight: 600 } : undefined}>
        {part}
      </span>
    );
  });
}

function TerminalLine({ line, index, total, page }: {
  line: string;
  index: number;
  total: number;
  page: number;
}) {
  const { head, rest } = splitTimestamp(line);
  const { color } = levelColor(rest);
  const lineNo = total - (page - 1) * 50 - index;
  return (
    <div className="pg-term-line">
      <span className="pg-term-no">{lineNo}</span>
      {head && <span className="pg-term-time">{head} </span>}
      <span style={{ color }}>{highlightKeywords(rest)}</span>
    </div>
  );
}

// ─── Component ───────────────────────────────────────────────────────────────

export default function AppLogs() {
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [level, setLevel] = useState("all");
  const [autoInterval, setAutoInterval] = useState<number>(() => {
    const saved = localStorage.getItem(AUTO_REFRESH_KEY);
    const parsed = saved ? Number(saved) : 0;
    return refreshIntervals.some((item) => item.value === parsed) ? parsed : 0;
  });
  // "Follow the tail" behaves like `tail -f`: auto-scrolls to the newest line.
  // Scrolling up pauses the follow; the jump-down button resumes it.
  const [followTail, setFollowTail] = useState(true);
  const [pausedOnce, setPausedOnce] = useState(false);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(search), 300);
    return () => clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    localStorage.setItem(AUTO_REFRESH_KEY, String(autoInterval));
  }, [autoInterval]);

  const keyword = debouncedSearch || undefined;

  const { data: logs, isLoading, refetch } = useAppLogs(
    { page, page_size: 50, keyword },
    autoInterval > 0 ? autoInterval * 1000 : false,
  );

  const { data: logInfo } = useAppLogInfo();

  const filteredLines = useMemo(() => {
    if (!logs?.lines) return [];
    const match = levelFilters.find((f) => f.id === level)?.match;
    if (!match) return logs.lines;
    return logs.lines.filter((line) => line.includes(match));
  }, [logs?.lines, level]);

  const hasPrev = (logs?.page ?? 1) > 1;
  const hasNext = filteredLines.length === 50; // rough "has more"

  // Scroll to the bottom when new lines arrive while following the tail.
  useEffect(() => {
    if (!followTail || !scrollRef.current || !logs?.lines?.length) return;
    const el = scrollRef.current;
    requestAnimationFrame(() => { el.scrollTop = el.scrollHeight; });
  }, [filteredLines, followTail, logs?.lines?.length]);

  // Detect manual scrolling away from the bottom → pause follow.
  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (!atBottom && followTail) {
      setFollowTail(false);
      setPausedOnce(true);
    } else if (atBottom && !followTail) {
      setFollowTail(true);
    }
  };

  const jumpToBottom = () => {
    setFollowTail(true);
    setPausedOnce(false);
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  };

  return (
    <div className="space-y-4 animate-fade-in pg-page">
      {/* Header */}
      <div className="pg-page-header flex items-center justify-between">
        <div>
          <div className="pg-eyebrow mb-1">Application Log</div>
          <h2 style={{ color: "var(--text-primary)" }}>程序日志</h2>
          <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>
            <Terminal size={10} className="inline -mt-px mr-0.5" />
            {logInfo?.file_path
              ? `${logInfo.file_path} · ${formatBytes(logInfo.file_size)} · ${logInfo.line_count.toLocaleString()} 行`
              : "日志文件未就绪"}
          </p>
        </div>
        <Button
          size="sm"
          variant="outline"
          title="复制日志目录路径"
          onClick={() => {
            if (logInfo?.log_dir) navigator.clipboard?.writeText(logInfo.log_dir);
          }}
        >
          <Download size={14} /> 导出
        </Button>
      </div>

      {/* Toolbar */}
      <div className="flex items-center gap-3 flex-wrap">
        {/* Level filter */}
        <div className="flex items-center gap-1 p-0.5 rounded-md" style={{ backgroundColor: "var(--bg-elevated)" }}>
          {levelFilters.map((f) => (
            <button
              key={f.id}
              onClick={() => { setLevel(f.id); setPage(1); }}
              className={`px-3 py-1.5 text-xs rounded transition-all duration-150 cursor-pointer ${
                level === f.id
                  ? "bg-[var(--color-brand)] text-white"
                  : "text-[var(--text-dim)] hover:text-[var(--text-secondary)]"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>

        {/* Search */}
        <div className="relative flex-1 min-w-[180px] max-w-xs">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: "var(--text-dim)" }} />
          <Input
            className="pl-9"
            placeholder="搜索关键词、request_id..."
            value={search}
            onChange={(e) => { setSearch(e.target.value); setPage(1); }}
          />
        </div>

        {/* Refresh controls */}
        <div className="flex items-center gap-1.5 shrink-0 ml-auto">
          <div
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md border"
            style={{ borderColor: "var(--border-default)", backgroundColor: "var(--bg-elevated)" }}
          >
            <span
              className="w-1.5 h-1.5 rounded-full"
              style={{
                backgroundColor: autoInterval > 0 ? "var(--color-ok)" : "var(--text-dim)",
                boxShadow: autoInterval > 0 ? "0 0 6px var(--color-ok)" : "none",
              }}
            />
            <span className="text-[11px] whitespace-nowrap" style={{ color: "var(--text-dim)" }}>自动刷新</span>
            <select
              value={autoInterval}
              onChange={(e) => setAutoInterval(Number(e.target.value))}
              className="bg-transparent text-[11px] font-medium outline-none cursor-pointer"
              style={{ color: "var(--text-primary)" }}
              title="设置自动刷新间隔"
            >
              {refreshIntervals.map((item) => (
                <option key={item.value} value={item.value}>{item.label}</option>
              ))}
            </select>
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={isLoading}
            onClick={() => void refetch()}
            title="刷新日志"
          >
            <RefreshCw size={13} className={isLoading ? "animate-spin" : ""} /> 刷新
          </Button>
        </div>
      </div>

      {/* Terminal viewer */}
      <div className="rounded-lg overflow-hidden border" style={{ borderColor: TERMINAL.border }}>
        {/* Terminal window chrome */}
        <div
          className="flex items-center gap-2 px-3.5 py-2 select-none"
          style={{ backgroundColor: TERMINAL.bgHeader, borderBottom: `1px solid ${TERMINAL.border}` }}
        >
          <span className="w-3 h-3 rounded-full" style={{ backgroundColor: "#ff5f56" }} />
          <span className="w-3 h-3 rounded-full" style={{ backgroundColor: "#ffbd2e" }} />
          <span className="w-3 h-3 rounded-full" style={{ backgroundColor: "#27c93f" }} />
          <span className="ml-2 text-[11px] font-mono truncate" style={{ color: TERMINAL.debug }}>
            {logInfo?.file_path ? `poolgate — ${logInfo.file_path.split("/").pop()}` : "poolgate — app.log"}
          </span>
          {autoInterval > 0 && (
            <span className="ml-auto text-[10px] font-mono" style={{ color: "#3fb950" }}>
              ● tail -f {autoInterval}s
            </span>
          )}
          {!followTail && (
            <span className="ml-2 text-[10px] font-mono" style={{ color: TERMINAL.warn }}>
              ▼ 已暂停跟随
            </span>
          )}
        </div>

        {/* Scrollable log body */}
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="relative overflow-x-auto overflow-y-auto"
          style={{ backgroundColor: TERMINAL.bg, maxHeight: "calc(100vh - 300px)", minHeight: 320 }}
        >
          {isLoading && !logs ? (
            <div className="px-4 py-12 text-center font-mono" style={{ color: TERMINAL.debug }}>
              <Spinner /><span className="ml-2 text-sm">加载中...</span>
            </div>
          ) : filteredLines.length === 0 ? (
            <div className="px-4 py-12 text-center font-mono" style={{ color: TERMINAL.debug }}>
              {logs?.total === 0 ? "暂无日志" : "当前筛选条件无匹配行"}
            </div>
          ) : (
            <div className="py-1.5 font-mono text-xs leading-[1.65]">
              {filteredLines.map((line, i) => (
                <TerminalLine
                  key={`${logs?.page}-${i}`}
                  line={line}
                  index={i}
                  total={logs?.total ?? 0}
                  page={logs?.page ?? 1}
                />
              ))}
            </div>
          )}

          {/* Jump-to-bottom pill when the tail is paused */}
          {pausedOnce && !followTail && filteredLines.length > 0 && (
            <button
              onClick={jumpToBottom}
              className="absolute bottom-3 left-1/2 -translate-x-1/2 flex items-center gap-1.5 px-3 py-1.5 rounded-full text-[11px] font-medium shadow-lg transition-transform hover:scale-105 cursor-pointer"
              style={{ backgroundColor: TERMINAL.bgHeader, color: TERMINAL.text, border: `1px solid ${TERMINAL.border}` }}
            >
              <ArrowDownToLine size={12} /> 回到底部
            </button>
          )}
        </div>
      </div>

      {/* Pagination */}
      {(hasPrev || hasNext) && (
        <div className="flex items-center justify-center gap-2">
          <Button size="sm" variant="outline" disabled={!hasPrev} onClick={() => { setPage((p) => Math.max(1, p - 1)); setFollowTail(false); setPausedOnce(true); }}>
            <ChevronLeft size={14} /> 上一页
          </Button>
          <span className="text-sm px-3" style={{ color: "var(--text-dim)" }}>第 {page} 页 · 共 {logs?.total?.toLocaleString() ?? 0} 行</span>
          <Button size="sm" variant="outline" disabled={!hasNext} onClick={() => { setPage((p) => p + 1); setFollowTail(true); }}>
            下一页 <ChevronRight size={14} />
          </Button>
        </div>
      )}
    </div>
  );
}
