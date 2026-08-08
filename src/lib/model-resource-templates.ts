/**
 * Model resource templates.
 *
 * A "model resource" is an upstream provider that exposes an
 * OpenAI/Anthropic/Gemini-compatible endpoint and can be added via API Key,
 * OAuth or Token. Importing accounts from other tools (Sub2API, CPA, Cockpit,
 * Codex auth.json, CSV, …) is handled by the Token & JSON / batch import flows.
 *
 * The provider catalog mirrors cc-switch (https://github.com/farion1231/cc-switch):
 * every built-in Claude Code / Coding Plan endpoint is available here, grouped
 * by category. A single provider may speak multiple protocols — `protocols`
 * is an array; the first entry is the primary protocol.
 */

export type AuthMethod = "oauth" | "token" | "apikey" | "batch";

/** Product-facing category used by the model-resource page and import flow. */
export type ModelResourceCategory =
  | "coding_plan"
  | "api_key"
  | "free"
  | "gateway"
  | "custom";

export const modelResourceCategoryLabels: Record<ModelResourceCategory, string> = {
  coding_plan: "Coding Plan",
  api_key: "模型 API",
  free: "免费模型",
  gateway: "聚合网关",
  custom: "自定义",
};

/** Protocol spoken by the upstream. Canonical set: responses / chat / anthropic / gemini. */
export type Protocol = "responses" | "chat" | "anthropic" | "gemini";

/** Logical grouping for the provider picker. Mirrors cc-switch categories. */
export type ProviderGroup =
  | "official"
  | "cn_official"
  | "aggregator"
  | "third_party"
  | "cloud"
  | "custom";

export interface ModelResourceTemplate {
  id: string;
  name: string;
  group: ProviderGroup;
  /** Protocols spoken by this upstream, in priority order. */
  protocols: Protocol[];
  /** Legacy/default Base URL. Used when a protocol-specific URL is not configured. */
  baseUrl: string;
  /** Protocol-specific Base URLs for providers whose OpenAI/Anthropic endpoints differ. */
  baseUrls?: Partial<Record<Protocol, string>>;
  /** Optional model-list discovery override when it differs from the selected protocol endpoint. */
  modelDiscovery?: {
    url: string;
    protocol?: Protocol;
  };
  models: string[];
  description: string;
  recommended?: boolean;
  /** Vendor offers a Coding Plan OAuth/adapter access path. */
  codingPlan?: {
    oauthUrl?: string;
    baseUrl?: string;
    models?: string[];
  };
  /** Real OAuth adapter configuration. Omitted when Token/JSON is the only login path. */
  oauth?: {
    adapter: "codex";
    authorizationEndpoint: string;
    tokenEndpoint: string;
    redirectUri: string;
    scopes: string[];
    pkce: true;
    quotaAdapter?: "codex";
  };
  /** Which import methods this provider currently supports in PoolGate. */
  authMethods: AuthMethod[];
  /** Optional explicit product category. Otherwise inferred from template metadata. */
  resourceCategory?: ModelResourceCategory;
  /** Whether the provider is marketed with free models or a reusable free tier. */
  freeModels?: boolean;
  /** Whether this provider allows custom Base URL override. */
  allowCustomBaseUrl?: boolean;
  /** Vendor website, for reference. */
  website?: string;
}

export const providerGroupLabels: Record<ProviderGroup, string> = {
  official: "官方直连",
  cn_official: "国内官方",
  aggregator: "聚合与中转",
  third_party: "第三方服务",
  cloud: "云服务商",
  custom: "自定义上游",
};

/**
 * Import formats recognised by the Rust backend `detect_format`.
 * Surfaced in the Token & JSON / batch import UI as "auto-detect" hints.
 */
export const importFormatLabels: Record<string, string> = {
  sub2api: "Sub2API",
  cpa: "CPA / CLIProxyAPI",
  cockpit: "Cockpit",
  codex_auth: "Codex auth.json",
  json: "通用 JSON",
  csv: "CSV",
  api_key_text: "API Key 文本",
};

export const supportedImportFormats = [
  "sub2api",
  "cpa",
  "cockpit",
  "codex_auth",
  "json",
  "csv",
  "api_key_text",
] as const;

/** Convenience: the first/primary protocol of a template. */
export function primaryProtocol(template: ModelResourceTemplate): Protocol {
  return template.protocols[0];
}

