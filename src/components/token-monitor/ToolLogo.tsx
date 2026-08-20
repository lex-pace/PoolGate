import { AppWindow } from "lucide-react";
import freebuffIcon from "@/assets/logos/freebuff.png";
import claudeCode from "@/assets/logos/claude_code.svg?raw";
import codex from "@/assets/logos/codex.svg?raw";
import opencode from "@/assets/logos/opencode.svg?raw";
import cursor from "@/assets/logos/cursor.svg?raw";
import githubCopilot from "@/assets/logos/github_copilot.svg?raw";
import workbuddy from "@/assets/logos/workbuddy.svg?raw";
import mimo from "@/assets/logos/mimo.svg?raw";
import zcode from "@/assets/logos/zcode.svg?raw";
import codebuddy from "@/assets/logos/codebuddy.svg?raw";
import antigravity from "@/assets/logos/antigravity.svg?raw";
import kimi from "@/assets/logos/kimi.svg?raw";
import qwen from "@/assets/logos/qwen.svg?raw";
import grokBuild from "@/assets/logos/grok_build.svg?raw";
import hermes from "@/assets/logos/hermes.svg?raw";
import zed from "@/assets/logos/zed.svg?raw";
import kiro from "@/assets/logos/kiro.svg?raw";
import cline from "@/assets/logos/cline.svg?raw";
import kiloCode from "@/assets/logos/kilo_code.svg?raw";
import pi from "@/assets/logos/pi.svg?raw";
import proma from "@/assets/logos/proma.svg?raw";
import openclaw from "@/assets/logos/openclaw.svg?raw";
import gemini from "@/assets/logos/gemini.svg?raw";
import deepseek from "@/assets/logos/deepseek.svg?raw";
import minimax from "@/assets/logos/minimax.svg?raw";
import openrouter from "@/assets/logos/openrouter.svg?raw";
import ollama from "@/assets/logos/ollama.svg?raw";
import volcengine from "@/assets/logos/volcengine.svg?raw";
import newapi from "@/assets/logos/newapi.svg?raw";
import meta from "@/assets/logos/meta.svg?raw";
import mistral from "@/assets/logos/mistral.svg?raw";
import cohere from "@/assets/logos/cohere.svg?raw";
import moonshot from "@/assets/logos/moonshot.svg?raw";
import doubao from "@/assets/logos/doubao.svg?raw";
import hunyuan from "@/assets/logos/hunyuan.svg?raw";
import xiaomi from "@/assets/logos/xiaomi.svg?raw";

/** 官方品牌 SVG（fill="currentColor"）+ 品牌色。与开源 Token Monitor 同方案：图标打包为本地资源、内联渲染、vendor 色上色。 */
const LOGOS = {
  claude_code: { svg: claudeCode, color: "#D97757" },
  codex: { svg: codex, color: "#10A37F" },
  opencode: { svg: opencode, color: "#3B82F6" },
  cursor: { svg: cursor, color: "#E8E8ED" },
  github_copilot: { svg: githubCopilot, color: "#8957E5" },
  workbuddy: { svg: workbuddy, color: "#FF6B35" },
  mimo: { svg: mimo, color: "#E85D75" },
  zcode: { svg: zcode, color: "#7C5CFF" },
  codebuddy: { svg: codebuddy, color: "#F59E0B" },
  antigravity: { svg: antigravity, color: "#14B8A6" },
  kimi: { svg: kimi, color: "#3B5BDB" },
  qwen: { svg: qwen, color: "#7C5CFF" },
  grok_build: { svg: grokBuild, color: "#F5F5F7" },
  hermes: { svg: hermes, color: "#A855F7" },
  zed: { svg: zed, color: "#1E5BFF" },
  kiro: { svg: kiro, color: "#EC4899" },
  cline: { svg: cline, color: "#6D4AFF" },
  kilo_code: { svg: kiloCode, color: "#2DD4BF" },
  pi: { svg: pi, color: "#EF4444" },
  proma: { svg: proma, color: "#22C55E" },
  openclaw: { svg: openclaw, color: "#F97316" },
  gemini: { svg: gemini, color: "#4285F4" },
  // ── 供应商（额度卡/状态条/模型行）──
  deepseek: { svg: deepseek, color: "#4D6BFE" },
  minimax: { svg: minimax, color: "#0E9F6E" },
  openrouter: { svg: openrouter, color: "#8488F5" },
  ollama: { svg: ollama, color: "#D4D4D8" },
  volcengine: { svg: volcengine, color: "#3370FF" },
  newapi: { svg: newapi, color: "#3B82F6" },
  meta: { svg: meta, color: "#0866FF" },
  mistral: { svg: mistral, color: "#F97316" },
  cohere: { svg: cohere, color: "#39594D" },
  moonshot: { svg: moonshot, color: "#3B5BDB" },
  doubao: { svg: doubao, color: "#3370FF" },
  hunyuan: { svg: hunyuan, color: "#1D6BF3" },
  xiaomi: { svg: xiaomi, color: "#FF6900" },
  // Freebuff 本体：官方应用图标（PNG，非 currentColor SVG）
  freebuff: { img: freebuffIcon },
} as const;

