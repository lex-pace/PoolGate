/**
 * Provider brand identity — colors + monogram glyphs for the provider picker.
 *
 * cc-switch renders every vendor as a small colored icon tile plus a name.
 * We don't ship vendor logos (licensing + bundle size), so each provider gets
 * a deterministic brand color and a 1–2 char monogram rendered on a rounded
 * tile. Colors are picked to be recognizable (brand-adjacent) yet readable on
 * both light and dark surfaces.
 */

export interface ProviderBrand {
  /** Solid tile background color. */
  color: string;
  /** Monogram shown on the tile (1–2 glyphs). */
  mono: string;
  /** Optional foreground override; defaults to white. */
  fg?: string;
}

/** Per-provider brand overrides keyed by template id. */
const brandMap: Record<string, ProviderBrand> = {
  // 官方直连
  openai: { color: "#10a37f", mono: "AI" },
  anthropic: { color: "#d97757", mono: "Cl" },
  codex: { color: "#111111", mono: "Cx" },
  "gemini-native": { color: "#1a73e8", mono: "G" },
  "github-copilot": { color: "#1f2328", mono: "Co" },
  "xai-grok": { color: "#111111", mono: "xA" },
  // 国内官方
  deepseek: { color: "#4d6bfe", mono: "DS" },
  kimi: { color: "#16182f", mono: "K" },
  "kimi-for-coding": { color: "#16182f", mono: "KC" },
  "zhipu-glm": { color: "#3859ff", mono: "GLM" },
  "zhipu-glm-en": { color: "#3859ff", mono: "GLM" },
  bailian: { color: "#615ced", mono: "百" },
  "bailian-for-coding": { color: "#615ced", mono: "百" },
  "baidu-qianfan": { color: "#2932e1", mono: "百" },
  "volcengine-agentplan": { color: "#00e5e0", mono: "火", fg: "#0b3a3a" },
  "doubao-seed": { color: "#3b7cff", mono: "豆" },
  byteplus: { color: "#00e5e0", mono: "BP", fg: "#0b3a3a" },
  minimax: { color: "#f23b56", mono: "M" },
  "minimax-en": { color: "#f23b56", mono: "M" },
  stepfun: { color: "#0a5fff", mono: "Step" },
  "stepfun-en": { color: "#0a5fff", mono: "Step" },
  longcat: { color: "#ff8a3d", mono: "L" },
  "kat-coder": { color: "#111111", mono: "KAT" },
  bailing: { color: "#1677ff", mono: "百" },
  "xiaomi-mimo": { color: "#ff6900", mono: "Mi" },
  "xiaomi-mimo-token": { color: "#ff6900", mono: "Mi" },
  // 聚合与中转
  openrouter: { color: "#6467f2", mono: "OR" },
  siliconflow: { color: "#6d5efc", mono: "SF" },
  "siliconflow-en": { color: "#6d5efc", mono: "SF" },
  modelscope: { color: "#624aff", mono: "MS" },
  aihubmix: { color: "#7c5cff", mono: "Hub" },
  dmxapi: { color: "#ff7a45", mono: "DMX" },
  zetaapi: { color: "#5b8def", mono: "Z" },
  fennoai: { color: "#3aa0ff", mono: "Fn" },
  runapi: { color: "#f0453a", mono: "Run" },
  unity2: { color: "#111827", mono: "U2" },
  shengsuanyun: { color: "#7b61ff", mono: "胜" },
  subrouter: { color: "#22c55e", mono: "SR" },
  "claudeapi-apito": { color: "#d97757", mono: "API" },
  code0: { color: "#16a34a", mono: "C0" },
  teamorouter: { color: "#0ea5e9", mono: "TR" },
  nekocode: { color: "#f472b6", mono: "Nk" },
  a6api: { color: "#f59e0b", mono: "A6" },
  atlascloud: { color: "#2563eb", mono: "At" },
  compshare: { color: "#7c3aed", mono: "优" },
  "compshare-coding": { color: "#7c3aed", mono: "优" },
  ccsub: { color: "#111827", mono: "CC" },
  qiniu: { color: "#0aa1ff", mono: "七" },
  amux: { color: "#111827", mono: "Ax" },
  cherryin: { color: "#e0405a", mono: "IN" },
  therouter: { color: "#6366f1", mono: "TR" },
  novita: { color: "#111827", mono: "Nv" },
  nvidia: { color: "#76b900", mono: "Nv" },
  pipellm: { color: "#111827", mono: "PL" },
  // 第三方服务
  packycode: { color: "#7c5cff", mono: "Pk" },
  apinebula: { color: "#3aa0ff", mono: "Nb" },
  aicodemirror: { color: "#f43f5e", mono: "Mr" },
  patewayai: { color: "#0ea5e9", mono: "Pt" },
  aigocode: { color: "#22c55e", mono: "Go" },
  aicoding: { color: "#6366f1", mono: "AC" },
  "apikey-fun": { color: "#f59e0b", mono: "AK" },
  claudecn: { color: "#d97757", mono: "CN" },
  sssaicode: { color: "#64748b", mono: "SSS" },
  micu: { color: "#3b82f6", mono: "Mi" },
  rightcode: { color: "#10b981", mono: "R" },
  etok: { color: "#f97316", mono: "ET" },
  cubence: { color: "#6366f1", mono: "Cu" },
  crazyrouter: { color: "#8b5cf6", mono: "CR" },
  "sudocode-chat": { color: "#111827", mono: "Su" },
  "sudocode-us": { color: "#f472b6", mono: "Su" },
  "opencode-go": { color: "#334155", mono: "Go" },
  relaxycode: { color: "#14b8a6", mono: "Rx" },
  "e-flowcode": { color: "#22c55e", mono: "EF" },
  // 云服务商
  "aws-bedrock-aksk": { color: "#ff9900", mono: "AWS", fg: "#111" },
  "aws-bedrock-apikey": { color: "#ff9900", mono: "AWS", fg: "#111" },
  // 自定义
  custom: { color: "#5b6472", mono: "+" },
};

/** Deterministic fallback palette for providers without an explicit brand. */
const fallbackPalette = [
  "#0a84ff", "#5e5ce6", "#bf5af2", "#ff375f", "#ff9f0a",
  "#30d158", "#64d2ff", "#ac8e68", "#ff6482", "#5ac8fa",
];

function hashString(input: string): number {
  let h = 0;
  for (let i = 0; i < input.length; i++) {
    h = (h * 31 + input.charCodeAt(i)) >>> 0;
  }
  return h;
}

/** Derive a 1–2 char monogram from a provider name. */
function deriveMono(name: string): string {
  const cleaned = name.replace(/[^A-Za-z0-9\u4e00-\u9fa5]/g, " ").trim();
  if (!cleaned) return "?";
  // Chinese: take first character.
  if (/[\u4e00-\u9fa5]/.test(cleaned[0])) return cleaned[0];
  const parts = cleaned.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }
  return cleaned.slice(0, 2).toUpperCase();
}

/** Resolve the brand identity for a provider template. */
export function getProviderBrand(id: string, name: string): ProviderBrand {
  const explicit = brandMap[id];
  if (explicit) return explicit;
  const color = fallbackPalette[hashString(id || name) % fallbackPalette.length];
  return { color, mono: deriveMono(name) };
}
