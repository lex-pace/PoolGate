# PoolGate：本地网关 → 号池 → 大模型厂商真实链路

## 一、结论

原实现并非完全没有使用号池：每个推理入口都调用了 `router::select_account`，并把选中的 Account 凭证注入到上游请求中。

但原实现只完成了“单号选择”，没有形成完整号池闭环：

1. 每个请求只选一个账号，401 / 403 / 429 / 5xx / 网络失败时不会切换下一个账号；
2. 上游返回 429 / 500 仍被错误记录为 success，并重置账号熔断器；
3. 选中熔断账号后直接返回 503，不继续尝试池内其他账号；
4. `last_used_at` 从未更新，资源页视觉上看不到账号被使用；
5. 部分协议选中账号后，上游 URL 或鉴权头错误：Responses、Anthropic route takeover、Gemini 原生路径均有断点。

现已补齐“筛选 → 选号 → 调厂商 → 失败切号 → 日志留痕 → 最近使用更新”的闭环。

## 二、请求真实链路

```text
本地 Agent
  │
  │  HTTP POST
  │  /v1/responses | /v1/chat/completions | /v1/messages
  │  /v1beta/models/{model}:{action}
  ▼
PoolGate axum handler
  │
  ├─ 读取 X-Group-Id / X-Pool-Group（默认 default）
  ├─ 从请求体或路径提取 model
  └─ 读取分组路由策略（默认 round_robin）
  ▼
router::select_account_excluding
  │
  ├─ 有明确分组且包含账号：只使用组内账号
  ├─ default / 无分组 / 空分组：回退整个 accounts 表
  ├─ 排除 disabled、不可路由、已尝试账号
  ├─ 按入口协议过滤 responses / chat / anthropic / gemini
  ├─ 按 model 能力过滤
  ├─ 健康状态过滤
  └─ 按 round_robin / priority / random / least_used 等策略选号
  ▼
选中 Account + Provider
  │
  ├─ accounts.mark_used → 更新 last_used_at
  ├─ CircuitBreaker.allow_request → 跳过已熔断账号
  ├─ authorization_secret / api_key_secret → 读取该账号自己的凭证
  └─ Provider 仅提供 Base URL、协议、代理、超时等连接器配置
  ▼
协议适配与上游请求
  │
  ├─ responses 原生 → /v1/responses
  ├─ Codex OAuth → /backend-api/codex/responses
  ├─ chat → /v1/chat/completions + Bearer
  ├─ anthropic → /v1/messages + x-api-key + anthropic-version
  └─ gemini → /v1beta/models/{model}:{action}?key=账号密钥
  ▼
真实大模型厂商 / 聚合上游
  │
  ├─ 2xx：返回客户端、清除账号连续失败
  ├─ 400/404/422：返回客户端（通常是请求问题，不切号）
  └─ 401/403/408/429/5xx/网络失败：记录失败并切换下一个账号
  ▼
最多尝试 3 个不同账号
  │
  ├─ 每次尝试写 request_logs（account_id + provider_id + status_code）
  └─ 最终响应增加账号路由证明头
```

## 三、如何证明号池已被使用

发送请求时使用 `curl -i` 查看响应头：

```text
x-poolgate-account-id: acct_xxx
x-poolgate-provider-id: prov_xxx
x-poolgate-attempt: 1
```

如果第一个账号 429 / 401 / 5xx，第二个账号成功：

```text
x-poolgate-account-id: acct_second
x-poolgate-attempt: 2
```

同时：

- “请求日志”页面中的 `account_id` 会记录每次真实尝试；
- 失败切号时同一个外部请求可能出现多条日志，每条对应一个账号；
- “模型资源”详情的“最近使用”会随选号更新。

## 四、协议入口与号池匹配

| 本地 Agent | 网关入口 | 号池候选协议 | 上游厂商路径 |
|---|---|---|---|
| Codex / Responses 客户端 | `/v1/responses` | `responses`；开启路由接管时也可选 `chat` / `anthropic` | 原生 Responses，或转换到 Chat / Anthropic |
| OpenAI Chat 客户端 | `/v1/chat/completions` | 仅 `chat` | `/v1/chat/completions` |
| Claude / Anthropic 客户端 | `/v1/messages` | 仅 `anthropic` | `/v1/messages` |
| Gemini 客户端 | `/v1beta/models/{model}:{action}` | 仅 `gemini` | 保留 model 与 action 的 Gemini 原生路径 |

Gemini 不通过 Responses 路由接管转换，保持原生直连语义。

## 五、本次改动文件

- `src-tauri/src/proxy/server.rs`
  - 四个推理入口接入最多 3 个不同账号的失败切换；
  - 正确识别非 2xx；
  - 增加路由证明响应头；
  - Responses 按选中账号协议构造 URL 与鉴权。
- `src-tauri/src/proxy/router.rs`
  - 新增 `select_account_excluding`，重试时排除已尝试账号；
  - 选中账号后更新 `last_used_at`。
- `src-tauri/src/db/accounts.rs`
  - 新增 `mark_used`。
- `src-tauri/src/proxy/protocol/mod.rs`
  - 新增统一 Base URL + API 路径拼接，避免 `/v1/v1/...`。
- `src-tauri/src/proxy/protocol/openai.rs`
  - 使用统一 URL 拼接。
- `src-tauri/src/proxy/protocol/anthropic.rs`
  - 使用统一 URL 拼接。
- `src-tauri/src/proxy/protocol/gemini.rs`
  - 保留客户端请求的 `{model}:{action}`，不再错误 POST 到 Base URL 根路径。
- `src-tauri/src/proxy/protocol/responses.rs`
  - 原生 Responses 默认上游路径改为 `/v1/responses`。

## 六、验证状态

- `cargo fmt --all --check`：通过
- `cargo build`：通过
- `cargo test --lib`：18/18 项测试通过
- `npm run build`（`tsc + vite build`）：通过，2199 个模块完成生产构建

真实厂商端到端仍需要至少两个同协议、同模型且有效的账号，才能现场观察 `attempt: 2` 的失败切号。代码层闭环已完成。