/** Resolve the default Base URL for one concrete protocol connection. */
export function baseUrlForProtocol(
  template: ModelResourceTemplate,
  protocol: Protocol | undefined,
): string {
  if (protocol) {
    const protocolUrl = template.baseUrls?.[protocol];
    if (protocolUrl) return protocolUrl;
  }
  return template.baseUrl;
}

export const protocolOptions = [
  { value: "responses" as Protocol, label: "Responses（直连）" },
  { value: "chat" as Protocol, label: "Chat Completions" },
  { value: "anthropic" as Protocol, label: "Anthropic Messages" },
  { value: "gemini" as Protocol, label: "Gemini（直连）" },
];

/**
 * Normalize a user-supplied Base URL to the canonical mount-point form.
 *
 * Mirror of the Rust `normalize_base_url`. The gateway owns the API version
 * segment (`/v1`, `/v1beta`) and the endpoint path (`/chat/completions`,
 * `/messages`, `/models`); the stored Base URL must be the origin plus any
 * provider-specific mount prefix (e.g. `/anthropic`). Pasting a complete
 * endpoint URL is therefore reduced to the same upstream host that routing
 * and health checks target.
 */
export function normalizeBaseUrl(protocol: Protocol | undefined, raw: string): string {
  let url = (raw || "").trim();
  if (!url) return url;
  while (url.endsWith("/")) url = url.slice(0, -1);

  const tails: string[] =
    protocol === "anthropic"
      ? ["/v1/messages", "/messages", "/v1"]
      : protocol === "gemini"
        ? ["/v1beta/models", "/v1beta"]
        : protocol === "chat" || protocol === "responses"
          ? ["/v1/chat/completions", "/chat/completions", "/v1/models", "/v1", "/models"]
          : [
              "/v1/chat/completions",
              "/chat/completions",
              "/v1/messages",
              "/messages",
              "/v1beta/models",
              "/v1/models",
              "/models",
              "/v1",
              "/v1beta",
            ];

  for (const tail of tails) {
    if (url.endsWith(tail)) {
      url = url.slice(0, url.length - tail.length);
      break;
    }
  }
  return url;
}

/** Display string for a template's protocols, e.g. "openai · anthropic". */
export function protocolsLabel(template: ModelResourceTemplate): string {
  return template.protocols.join(" · ");
}

/** Infer the user-facing category for a built-in template. */
export function resourceCategoryForTemplate(
  template: ModelResourceTemplate,
  method?: AuthMethod,
): ModelResourceCategory {
  if (template.resourceCategory) return template.resourceCategory;
  if (template.group === "custom") return "custom";
  const codingSignature = `${template.id} ${template.name}`.toLowerCase();
  const isDedicatedCodingPlan = /codex|coding|agentplan|token-plan/.test(codingSignature);
  if (method === "oauth" || (template.codingPlan && method === "token") || isDedicatedCodingPlan) return "coding_plan";
  if (template.freeModels) return "free";
  if (template.group === "aggregator" || template.group === "third_party") return "gateway";
  return "api_key";
}

/** Stable tag persisted with imported accounts so filters do not rely on names. */
export function resourceCategoryTag(category: ModelResourceCategory): string {
  return `resource_category:${category}`;
}

