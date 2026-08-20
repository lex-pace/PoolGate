# PoolGate 多 Provider 统一适配方案（Gemini / Codex / Claude / Qwen / Grok）

> 决策边界：号池反代仅覆盖这五家国外大模型，使用官方 OAuth 通道；不做任何逆向私有端点（Cursor / Windsurf / Trae / Kiro / CodeBuddy 等）。其他 AI 编程工具只做账号管理与配额，参考 Cockpit-tools。

---

## 1. 目标

| 维度 | 要求 |
|------|------|
| Provider 数量 | 5 家（Codex / Claude / Gemini / Qwen / Grok）|
| 用户体验 | OAuth 一键登录 → 进号池 → 本地统一端点即用 |
| 协议覆盖 | PoolGate 对外提供 `/v1/chat/completions` + `/v1/responses` + `/v1/messages` 三条入口，内部按 Provider 协议做转换 |
| 合规 | 仅使用官方客户端同端点，不伪装官方客户端名（`originator: poolgate`），不共享账号，不转售订阅权益 |
| 参考实现 | CLIProxyAPI（router-for-me，Rust）已原生实现四家 OAuth，Codex adapter 已在 PoolGate 内闭环 |

---

## 2. 总体架构

```
                   ┌─────────────────────────────────┐
                   │       统一前端 OAuth 登录          │
                   │  PKCE / Device Flow / PAT 直接填入 │
                   └──────────┬──────────────────────┘
                              │
                   ┌──────────▼──────────────────────┐
                   │     provider_adapters/            │
                   │  ┌─ codex_adapter.rs   (✅ 已完成) │
                   │  ├─ claude_adapter.rs             │
                   │  ├─ copilot_adapter.rs            │
                   │  ├─ gemini_adapter.rs             │
                   │  ├─ qwen_adapter.rs               │
                   │  └─ grok_adapter.rs               │
                   └──────────┬──────────────────────┘
                              │
    ┌─────────────────────────▼───────────────────────────┐
    │                  proxy/router.rs                      │
    │  codex_protocol_matches() → account selection         │
    │  每 Provider 只参与对应协议入口（responses/messages/...）│
    │  单账号同池防轮询                                       │
    └─────────────────────────┬───────────────────────────┘
                              │
    ┌─────────┬───────────────┼──────────────┬──────────┐
    │ Codex   │ Claude Code   │ Copilot      │ Gemini   │ ...
    │ 原生     │ Messages API  │ Chat API     │ generate │
    │ Responses│              │              │ Content   │
    └─────────┴───────────────┴──────────────┴──────────┘
```

### 2.1 统一适配接口（trait 结构，供代码参考）

```rust
// 伪代码，实际 Rust 可用 enum dispatch（无需动态 trait）
struct ProviderAdapterConfig {
    provider_type: &'static str,
    credential_types: &'static [&'static str],
    oauth_authorize: &'static str,
    oauth_token: &'static str,
    oauth_client_id: &'static str,
    oauth_scope: &'static str,
    oauth_redirect_port: u16,       // 0 = device flow（无需回调）
    upstream_base_url: &'static str,
    upstream_path: &'static str,
    originator: &'static str,
    stream_required: bool,          // Codex/Qwen 强制 true
    system_prefix: Option<&'static str>, // Claude 注入前缀
    health_check_path: &'static str,
    health_check_method: &'static str,  // GET / POST
}
```

---

## 3. 各 Provider 适配详情

### 3.1 Codex — ✅ 已完成（`services/codex_adapter.rs`）

| 项 | 值 |
|----|---|
| OAuth 端点 | `https://auth.openai.com/oauth/authorize` → `https://auth.openai.com/oauth/token` |
| Client ID | `app_EMoamEEZ73f0CkXaXp7hrann`（官方 Codex CLI 公开 ID）|
| Scope | `openid profile email offline_access` |
| 回调端口 | `1455` |
| Token 类型 | OAuth access_token + refresh_token（PKCE，code_verifier 本地生成）|
| 上游端点 | `https://chatgpt.com/backend-api/codex/responses` |
| 请求头 | `Authorization: Bearer` + `ChatGPT-Account-Id` + `originator: poolgate` + `Accept: text/event-stream` |
| Body 约束 | 强制 `stream: true`，强制 `store: false` |
| 健康检查 | `GET backend-api/codex/models?client_version=1.0.0` |
| 协议 | Responses（原生）|
| 路由约束 | 不进 default 兜底池，只匹配 responses 入口，同池单账号 |

