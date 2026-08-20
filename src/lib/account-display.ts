import type { QuotaAccountView } from "@/lib/token-monitor-commands";
import type { AccountDisplayMode } from "@/components/ui/AccountDisplay";

/**
 * 额度账号展示工具（托盘 + 桌面端共用）。
 *
 * - providerLabel：provider_id → 真实供应商名（兼容 `provider_*` 网关前缀）；不可读标识
 *   （UUID/十六进制/密钥）统一回退「API 服务商」，不再标题化拼出 "Prov" 之类的乱码；
 * - accountDisplayName：账号展示名——脱密模式只取可读名称（拒绝裸密钥/十六进制串），
 *   完整模式按原样展示 label；
 * - accountIdentity / maskIdentity：身份行——脱密模式掩码邮箱/密钥，完整模式原样展示。
 */

// 额度账号来源（provider_id → 展示名）：OpenAI / Antigravity / Claude Code 等
const PROVIDER_LABELS: Record<string, string> = {
  openai: "OpenAI",
  chatgpt: "ChatGPT",
  codex: "OpenAI Codex",
  gemini: "Gemini",
  claude: "Claude",
  claude_code: "Claude Code",
  antigravity: "Antigravity",
  anthropic: "Anthropic",
  deepseek: "DeepSeek",
  grok: "Grok",
  xai: "xAI",
  openrouter: "OpenRouter",
  minimax: "MiniMax",
  volcengine_ark: "火山方舟",
  workbuddy: "WorkBuddy",
  mimo: "MiMo Code",
  cursor: "Cursor",
  copilot: "GitHub Copilot",
  qwen: "Qwen",
  moonshot: "Moonshot",
  kimi: "Kimi",
  zhipu: "智谱",
  glm: "GLM",
  doubao: "豆包",
  togetherai: "Together AI",
  mistral: "Mistral",
  perplexity: "Perplexity",
  groq: "Groq",
  cerebras: "Cerebras",
  azure_openai: "Azure OpenAI",
  cohere: "Cohere",
  openai_compatible: "OpenAI 兼容",
  gateway: "PoolGate",
  // 网关 services 层的前缀 provider_id（此前会被标题化截断成 "Prov…"）
  provider_openai_codex: "OpenAI Codex",
  provider_google_gemini: "Gemini",
  provider_google_antigravity: "Antigravity",
  provider_github_copilot: "GitHub Copilot",
  provider_xai_grok: "Grok",
  provider: "API 服务商",
  custom: "API 服务商",
};

/** 是否「不可读」标识：裸密钥 / 十六进制串 / UUID / 内部 provider key
 *  （不应作为供应商名或账号名展示）。任意位置的长十六进制词（"prov 3fbd…"）也算。 */
export function isKeyLike(value?: string | null): boolean {
  if (!value) return false;
  const s = value.trim();
  if (!s) return false;
  // 常见密钥前缀：sk- / pk- / ghp_ 等
  if (/^[a-z]{2,4}-[a-z0-9_-]{8,}$/i.test(s)) return true;
  // 十六进制/数字串（≥12 位）
  if (/^[0-9a-f]{12,}$/i.test(s)) return true;
  // UUID
  if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)) return true;
  // 内部 provider key / 任意位置的长十六进制段（"provider_3fbd…"、"prov 3fbd…" 等）
  if (/\b[0-9a-f]{12,}\b/i.test(s)) return true;
  // 网关内部 id：prov_<hex> / acct_<hex> / prov_<uuid>（"prov_3fbd…" 整体是 word 字符，
  // 上面 \b 匹配不到，需显式识别前缀 + 十六进制段）
  if (/^(prov|acct|account|provider|route|client)[_-][0-9a-f]{8,}/i.test(s)) return true;
  return false;
}

/** 是否身份型字符串（邮箱/密钥/十六进制）：这类值不应作为品牌名展示，交给身份行。 */
export function isIdentityLike(value?: string | null): boolean {
  return isKeyLike(value) || /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test((value ?? "").trim());
}

/** 从 label/账号信息中嗅探平台关键词（provider_id 不可读时兜底）：
 *  label 含 "openai/chatgpt/codex" → OpenAI；"anthropic/claude" → Claude；"gemini" → Gemini 等。 */