/** 供应商 id / 别名 → LOGOS key（额度账号 provider_id、品牌显示名、模型名前缀统一走这里）。 */
const PROVIDER_ALIAS: Record<string, string> = {
  claude: "claude_code", anthropic: "claude_code",
  openai: "codex", chatgpt: "codex", gpt: "codex", codex: "codex", oai: "codex",
  deepseek: "deepseek",
  gemini: "gemini", google: "gemini",
  qwen: "qwen", tongyi: "qwen", alibaba: "qwen",
  kimi: "kimi", moonshot: "moonshot",
  xai: "grok_build", grok: "grok_build",
  zai: "zcode", glm: "zcode", zhipu: "zcode", zcode: "zcode",
  doubao: "doubao", volcengine: "volcengine",
  minimax: "minimax",
  openrouter: "openrouter",
  ollama: "ollama",
  mistral: "mistral", codestral: "mistral",
  cohere: "cohere", command: "cohere",
  meta: "meta", llama: "meta",
  xiaomi: "xiaomi",
  hunyuan: "hunyuan", tencent: "hunyuan",
  newapi: "newapi", new_api: "newapi",
  freebuff: "freebuff",
  cursor: "cursor",
  copilot: "github_copilot", github_copilot: "github_copilot",
  workbuddy: "workbuddy",
  opencode: "opencode",
};

/** 模型名 → 供应商 id（模型行 Logo 用）。 */
export function providerForModel(model: string): string | null {
  const m = String(model || "").toLowerCase();
  if (!m) return null;
  if (m.startsWith("claude")) return "claude";
  if (/^(gpt|o1|o3|o4|chatgpt)/.test(m)) return "openai";
  if (m.includes("gemini")) return "gemini";
  if (m.includes("deepseek")) return "deepseek";
  if (m.includes("qwen")) return "qwen";
  if (m.includes("kimi")) return "kimi";
  if (m.includes("moonshot")) return "moonshot";
  if (m.includes("grok")) return "xai";
  if (/^(glm|zhipu)/.test(m) || m.includes("z-code")) return "zai";
  if (m.includes("doubao") || m.includes("seed")) return "doubao";
  if (m.includes("minimax")) return "minimax";
  if (/^(mistral|codestral|pixtral)/.test(m)) return "mistral";
  if (m.includes("llama") || m.includes("meta")) return "meta";
  if (m.includes("command") || m.includes("cohere")) return "cohere";
  if (m.includes("hunyuan")) return "hunyuan";
  if (m.includes("xiaomi")) return "xiaomi";
  return null;
}

/** 供应商显示名（如 "Claude"、"OpenAI"）→ 品牌 Logo key；未知返回 null。 */
export function logoKeyForBrand(brand: string): string | null {
  const s = String(brand || "").toLowerCase().replace(/[^a-z0-9_]/g, "");
  if (!s) return null;
  // 逐段最长匹配（如 "chatgpt-openai" → openai）
  const keys = Object.keys(PROVIDER_ALIAS).sort((a, b) => b.length - a.length);
  for (const key of keys) {
    if (s.includes(key)) return PROVIDER_ALIAS[key];
  }
  return null;
}

/** 供应商 id → 品牌 Logo key。 */
export function logoKeyForProvider(providerId: string): string | null {
  const s = String(providerId || "").toLowerCase();
  return PROVIDER_ALIAS[s] ?? null;
}

function BrandGlyph({ logoKey, size, className = "" }: { logoKey: string; size: number; className?: string }) {
  const brand = LOGOS[logoKey as keyof typeof LOGOS];
  if (!brand) return null;
  if ("img" in brand) {
    // PNG 位图（如 Freebuff 官方应用图标）：直接渲染，不做品牌色覆盖
    return (
      <span
        className={`pg-tool-logo ${className}`}
        style={{ width: size, height: size }}
        aria-hidden
      >
        <img src={brand.img} alt="" style={{ width: "100%", height: "100%", objectFit: "contain" }} />
      </span>
    );
  }
  return (
    <span
      className={`pg-tool-logo ${className}`}
      style={{ width: size, height: size, color: brand.color, fontSize: Math.round(size * 0.62) }}
      aria-hidden
    >
      <span dangerouslySetInnerHTML={{ __html: brand.svg }} />
    </span>
  );
}

/** 工具 Logo：tool_id → 官方品牌 SVG（fill="currentColor" + 品牌色）。 */
export default function ToolLogo({
  toolId,
  displayName,
  size = 36,
  className = "",
}: {
  toolId: string;
  displayName?: string;
  size?: number;
  className?: string;
}) {
  const logoKey = logoKeyForProvider(toolId) ?? logoKeyForBrand(toolId);
  if (logoKey) {
    return <BrandGlyph logoKey={logoKey} size={size} className={className} />;
  }
  // 自定义应用/未知：中性「应用」图标（诚实回退，不伪造品牌）
  return (
    <span
      className={`pg-tool-logo pg-tool-logo-generic ${className}`}
      style={{ width: size, height: size }}
      title={displayName}
      aria-hidden
    >
      <AppWindow size={Math.round(size * 0.52)} strokeWidth={1.8} />
    </span>
  );
}

/** 供应商 Logo：provider_id / 品牌显示名 / 模型名 → 官方品牌 SVG。 */
export function ProviderLogo({
  provider,
  brand,
  model,
  size = 18,
  className = "",
}: {
  /** 供应商 id（如 "claude"、"openai"、"deepseek"） */
  provider?: string;
  /** 品牌显示名（如 "Claude"、"OpenAI"） */
  brand?: string;
  /** 模型名（如 "claude-3.5-sonnet"）→ 自动映射到供应商 */
  model?: string;
  size?: number;
  className?: string;
}) {
  const logoKey =
    (provider ? logoKeyForProvider(provider) : null) ??
    (brand ? logoKeyForBrand(brand) : null) ??
    (model ? (() => { const p = providerForModel(model); return p ? logoKeyForProvider(p) : null; })() : null);
  if (logoKey) {
    return <BrandGlyph logoKey={logoKey} size={size} className={className} />;
  }
  return null;
}