---

### 3.2 Claude Code（Anthropic 订阅 OAuth）

| 项 | 值 |
|----|---|
| OAuth 端点 | `https://console.anthropic.com/oauth/authorize`（Pro）/ `https://claude.ai/oauth/authorize`（Max）|
| Token 端点 | `https://console.anthropic.com/v1/oauth/token` |
| Client ID | 官方 Claude Code 公开 client（无需 secret）|
| Scope | `org:create_api_key user:profile user:inference` |
| 回调端口 | `54545`（CLIProxyAPI 已验证）|
| Token 类型 | access_token + refresh_token（PKCE）；`claude setup-token` 可生成一年期 token（`CLAUDE_CODE_OAUTH_TOKEN`）|
| 上游端点 | `https://api.anthropic.com/v1/messages` |
| 请求头 | `Authorization: Bearer <token>`（注意不是 x-api-key）+ `anthropic-version: 2023-06-01` |
| **关键约束** | 必须在 system messages 最前注入固定前缀：`"You are Claude Code, Anthropic's official CLI for Claude."`（未注入会返回错误）|
| Body 格式 | Anthropic Messages API（`messages` + `max_tokens` + `system`）|
| 健康检查 | `POST api.anthropic.com/v1/messages`，body `{"model":"claude-3-haiku-20240307","max_tokens":1,"messages":[{"role":"user","content":"ping"}]}` |
| 协议 | Anthropic Messages |
| 路由约束 | 只匹配 `messages` / `anthropic` 入口 |
| 实现要点 | PoolGate 已有 `proxy/protocol/anthropic.rs` 做 Responses→Anthropic 转换；Claude adapter 需要在请求 body 的 `system` 字段最前拼接前缀（前端登录时先填入 plan type，区分 Pro/Max authorize URL）|

---

### 3.3 GitHub Copilot（官方 Chat API）

| 项 | 值 |
|----|---|
| OAuth 端点 | `https://github.com/login/device`（device flow）|
| Token 端点 | `https://github.com/login/oauth/access_token`（device flow poll）|
| 或者 PAT 方式 | 直接填入 `ghp_...` 或 `ghu_...`，无需 OAuth 流程（最简）|
| Copilot Token | 用 GitHub Token 调 `https://api.github.com/copilot_internal/v2/token` 换取短期 Copilot token（25 分钟有效）|
| 上游端点 | `https://api.githubcopilot.com/chat/completions`（个人）/ `https://api.individual.githubcopilot.com/chat/completions` / 企业版另选 |
| 请求头 | `Authorization: Bearer <copilot_token>` + `Copilot-Integration-Id: copilot-developer-cli` + `Content-Type: application/json` |
| Body 格式 | OpenAI Chat Completions 格式（`messages` + `model` + `stream`）|
| 支持模型 | gpt-4o / gpt-4o-mini / claude-sonnet-4 / gemini-2.5-pro / o3-mini 等（取决于 Copilot 订阅等级）|
| /responses 支持 | gpt-5 家族只支持 `/responses`，需根据模型自动路由 |
| 健康检查 | `GET api.githubcopilot.com/models`（带 Copilot token + Integration-Id）|
| 协议 | Chat Completions + Responses（双端点，按模型自动切换）|
| 路由约束 | 同时支持 chat 和 responses 入口，需要 router 能按模型动态选端点 |
| 实现要点 | Copilot token 需定时刷新（25 分钟）；PAT → Copilot token 换取是关键步骤；LLM 官方文档/docker-agent 已有完整实现可参考 |

---

### 3.4 Gemini（Google OAuth）

| 项 | 值 |
|----|---|
| OAuth 端点 | `https://accounts.google.com/o/oauth2/auth`（Google OAuth）|
| Token 端点 | `https://oauth2.googleapis.com/token` |
| Scope | `https://www.googleapis.com/auth/generative-language`（或按 CLIProxyAPI 实现调整）|
| 回调端口 | `8085`（CLIProxyAPI 已验证）|
| 免费额度 | AI Studio 免费 tier（Flash 模型每天 1000 次/60 RPM，Pro 约 50 次/5 RPM）|
| 上游端点 | `https://generativelanguage.googleapis.com/v1beta/models/{model}:{generateContent|streamGenerateContent}` |
| 请求头 | `Authorization: Bearer <access_token>` 或 API Key query param（两种认证都支持）|
| Body 格式 | Gemini native（`contents` + `generationConfig`），PoolGate 已有 `proxy/protocol/gemini.rs` 做 Responses→Gemini 转换 |
| stream 支持 | `streamGenerateContent`（SSE，需 `alt=sse` query param）|
| 健康检查 | `GET generativelanguage.googleapis.com/v1beta/models` |
| 协议 | Gemini native |
| 路由约束 | 只匹配 `gemini` 入口 |
| 实现要点 | PoolGate 已有 Gemini 协议处理；需要新增 Google OAuth 登录（PKCE）；CLIProxyAPI 源码有完整的 Google OAuth 实现可参考；还有 cookie 认证方式（`--gemini-web-auth`）作为可选方案 |

