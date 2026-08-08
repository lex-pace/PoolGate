import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatNumber(num: number): string {
  if (num >= 1_000_000) return (num / 1_000_000).toFixed(1) + "M";
  if (num >= 1_000) return (num / 1_000).toFixed(1) + "K";
  return num.toLocaleString();
}

export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return (tokens / 1_000_000).toFixed(2) + "M";
  if (tokens >= 1_000) return (tokens / 1_000).toFixed(1) + "K";
  return tokens.toString();
}

/** Chinese unit formatting (万 / 亿) for token counters, e.g. 6993.1万. */
export function formatTokensZh(tokens: number): string {
  const amount = Math.max(0, tokens || 0);
  if (amount >= 100_000_000) return `${(amount / 100_000_000).toFixed(amount >= 1_000_000_000 ? 1 : 2)}亿`;
  if (amount >= 10_000) return `${(amount / 10_000).toFixed(amount >= 1_000_000 ? 0 : 1)}万`;
  return amount.toLocaleString("zh-CN");
}

export function formatCost(cost: number): string {
  if (cost === 0) return "--";
  if (cost < 0.01) return "<$0.01";
  return "$" + cost.toFixed(2);
}

export function formatLatency(ms: number): string {
  if (ms >= 1000) return (ms / 1000).toFixed(1) + "s";
  return ms + "ms";
}

export function formatTime(isoString: string): string {
  const d = new Date(isoString);
  return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatDate(isoString: string): string {
  const d = new Date(isoString);
  return d.toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" });
}

export function maskApiKey(key: string): string {
  if (key.length <= 8) return key;
  return key.slice(0, 7) + "..." + key.slice(-4);
}

export function getTimeAgo(isoString: string | null | undefined): string {
  if (!isoString) return "从未";
  const now = Date.now();
  const then = new Date(isoString).getTime();
  const diffMs = now - then;
  const diffMin = Math.floor(diffMs / 60000);
  if (diffMin < 1) return "刚刚";
  if (diffMin < 60) return `${diffMin}分钟前`;
  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return `${diffHour}小时前`;
  const diffDay = Math.floor(diffHour / 24);
  return `${diffDay}天前`;
}