function platformFromText(text?: string | null): string | null {
  const s = (text ?? "").toLowerCase();
  if (!s) return null;
  const hints: Array<[RegExp, string]> = [
    [/openai|chatgpt|gpt-|codex/i, "OpenAI"],
    [/anthropic|claude/i, "Claude"],
    [/gemini|antigravity|bard/i, "Gemini"],
    [/deepseek/i, "DeepSeek"],
    [/copilot|github/i, "GitHub Copilot"],
    [/cursor/i, "Cursor"],
    [/qwen/i, "Qwen"],
    [/moonshot|kimi/i, "Moonshot"],
    [/grok|xai/i, "Grok"],
    [/openrouter/i, "OpenRouter"],
    [/minimax/i, "MiniMax"],
    [/volcengine|ark/i, "火山方舟"],
    [/workbuddy/i, "WorkBuddy"],
    [/mimo/i, "MiMo Code"],
  ];
  for (const [re, name] of hints) {
    if (re.test(s)) return name;
  }
  return null;
}

/** 账号品牌名（卡片标题）：平台优先——可读的供应商名/provider 映射优先，
 *  否则可读 label（品牌/计划名），身份型 label（邮箱/密钥/内部 ID）直接跳过。 */
export function accountBrandName(account: QuotaAccountView): string {
  const label = account.label?.trim();
  // 1) 网关 providers 表 / provider_id 映射到真实平台（OpenAI Codex / Claude / Gemini …）
  const platform = providerLabel(account.provider_id, account.provider_label);
  if (platform !== "API 服务商") return platform;
  // 2) provider_id 不可读 → 从 label 嗅探平台关键词
  const hint = platformFromText(label ?? account.identity_masked);
  if (hint) return hint;
  // 3) 可读 label（品牌/计划名）
  if (label && !isIdentityLike(label) && !label.startsWith("provider_")) return label;
  // 4) 全部不可读 → 兜底
  return platform;
}

/** 品牌两段式（对齐开源卡片 "CODEX PLUS"）：[主名, 计划]；计划与主名末词重复时从主名剔除。 */
export function accountBrandParts(account: QuotaAccountView): { main: string; plan: string } {
  const name = accountBrandName(account);
  const raw = String(account.plan_name ?? "").trim().replace(/\s+/g, " ");
  let plan = raw;
  if (raw.length > 10) {
    const parts = raw.split(" ");
    plan = parts.length > 1 ? parts[parts.length - 1] : raw.slice(0, 10);
  }
  if (!plan) return { main: name, plan: "" };
  const words = name.split(/\s+/);
  const last = words[words.length - 1]?.toLowerCase();
  if (last && last === plan.toLowerCase()) {
    return { main: words.slice(0, -1).join(" ") || name, plan };
  }
  return { main: name, plan };
}

/** 会员等级胶囊：从 plan_name 中识别 Free/Plus/Pro/Max/Team/Enterprise 等级并映射颜色。
 *  优先级从上到下：Enterprise > Team > Max > Pro > Plus > Free；
 *  \b 词边界保证 "plus" 不会被误判为 "pro"（旧实现中 "ChatGPT Plus" 曾错误显示为橙色 Pro）。 */
export interface AccountTier {
  label: string;
  /** 胶囊底色（文字白色） */
  color: string;
}

const TIER_RULES: Array<{ re: RegExp; label: string; color: string }> = [
  { re: /\b(enterprise)\b/i, label: "Enterprise", color: "#5e5ce6" },
  { re: /\b(team)\b/i, label: "Team", color: "#0fa3b1" },
  { re: /\b(max|ultra|ultimate)\b/i, label: "Max", color: "#ff9f0a" },
  { re: /\b(pro)\b/i, label: "Pro", color: "#bf5af2" },
  { re: /\b(plus|standard|starter)\b/i, label: "Plus", color: "#0a84ff" },
  { re: /\b(free|basic|trial)\b/i, label: "Free", color: "#34c759" },
];

/** 从 plan_name 提取账号等级；无法识别（如纯 "Codex"、空值）返回 null。 */
export function accountTier(planName?: string | null): AccountTier | null {
  const raw = String(planName ?? "").trim();
  if (!raw) return null;
  for (const { re, label, color } of TIER_RULES) {
    if (re.test(raw)) return { label, color };
  }
  return null;
}