---

### 3.5 Qwen（通义千问 Chat OAuth）

| 项 | 值 |
|----|---|
| OAuth 方式 | **Device Flow**（无本地回调端口，用户在浏览器输入 device code）|
| Device Code 端点 | `https://chat.qwen.ai/api/v1/oauth2/device/code` |
| Token 端点 | `https://chat.qwen.ai/api/v1/oauth2/token` |
| Client ID | `f0304373b74a44d2b584a3fb70ca9e56`（CLIProxyAPI 源码硬编码）|
| Scope | `openid profile email model.completion` |
| Grant Type | `urn:ietf:params:oauth:grant-type:device_code` |
| PKCE | 需要（code_verifier 本地生成 32 字节，S256 挑战）|
| Token 类型 | access_token + refresh_token（含 `resource_url` 字段，指定实际 API base）|
| 上游端点 | 从 token 响应的 `resource_url` 字段读取（动态）|
| 请求头 | `Authorization: Bearer <access_token>` |
| Body 格式 | OpenAI Chat Completions 兼容格式（`messages` + `model` + `stream`）|
| 健康检查 | `GET {resource_url}/v1/models` |
| 协议 | Chat Completions（OpenAI compatible）|
| 路由约束 | 匹配 chat / responses 入口 |
| 实现要点 | Device flow 无需本地回调端口，前端显示 device code + verification URI，后端 poll token endpoint 直到用户授权完成；PKCE code_verifier 需要在 poll 过程中保持一致；`resource_url` 是动态的，需存入 provider.base_url |

---

### 3.6 Grok（xAI 订阅 OAuth）

| 项 | 值 |
|----|---|
| OAuth 端点 | `https://accounts.x.ai/authorize`（xAI 官方，类似 Codex 的 auth.openai.com）|
| Token 端点 | `https://accounts.x.ai/oauth2/token`（确认中，参考 grok-proxy 实现）|
| 回调端口 | 待确认（grok-proxy 用网站授权回调，PoolGate 可沿用 PKCE 本地回调模式）|
| 订阅权益 | Grok Chat 订阅（xAI 计划），通过 OAuth 授权后可用订阅额度 |
| 上游端点 | `https://api.x.ai/chat/completions`（官方 API 端点，也支持订阅 token）|
| 请求头 | `Authorization: Bearer <access_token>` |
| Body 格式 | OpenAI Chat Completions 兼容格式（`messages` + `model` + `stream`）|
| 支持模型 | `grok-4.5`、`grok-4-code` |
| 健康检查 | `GET api.x.ai/v1/models`（或 `chat/completions` 最小请求）|
| 协议 | Chat Completions（OpenAI compatible）|
| 路由约束 | 匹配 chat / responses 入口 |
| 实现要点 | 优先参考 grok-proxy 源码（github.com/werbenhu/grok-proxy）确认 OAuth 端点和 token 刷新逻辑；xAI 官方 API key 方式作为备选（类似 Copilot 的 PAT 模式）|

---

## 4. 数据库变更

### 4.1 Provider 新增字段（可选）

现有 `providers` 表结构已足够，新增字段（migration 014）：

```sql
-- 增加 provider 子类型，用于区分同一官方厂商的 OAuth 和 API Key 两种认证
ALTER TABLE providers ADD COLUMN auth_mode TEXT DEFAULT 'api_key';
-- auth_mode 可选值: api_key | oauth_device_flow | oauth_pkce | pat_to_token

-- 增加 OAuth 配置存储（authorize_url、token_url、client_id、scope 等静态配置）
ALTER TABLE providers ADD COLUMN oauth_config TEXT;
-- JSON 格式: {"authorize_url":"...","token_url":"...","client_id":"...","scope":"...","redirect_port":1455,...}
```