export const modelResourceTemplates: ModelResourceTemplate[] = [
  // ── 官方直连 ─────────────────────────────────────────────
  {
    id: "anthropic",
    name: "Claude Official",
    group: "official",
    protocols: ["anthropic"],
    baseUrl: "https://api.anthropic.com",
    models: ["claude-sonnet-4", "claude-opus-4"],
    description: "Anthropic 官方 API 与 Claude Code 登录",
    recommended: true,
    authMethods: ["apikey", "token", "batch"],
    website: "https://www.anthropic.com/claude-code",
    codingPlan: { oauthUrl: "https://console.anthropic.com/oauth/authorize", models: ["claude-sonnet", "claude-opus"] },
  },
  {
    id: "openai",
    name: "OpenAI",
    group: "official",
    protocols: ["chat", "responses"],
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-5", "gpt-4.1", "o3"],
    description: "OpenAI 官方 API 与 Codex Coding Plan",
    recommended: true,
    oauth: {
      adapter: "codex",
      authorizationEndpoint: "https://auth.openai.com/oauth/authorize",
      tokenEndpoint: "https://auth.openai.com/oauth/token",
      redirectUri: "http://localhost:1455/auth/callback",
      scopes: ["openid", "profile", "email", "offline_access"],
      pkce: true,
      quotaAdapter: "codex",
    },
    authMethods: ["apikey", "oauth", "token", "batch"],
    website: "https://openai.com",
    codingPlan: { baseUrl: "https://chatgpt.com/backend-api/codex", models: ["codex", "gpt-5-codex"] },
  },
  {
    id: "codex",
    name: "Codex",
    group: "official",
    protocols: ["responses", "chat"],
    baseUrl: "https://chatgpt.com/backend-api/codex",
    models: ["codex", "gpt-5-codex"],
    description: "ChatGPT Codex 后端，OpenAI Responses 协议",
    oauth: {
      adapter: "codex",
      authorizationEndpoint: "https://auth.openai.com/oauth/authorize",
      tokenEndpoint: "https://auth.openai.com/oauth/token",
      redirectUri: "http://localhost:1455/auth/callback",
      scopes: ["openid", "profile", "email", "offline_access"],
      pkce: true,
      quotaAdapter: "codex",
    },
    authMethods: ["oauth", "token"],
    website: "https://openai.com/chatgpt/pricing",
  },
  {
    id: "gemini-native",
    name: "Gemini Native",
    group: "official",
    protocols: ["gemini", "chat"],
    baseUrl: "https://generativelanguage.googleapis.com",
    models: ["gemini-2.5-pro", "gemini-2.5-flash"],
    description: "Google AI Studio / Gemini 原生 API",
    authMethods: ["apikey", "token", "batch"],
    website: "https://ai.google.dev/gemini-api",
    codingPlan: { oauthUrl: "https://accounts.google.com/o/oauth2/v2/auth", models: ["gemini-2.5-pro", "gemini-2.5-flash"] },
  },
  {
    id: "github-copilot",
    name: "GitHub Copilot",
    group: "official",
    protocols: ["chat"],
    baseUrl: "https://api.githubcopilot.com",
    models: [],
    description: "GitHub Copilot，OpenAI Chat 协议",
    authMethods: ["token"],
    website: "https://github.com/features/copilot",
  },
  {
    id: "xai-grok",
    name: "xAI (Grok)",
    group: "official",
    protocols: ["chat"],
    baseUrl: "https://api.x.ai/v1",
    models: ["grok-4", "grok-code"],
    description: "xAI Grok，OpenAI Responses 协议",
    authMethods: ["token", "apikey"],
    website: "https://x.ai/grok",
  },

  // ── 国内官方 ─────────────────────────────────────────────
  {
    id: "deepseek",
    name: "DeepSeek",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.deepseek.com/anthropic",
    baseUrls: {
      anthropic: "https://api.deepseek.com/anthropic",
      chat: "https://api.deepseek.com",
    },
    modelDiscovery: {
      url: "https://api.deepseek.com/models",
      protocol: "chat",
    },
    models: ["deepseek-v4-flash", "deepseek-v4-pro"],
    description: "DeepSeek 官方，兼容 Anthropic 与 OpenAI 协议",
    recommended: true,
    authMethods: ["apikey", "batch"],
    allowCustomBaseUrl: true,
    website: "https://platform.deepseek.com",
  },
  {
    id: "kimi",
    name: "Kimi",
    group: "cn_official",
    protocols: ["anthropic"],
    baseUrl: "https://api.moonshot.cn/anthropic",
    models: ["kimi-k2", "moonshot-v1"],
    description: "月之暗面 Kimi，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://platform.kimi.com",
  },
  {
    id: "kimi-for-coding",
    name: "Kimi For Coding",
    group: "cn_official",
    protocols: ["anthropic"],
    baseUrl: "https://api.kimi.com/coding/",
    models: ["kimi-k2"],
    description: "Kimi Coding Plan 专用入口",
    authMethods: ["apikey", "token", "batch"],
    website: "https://www.kimi.com/code/",
    codingPlan: { models: ["kimi-k2"] },
  },
  {
    id: "zhipu-glm",
    name: "Zhipu GLM",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://open.bigmodel.cn/api/anthropic",
    models: ["glm-4.5", "glm-4-air"],
    description: "智谱 BigModel，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://open.bigmodel.cn",
  },
  {
    id: "zhipu-glm-en",
    name: "Zhipu GLM en",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.z.ai/api/anthropic",
    models: ["glm-4.5", "glm-4-air"],
    description: "智谱 z.ai 海外节点",
    authMethods: ["apikey", "batch"],
    website: "https://z.ai",
  },
  {
    id: "bailian",
    name: "Bailian",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://dashscope.aliyuncs.com/apps/anthropic",
    models: ["qwen3-coder", "qwen-max"],
    description: "阿里云百炼 DashScope，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://bailian.console.aliyun.com",
  },
  {
    id: "bailian-for-coding",
    name: "Bailian For Coding",
    group: "cn_official",
    protocols: ["anthropic"],
    baseUrl: "https://coding.dashscope.aliyuncs.com/apps/anthropic",
    models: ["qwen3-coder"],
    description: "百炼 Coding Plan 专用入口",
    authMethods: ["apikey", "token", "batch"],
    website: "https://bailian.console.aliyun.com",
    codingPlan: { models: ["qwen3-coder"] },
  },
  {
    id: "baidu-qianfan",
    name: "Baidu Qianfan Coding Plan",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://qianfan.baidubce.com/anthropic/coding",
    models: ["ernie-4.5", "deepseek-v3"],
    description: "百度千帆 Coding Plan",
    authMethods: ["apikey", "batch"],
    website: "https://cloud.baidu.com/product/qianfan_modelbuilder",
    codingPlan: { models: ["ernie-4.5"] },
  },
  {
    id: "volcengine-agentplan",
    name: "火山 Agentplan",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://ark.cn-beijing.volces.com/api/coding",
    models: ["doubao-seed", "deepseek-v3"],
    description: "火山方舟 Agent/Coding Plan",
    authMethods: ["apikey", "batch"],
    website: "https://www.volcengine.com",
    codingPlan: { models: ["doubao-seed"] },
  },
  {
    id: "doubao-seed",
    name: "DouBaoSeed",
    group: "cn_official",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://ark.cn-beijing.volces.com/api/compatible",
    models: ["doubao-seed"],
    description: "火山方舟豆包兼容模式",
    authMethods: ["apikey", "batch"],
    website: "https://console.volcengine.com/ark",
  },
  {
    id: "byteplus",
    name: "BytePlus",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://ark.ap-southeast.bytepluses.com/api/coding",
    models: ["doubao-seed"],
    description: "BytePlus ModelArk 海外节点",
    authMethods: ["apikey", "batch"],
    website: "https://www.byteplus.com",
  },
  {
    id: "minimax",
    name: "MiniMax",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.minimaxi.com/anthropic",
    models: ["MiniMax-Text-01"],
    description: "MiniMax 开放平台，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://platform.minimaxi.com",
  },
  {
    id: "minimax-en",
    name: "MiniMax en",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.minimax.io/anthropic",
    models: ["MiniMax-Text-01"],
    description: "MiniMax 海外节点",
    authMethods: ["apikey", "batch"],
    website: "https://platform.minimax.io",
  },
  {
    id: "stepfun",
    name: "StepFun",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.stepfun.com/step_plan",
    models: ["step-2"],
    description: "阶跃星辰 Step Plan",
    authMethods: ["apikey", "batch"],
    website: "https://platform.stepfun.com/step-plan",
  },
  {
    id: "stepfun-en",
    name: "StepFun en",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.stepfun.ai/step_plan",
    models: ["step-2"],
    description: "阶跃星辰海外节点",
    authMethods: ["apikey", "batch"],
    website: "https://platform.stepfun.ai/step-plan",
  },
  {
    id: "longcat",
    name: "Longcat",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.longcat.chat/anthropic",
    models: ["longcat"],
    description: "美团 Longcat，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://longcat.chat/platform",
  },
  {
    id: "kat-coder",
    name: "KAT-Coder",
    group: "cn_official",
    protocols: ["anthropic"],
    baseUrl: "https://vanchin.streamlake.ai/api/gateway/v1/endpoints",
    models: ["kat-coder"],
    description: "StreamLake KAT-Coder（需 Endpoint ID）",
    authMethods: ["apikey", "token"],
    allowCustomBaseUrl: true,
    website: "https://console.streamlake.ai",
  },
  {
    id: "bailing",
    name: "BaiLing",
    group: "cn_official",
    protocols: ["anthropic"],
    baseUrl: "https://api.tbox.cn/api/anthropic",
    models: ["bailing"],
    description: "蚂蚁百灵 TBox，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://alipaytbox.yuque.com",
  },
  {
    id: "xiaomi-mimo",
    name: "Xiaomi MiMo",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.xiaomimimo.com/anthropic",
    models: ["mimo"],
    description: "小米 MiMo，Anthropic 协议",
    authMethods: ["apikey", "batch"],
    website: "https://platform.xiaomimimo.com",
  },
  {
    id: "xiaomi-mimo-token",
    name: "Xiaomi MiMo Token Plan",
    group: "cn_official",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://token-plan-cn.xiaomimimo.com/anthropic",
    baseUrls: {
      anthropic: "https://token-plan-cn.xiaomimimo.com/anthropic",
      chat: "https://token-plan-cn.xiaomimimo.com/v1",
    },
    modelDiscovery: {
      url: "https://token-plan-cn.xiaomimimo.com/v1",
      protocol: "chat",
    },
    models: ["mimo"],
    description: "小米 MiMo Token Plan（国内），兼容 Anthropic 与 OpenAI 协议",
    authMethods: ["apikey", "token"],
    allowCustomBaseUrl: true,
    website: "https://platform.xiaomimimo.com",
    codingPlan: { models: ["mimo"] },
  },

  // ── 聚合与中转 ───────────────────────────────────────────
  {
    id: "openrouter",
    name: "OpenRouter",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://openrouter.ai/api",
    models: ["openrouter/auto", "deepseek/deepseek-chat"],
    description: "多模型聚合与免费模型路由",
    recommended: true,
    freeModels: true,
    authMethods: ["apikey", "batch"],
    website: "https://openrouter.ai",
  },
  {
    id: "siliconflow",
    name: "SiliconFlow",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://api.siliconflow.cn",
    models: ["Qwen/Qwen3-Coder", "deepseek-ai/DeepSeek-V3"],
    description: "硅基流动，含免费额度与免费模型",
    recommended: true,
    freeModels: true,
    authMethods: ["apikey", "batch"],
    website: "https://siliconflow.cn",
  },
  {
    id: "siliconflow-en",
    name: "SiliconFlow en",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://api.siliconflow.com",
    models: ["Qwen/Qwen3-Coder"],
    description: "硅基流动海外节点",
    authMethods: ["apikey", "batch"],
    website: "https://siliconflow.com",
  },
  {
    id: "modelscope",
    name: "ModelScope",
    group: "aggregator",
    protocols: ["chat"],
    baseUrl: "https://api-inference.modelscope.cn",
    models: ["Qwen/Qwen3-Coder"],
    description: "魔搭社区推理 API 与免费模型",
    freeModels: true,
    authMethods: ["apikey", "batch"],
    website: "https://modelscope.cn",
  },
  {
    id: "aihubmix",
    name: "AiHubMix",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://aihubmix.com",
    models: [],
    description: "多模型聚合服务",
    authMethods: ["apikey", "batch"],
    website: "https://aihubmix.com",
  },
  {
    id: "dmxapi",
    name: "DMXAPI",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://www.dmxapi.cn",
    models: [],
    description: "DMXAPI 聚合服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.dmxapi.cn",
  },
  {
    id: "zetaapi",
    name: "ZetaAPI",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.zetaapi.ai",
    models: [],
    description: "ZetaAPI 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://zetaapi.ai",
  },
  {
    id: "fennoai",
    name: "FennoAI",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.fenno.ai",
    models: [],
    description: "FennoAI 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://api.fenno.ai",
  },
  {
    id: "runapi",
    name: "RunAPI",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://runapi.co",
    models: [],
    description: "RunAPI 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://runapi.co",
  },
  {
    id: "unity2",
    name: "Unity2.ai",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.unity2.ai",
    models: [],
    description: "Unity2.ai 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://unity2.ai",
  },
  {
    id: "shengsuanyun",
    name: "胜算云",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://router.shengsuanyun.com/api",
    models: [],
    description: "胜算云 Router 聚合",
    authMethods: ["apikey", "batch"],
    website: "https://www.shengsuanyun.com",
  },
  {
    id: "subrouter",
    name: "SubRouter",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://subrouter.ai",
    models: [],
    description: "SubRouter 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://subrouter.ai",
  },
  {
    id: "claudeapi-apito",
    name: "ClaudeAPI",
    group: "aggregator",
    protocols: ["anthropic"],
    baseUrl: "https://gw.apito.ai",
    models: [],
    description: "Apito ClaudeAPI 网关",
    authMethods: ["apikey", "batch"],
    website: "https://www.apito.ai",
  },
  {
    id: "code0",
    name: "Code0",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://code0.ai",
    models: [],
    description: "Code0 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://code0.ai",
  },
  {
    id: "teamorouter",
    name: "TeamoRouter",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.teamorouter.com",
    models: [],
    description: "TeamoRouter 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://teamorouter.com",
  },
  {
    id: "nekocode",
    name: "NekoCode",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://nekocode.ai",
    models: [],
    description: "NekoCode 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://nekocode.ai",
  },
  {
    id: "a6api",
    name: "A6API",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.a6api.com",
    models: [],
    description: "A6API 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://www.a6api.com",
  },
  {
    id: "atlascloud",
    name: "AtlasCloud",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.atlascloud.ai",
    models: [],
    description: "AtlasCloud Coding Plan",
    authMethods: ["apikey", "batch"],
    website: "https://www.atlascloud.ai",
  },
  {
    id: "compshare",
    name: "Compshare",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://api.modelverse.cn",
    models: [],
    description: "优云智算 Compshare",
    authMethods: ["apikey", "batch"],
    website: "https://www.compshare.cn",
  },
  {
    id: "compshare-coding",
    name: "Compshare Coding Plan",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://cp.compshare.cn",
    models: [],
    description: "优云智算 Coding Plan",
    authMethods: ["apikey", "batch"],
    website: "https://www.compshare.cn",
    codingPlan: {},
  },
  {
    id: "ccsub",
    name: "CCSub",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://www.ccsub.net",
    models: [],
    description: "CCSub 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://www.ccsub.net",
  },
  {
    id: "qiniu",
    name: "七牛云",
    group: "aggregator",
    protocols: ["chat", "anthropic"],
    baseUrl: "https://api.qnaigc.com",
    models: [],
    description: "七牛云 AI 推理聚合",
    authMethods: ["apikey", "batch"],
    website: "https://s.qiniu.com",
  },
  {
    id: "amux",
    name: "Amux",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.amux.ai",
    models: [],
    description: "Amux 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://amux.ai",
  },
  {
    id: "cherryin",
    name: "CherryIN",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://open.cherryin.net",
    models: [],
    description: "CherryIN 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://open.cherryin.ai",
  },
  {
    id: "therouter",
    name: "TheRouter",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.therouter.ai",
    models: [],
    description: "TheRouter 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://therouter.ai",
  },
  {
    id: "novita",
    name: "Novita AI",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://api.novita.ai/anthropic",
    models: [],
    description: "Novita AI 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://novita.ai",
  },
  {
    id: "nvidia",
    name: "Nvidia",
    group: "aggregator",
    protocols: ["chat"],
    baseUrl: "https://integrate.api.nvidia.com",
    models: [],
    description: "NVIDIA NIM，OpenAI Chat 协议",
    authMethods: ["apikey", "batch"],
    website: "https://build.nvidia.com",
  },
  {
    id: "pipellm",
    name: "PIPELLM",
    group: "aggregator",
    protocols: ["anthropic", "chat"],
    baseUrl: "https://cc-api.pipellm.ai",
    models: [],
    description: "PIPELLM 聚合中转",
    authMethods: ["apikey", "batch"],
    website: "https://code.pipellm.ai",
  },

  // ── 第三方服务 ───────────────────────────────────────────
  {
    id: "packycode",
    name: "PackyCode",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://www.packyapi.ai",
    models: [],
    description: "PackyCode 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.packyapi.ai",
  },
  {
    id: "apinebula",
    name: "APINebula",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://apinebula.ai",
    models: [],
    description: "APINebula 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://apinebula.ai",
  },
  {
    id: "aicodemirror",
    name: "AICodeMirror",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.aicodemirror.ai/api/claudecode",
    models: [],
    description: "AICodeMirror 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.aicodemirror.ai",
  },
  {
    id: "patewayai",
    name: "PatewayAI",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.pateway.ai",
    models: [],
    description: "PatewayAI 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://pateway.ai",
  },
  {
    id: "aigocode",
    name: "AIGoCode",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.aigocode.app",
    models: [],
    description: "AIGoCode 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://aigocode.app",
  },
  {
    id: "aicoding",
    name: "AICoding",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.aicoding.inc",
    models: [],
    description: "AICoding 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://aicoding.inc",
  },
  {
    id: "apikey-fun",
    name: "APIKEY.FUN",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.apikey.fun",
    models: [],
    description: "APIKEY.FUN 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://apikey.fun",
  },
  {
    id: "claudecn",
    name: "ClaudeCN",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://claudecn.top",
    models: [],
    description: "ClaudeCN 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://claudecn.top",
  },
  {
    id: "sssaicode",
    name: "SSSAiCode",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://node-hk.sssaicodeapi.com/api",
    models: [],
    description: "SSSAiCode 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://sssaicodeapi.com",
  },
  {
    id: "micu",
    name: "Micu",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://www.micuapi.ai",
    models: [],
    description: "Micu 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.micuapi.ai",
  },
  {
    id: "rightcode",
    name: "RightCode",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://www.rightapi.ai/claude",
    models: [],
    description: "RightCode 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.rightapi.ai",
  },
  {
    id: "etok",
    name: "ETok.ai",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.etok.ai",
    models: [],
    description: "ETok.ai 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://etok.ai",
  },
  {
    id: "cubence",
    name: "Cubence",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.cubence.com",
    models: [],
    description: "Cubence 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://cubence.com",
  },
  {
    id: "crazyrouter",
    name: "CrazyRouter",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://cn.crazyrouter.com",
    models: [],
    description: "CrazyRouter 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.crazyrouter.com",
  },
  {
    id: "sudocode-chat",
    name: "SudoCode.chat",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://api.sudocode.chat",
    models: [],
    description: "SudoCode.chat 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://sudocode.chat",
  },
  {
    id: "sudocode-us",
    name: "SudoCode.us",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://sudocode.us",
    models: [],
    description: "SudoCode.us 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://sudocode.us",
  },
  {
    id: "opencode-go",
    name: "OpenCode Go",
    group: "third_party",
    protocols: ["chat"],
    baseUrl: "https://opencode.ai/zen/go",
    models: [],
    description: "OpenCode Go，OpenAI Chat 协议",
    authMethods: ["apikey", "token"],
    website: "https://opencode.ai/go",
  },
  {
    id: "relaxycode",
    name: "RelaxyCode",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://www.relaxycode.com",
    models: [],
    description: "RelaxyCode 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://www.relaxycode.com",
  },
  {
    id: "e-flowcode",
    name: "E-FlowCode",
    group: "third_party",
    protocols: ["anthropic"],
    baseUrl: "https://e-flowcode.cc",
    models: [],
    description: "E-FlowCode 第三方服务",
    authMethods: ["apikey", "batch"],
    website: "https://e-flowcode.cc",
  },

  // ── 云服务商 ─────────────────────────────────────────────
  {
    id: "aws-bedrock-aksk",
    name: "AWS Bedrock (AKSK)",
    group: "cloud",
    protocols: ["anthropic"],
    baseUrl: "https://bedrock-runtime.us-east-1.amazonaws.com",
    models: ["anthropic.claude-sonnet-4", "anthropic.claude-opus-4"],
    description: "AWS Bedrock，AK/SK 签名鉴权",
    authMethods: ["apikey"],
    allowCustomBaseUrl: true,
    website: "https://aws.amazon.com/bedrock/",
  },
  {
    id: "aws-bedrock-apikey",
    name: "AWS Bedrock (API Key)",
    group: "cloud",
    protocols: ["anthropic"],
    baseUrl: "https://bedrock-runtime.us-east-1.amazonaws.com",
    models: ["anthropic.claude-sonnet-4", "anthropic.claude-opus-4"],
    description: "AWS Bedrock，API Key 鉴权",
    authMethods: ["apikey"],
    allowCustomBaseUrl: true,
    website: "https://aws.amazon.com/bedrock/",
  },

  // ── 自定义上游（高级）───────────────────────────────────
  {
    id: "custom",
    name: "自定义",
    group: "custom",
    protocols: [],
    baseUrl: "",
    models: [],
    description: "自定义 API 地址、格式、模型和 Key",
    authMethods: ["apikey"],
    allowCustomBaseUrl: true,
  },
];

export function getResourceTemplate(id: string) {
  return modelResourceTemplates.find((template) => template.id === id);
}