export function providerLabel(providerId?: string, providerName?: string | null): string {
  // 网关 providers 表解析出的可读供应商名优先（"OpenAI Codex" / "Google Antigravity" …）
  if (providerName && providerName.trim()) return providerName.trim();
  if (!providerId) return "账号";
  const known = PROVIDER_LABELS[providerId];
  if (known) return known;
  // 未知但带 provider_ 前缀 → 剥前缀再查/再美化
  if (providerId.startsWith("provider_")) {
    const rest = providerId.slice("provider_".length);
    return providerLabel(rest);
  }
  if (isKeyLike(providerId)) return "API 服务商";
  return providerId.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

/** 脱敏身份：邮箱掩码（u***@example.com）、密钥/十六进制只留首尾（3fbd***d95d）。 */
export function maskIdentity(value?: string | null): string {
  const s = (value ?? "").trim();
  if (!s) return "";
  // 邮箱
  const at = s.indexOf("@");
  if (at > 0) {
    const local = s.slice(0, at);
    const domain = s.slice(at);
    if (local.length <= 2) return `${local[0]}***${domain}`;
    return `${local.slice(0, 2)}***${domain}`;
  }
  // 密钥 / 十六进制 / UUID
  if (isKeyLike(s)) {
    if (s.length <= 10) return `${s.slice(0, 3)}***`;
    return `${s.slice(0, 4)}***${s.slice(-4)}`;
  }
  // 其余（如 "@login"、中文标签）原样
  return s;
}

/**
 * 账号展示名（卡片标题）：
 * - 脱密模式：label 可读才用（拒绝裸密钥/十六进制）；否则回退供应商名；
 * - 完整模式：label 原样（无 label 回退供应商名）。
 */
export function accountDisplayName(account: QuotaAccountView, mode: AccountDisplayMode): string {
  const label = account.label?.trim();
  const fallback = providerLabel(account.provider_id, account.provider_label);
  if (!label) return fallback;
  if (mode === "full") return label;
  // 脱密模式：密钥/十六进制式名称不可读 → 直接用供应商名（掩码片段走身份行）
  if (isKeyLike(label)) return fallback;
  return label;
}

/** 身份行（卡片/列表副行）：完整 → 原样展示（网关账号 identity_masked 即完整邮箱）；
 *  脱密 → 一律掩码（identity_masked 可能是完整邮箱，不能直接透出）。 */
export function accountIdentity(account: QuotaAccountView, mode: AccountDisplayMode): string {
  const raw = account.identity_masked?.trim() || account.label?.trim() || "";
  if (!raw) return "";
  if (mode === "full") return raw;
  return maskIdentity(raw);
}

/** 供应商 · 账号名（额度列表行标题），按展示模式处理账号名。 */
export function accountTitle(account: QuotaAccountView, mode: AccountDisplayMode): string {
  const src = providerLabel(account.provider_id, account.provider_label);
  const name = accountDisplayName(account, mode);
  if (!name || name === src) return src;
  return `${src} · ${name}`;
}

// ───────────────────── 预付费余额窗口（DeepSeek 等）展示工具 ─────────────────────

/** 是否为预付费余额窗口：window_type=prepaid_balance / unit=currency / 标签含「余额|balance」。
 *  余额没有窗口上限与重置周期，圆环/进度条/重置行等「百分比窗口」模型不适用，需特殊展示。 */
export function isBalanceWindow(w: { window_type?: string; unit?: string; label?: string }): boolean {
  return (
    w.window_type === "prepaid_balance" ||
    w.unit === "currency" ||
    /(余额|balance)/i.test(w.label || "")
  );
}

/** 从「账户余额（CNY）」标签解析货币代码；解析不到默认 USD。 */
export function currencyOf(w: { label?: string }): string {
  const m = /[（(]\s*([A-Za-z]{3})\s*[)）]/.exec(w.label || "");
  return m ? m[1].toUpperCase() : "USD";
}

/** 货币符号：¥ / $ / € / £；未知代码回退为代码本身。 */
export function currencySymbol(code: string): string {
  switch (code.toUpperCase()) {
    case "CNY":
    case "JPY": return "¥";
    case "USD": return "$";
    case "EUR": return "€";
    case "GBP": return "£";
    default: return `${code} `;
  }
}

/** 余额金额格式化：¥97.60 / $2.05 / EUR 12.00。 */
export function formatBalance(value: number | null | undefined, code: string): string {
  if (value == null) return "—";
  return `${currencySymbol(code)}${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** 余额状态：耗尽红 / 偏低橙 / 充足绿（按货币阈值）；未知灰。 */
export function balanceTone(value: number | null | undefined, code: string): { color: string; label: string } {
  const LOW: Record<string, number> = { CNY: 10, JPY: 500, USD: 2, EUR: 2, GBP: 2 };
  if (value == null) return { color: "#9a9da3", label: "余额未知" };
  if (value <= 0) return { color: "#ff453a", label: "已耗尽" };
  if (value < (LOW[code.toUpperCase()] ?? 2)) return { color: "#ff9500", label: "偏低" };
  return { color: "#34c759", label: "充足" };
}