### 4.2 Account credential_type 扩展

现有 `credential_type` 已支持 `api_key/upstream_key/oauth/token/codex_oauth`，新增：

| credential_type | 说明 |
|---|---|
| `codex_oauth` | ✅ 已存在 |
| `claude_oauth` | Claude Code 订阅 OAuth |
| `copilot_pat` | GitHub Copilot PAT |
| `copilot_oauth` | GitHub Copilot device flow OAuth |
| `gemini_oauth` | Google OAuth（Gemini 订阅/AI Studio 免费额度）|
| `qwen_oauth` | Qwen Chat OAuth（device flow）|
| `grok_oauth` | xAI Grok 订阅 OAuth |

### 4.3 credential_data 统一结构

复用现有 `CredentialPayload`，增加 `resource_url` 字段（Qwen 动态 API base）：

```rust
pub struct CredentialPayload {
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub session_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_at: Option<String>,
    pub token_type: Option<String>,
    pub base_url: Option<String>,
    pub agent_identity: Option<Value>,
    pub metadata: Option<Value>,
    pub resource_url: Option<String>, // Qwen 动态 API base
}
```

---

## 5. 前端变更

### 5.1 OAuth 登录 UI（AgentGroups / AccountPool 页面）

新增「添加订阅账号」面板，按 Provider 类型展示登录方式：

| Provider | 登录方式 | 前端交互 |
|---|---|---|
| Codex | OAuth PKCE（打开浏览器）| 已有，复用 |
| Claude | OAuth PKCE（打开浏览器）| 新增，区分 Pro/Max 两个 authorize URL |
| Copilot | PAT 直接填入 或 device flow | PAT 最简：填入 `ghp_...`，点击验证；device flow 显示 code + 链接 |
| Gemini | OAuth PKCE（打开浏览器）| 新增，显示免费额度说明 |
| Qwen | Device Flow（显示 device code）| 新增：展示 verification URI + user code，用户输入后轮询 |
| Grok | OAuth PKCE（打开浏览器）| 新增，参考 Codex 的实现 |

### 5.2 状态展示

复用现有 Badge 体系：

| 状态 | badge | 说明 |
|---|---|---|
| `active + healthy` | 可用（绿）| OAuth token 有效 + 最近一次推理成功 |
| `active + unchecked` | 未检查（灰）| 刚登录/token 刷新成功，尚未发真实推理 |
| `disabled + adapter_required` | 待适配（黄）| 暂时保留，等专用适配器就绪 |
| `token_expired` | Token 过期（红）| refresh 失败，需重新 OAuth |

---

## 6. 协议转换矩阵

PoolGate 对外统一端点，内部按 Provider 实际协议做转换：

| PoolGate 入口 → | Codex Responses | Claude Messages | Copilot Chat | Copilot Responses | Gemini GenerateContent | Qwen Chat | Grok Chat |
|---|---|---|---|---|---|---|---|
| `/v1/responses` | ✅ 原生透传 | responses→messages | responses→chat | ✅ 原生 | responses→gemini | responses→chat | responses→chat |
| `/v1/chat/completions` | chat→responses | chat→messages | ✅ 原生 | chat→responses | chat→gemini | ✅ 原生 | ✅ 原生 |
| `/v1/messages` | messages→responses | ✅ 原生 | messages→chat | messages→responses | messages→gemini | messages→chat | messages→chat |

PoolGate 已有 `proxy/protocol/responses.rs`、`anthropic.rs`、`gemini.rs`、`openai.rs` 四套转换层，新增路由逻辑只做入口→Provider 协议映射即可。

---

## 7. 健康检查策略

统一用「专用轻量探测」，不用通用 `/v1/models`（Claude、Grok 等不支持）：

| Provider | 探测方式 | 判定条件 |
|---|---|---|
| Codex | `GET backend-api/codex/models` | 2xx = healthy |
| Claude | `POST /v1/messages`（max_tokens=1）| 2xx = healthy；401 = token_expired |
| Copilot | `GET api.githubcopilot/models` + `Copilot-Integration-Id` | 2xx = healthy；401 = token expired |
| Gemini | `GET /v1beta/models` | 2xx = healthy |
| Qwen | `GET {resource_url}/v1/models` | 2xx = healthy |
| Grok | `GET api.x.ai/v1/models` | 2xx = healthy |

对 OAuth 账号：token 刷新成功后写 `unchecked`（不写 healthy），只有真实推理成功或专用探测成功才写 `healthy`（沿用 Codex adapter 的正确语义）。

---

## 8. 401 刷新重试机制（复用 Codex 模式）

统一为：真实请求收到 `401 Unauthorized` → 同账号互斥锁（`LazyLock<Mutex<HashMap>>`）→ 单次 token 刷新 → 重试一次 → 成功则更新健康，失败则标记 `token_expired`。

各 Provider 的 token 刷新端点不同，统一由 `provider_adapters/<provider>.rs` 内的 `refresh_after_unauthorized` 函数处理（或抽象为通用函数，传入 token URL）。

---

## 9. 测试方案

| 类别 | 内容 |
|------|------|
| 单元测试 | 每个 adapter 的 body 转换函数、header 构造、system 前缀注入 |
| 集成测试 | mock OAuth 服务器 → token 交换 → 下游请求构造（不发起真实上游请求）|
| 协议转换测试 | responses/chat/messages 三种入口 × 五家 Provider 的互转正确性 |
| 路由测试 | 单账号限制、协议匹配、default 池排除、健康状态过滤 |
| 既有测试 | 保持 `cargo test` 105 passed / 0 failed |

---

## 10. 落地顺序（建议）

| 顺序 | Provider | 原因 | 工作量 |
|---|---|---|---|
| 1 | **Copilot** | PAT 模式最简（直接填 key，无 OAuth 流程），跑通端到端 adapter 接口 | 1-2 天 |
| 2 | **Claude** | 官方 OAuth 是明确支持的（`claude setup-token` 是官方功能），PoolGate 已有 anthropic 转换层 | 2-3 天 |
| 3 | **Gemini** | Google OAuth + 免费额度用户量大，PoolGate 已有 gemini 转换层 | 2-3 天 |
| 4 | **Qwen** | Device flow 最简（无回调端口），Qwen 是国内用户高频工具 | 1-2 天 |
| 5 | **Grok** | 参考 grok-proxy 实现，xAI 端点信息最需确认 | 1-2 天 |

总工作量：8-12 天（含测试），可并行推进 Copilot/Claude。

---

## 11. 风险与决策点

| 风险项 | 影响 | 缓解 |
|---|---|---|
| Claude 订阅 OAuth 被 Anthropic 封禁 | 高 | Anthropic 官方 `claude setup-token` 是正式功能，风险低；PoolGate 用真实 product 标识（`originator: poolgate`）不伪装 |
| Copilot 限流（个人版）| 中 | Copilot token 25 分钟过期需刷新；合理间隔，不超速 |
| Qwen/Qwen Chat 改 OAuth 端点 | 中 | Device flow 端点写入 provider.oauth_config（可热更新），无需重新编译 |
| Grok OAuth 端点尚未100%确认 | 低 | 优先从 grok-proxy 源码提取，备选用 xAI 官方 API key |
| 五家账号统一进路由池的管理复杂度 | 低 | 每个 adapter 的 `pool_join_rules` 单独定义（Codex 单账号、Copilot 多账号可并行）|

---

## 12. 文件结构（新增/修改）

```
src-tauri/src/services/
├── codex_adapter.rs         ✅ 已完成（模板）
├── claude_adapter.rs        新增
├── copilot_adapter.rs       新增
├── gemini_adapter.rs        新增
├── qwen_adapter.rs          新增
├── grok_adapter.rs          新增
├── provider_registry.rs     新增（统一 adapter 注册和分发）
├── health_check.rs          修改（每 provider 专用探测）
├── oauth.rs                 修改（新增多家 OAuth 流程路由）
└── token_refresh.rs         修改（新增多家 token 刷新）

src-tauri/src/proxy/
├── server.rs                修改（responses/chat/messages handler 增加 provider 分支）
└── router.rs                修改（协议匹配增加 provider-specific 入口）

src-tauri/migrations/
└── 014_multi_provider_oauth.sql   新增

src/pages/AccountPool/
├── ImportCenter.tsx          修改（新增 OAuth 登录面板）
└── OAuthLogin.tsx            新增（通用 OAuth 登录组件）

src/pages/AgentGroups/
└── index.tsx                 修改（路由池添加供应商面板支持 OAuth 账号）
```

---

*方案版本：v1.0 · 2026-08-03 · 基于 CLIProxyAPI (router-for-me) 参考实现*
