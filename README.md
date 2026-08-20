<p align="center">
  <img src="src/assets/brand/poolgate-readme-header.png" alt="PoolGate — Local Agent Gateway" width="900" />
</p>

# PoolGate — Local Agent Gateway

本地模型网关：统一管理多来源的模型账号（API Key / OAuth / 订阅 / 各种导入格式），单端口（默认 9800）对外提供 OpenAI、Anthropic、Gemini 与 OpenAI Responses 兼容 API，内置账号健康监控、配额追踪、路由池调度与故障转移，并附带完整的请求日志、用量分析与 **Token Monitor** 本地工具用量监控仪表盘。

品牌标识中的三条输入路径代表多协议与多来源模型资源，P 形门体代表本地控制边界，单一青色出口代表 PoolGate 对 Agent 暴露的统一端点。源文件位于 [`src/assets/brand`](src/assets/brand)。

## 产品定位

**个人使用为主，小团队多人共享 API Key 为辅。**

- 🧑‍💻 **个人使用（主）**：面向个人开发者与重度 Agent 用户，复刻 sub2api / new-api 的中转站能力（账号池路由、故障转移、多协议转换），但**不需要 Docker / 服务器部署**——桌面应用安装即用、一键启停，默认仅本机监听，账号、密钥与流量全部留在自己电脑上
- 👥 **小团队共享（辅）**：同一网络内的小团队可共享 **API Key 类**账号池——设置页切到「局域网共享」并配置网关访问密钥后，同事通过 `http://<本机局域网 IP>:9800` 接入，各自使用独立的客户端密钥；团队规模大、需要公网访问或精细计量时，sub2api / new-api 等中转站仍是更合适的选择
- ⛔ **OAuth 订阅账号不可共享**：Claude / Gemini / Copilot / Codex / Grok 等 OAuth 订阅账号仅限本机本人使用（上游 ToS 限制，导入时亦有提示），不要加入共享路由池或分发给同事；共享只适用于 API Key / 上游密钥类资源

## 核心特性

### 🔌 统一网关（单端口 · 多协议）
- **单端口对外**：默认监听 `127.0.0.1:9800`，客户端无需关心上游协议差异
- **监听地址可切换**：设置页「监听地址」支持 `仅本机`（默认，`127.0.0.1`）与 `局域网共享`（`0.0.0.0`）两种模式——局域网模式下网关自动重启并强制要求访问密钥，设置页实时展示本机局域网 IP 供同事配置（个人使用默认仅本机，小团队共享时切到局域网）
- **多协议端点**：
  - `POST /v1/chat/completions` — OpenAI 兼容
  - `POST /v1/messages` — Anthropic Messages
  - `POST /v1/responses` — OpenAI Responses（Codex CLI）
  - `POST /v1beta/models/{...}`、`/v1/models/{...}` — Gemini
  - `GET /v1/models` — 聚合后的可路由模型清单（只列出真实可用模型）
  - `GET /health` — 健康探针（始终免鉴权）
  - `GET /v1/stats` — 运行统计
- **协议转换矩阵**：`responses / chat / anthropic / gemini` 四种入口可互相转换（含路由接管），统一路由池可混合调度任意协议账号
- **网关访问密钥**：可设置全局访问密钥，客户端需携带 `Authorization: Bearer`、`x-api-key` 或 `x-goog-api-key`；密钥只写入系统凭证库，修改立即生效；**局域网监听模式下强制要求访问密钥**（未设置密钥时无法开启局域网，局域网中也不能清除密钥，避免账号池对网络裸奔）
- **流式响应与并发控制**：SSE 流式转发、按账号的并发信号量限制（真实反映活跃/可用/排队容量）

### 🏦 多来源账号池
- **统一凭证管理**：API Key、OAuth、Token 等凭证类型，敏感信息存入系统钥匙串（macOS Keychain / Windows 凭据管理器 / Linux Secret Service）
- **批量导入**：API Key 文本（`name=key`）、Codex `auth.json`、Sub2API、CPA、Cockpit 完整备份、通用 JSON / CSV
- **OAuth 登录流程**：Claude、Gemini、Copilot（设备码流 + PAT 校验）、Grok、Antigravity 等内置登录向导（**订阅账号仅限本机本人使用**，见「产品定位」）
- **OAuth / Token 自动刷新**：后台刷新循环保证订阅类账号不因过期而掉出路由池
- **健康监控**：健康检查（批量/单账号）、异常自动恢复（60s 冷却重检）、故障自动排除

### 🗂️ 路由池与调度
- **按模型能力组池**：路由池绑定模型资源（供应商 × 模型），严格池边界；未入池的模型不会逃逸
- **路由策略**：轮询 / 最少使用 / 优先级 / 随机 / 成本优先
- **Codex 串行（粘性）路由**：同一模型固定一个本人账号发起，额度耗尽或失败后才切换，绝不按请求轮询
- **会话粘性（Session Affinity）**：同一会话固定同一账号——优先读 `x-poolgate-session` / `session_id` / `conversation_id` 头，否则用首条用户消息指纹；既保住上游 prompt cache 命中率，也符合「一人一机」的正常流量画像；粘住的账号被故障转移排除后自动换绑
- **故障转移**：401 / 429 / 5xx 时自动排除失败账号并重试下一个候选
- **客户端密钥**：可创建多把访问密钥并限定到指定路由池与模型范围
- **缓存符对齐（跨协议 Token 口径归一）**：内部统一五元组口径（fresh input / cache read / cache write / output / available）——Anthropic 的 `input_tokens` 不含缓存、OpenAI `prompt_tokens` 与 Gemini `promptTokenCount` 含缓存，入口统一折算为「新鲜输入 + 读缓存 + 写缓存」，混池统计不再双算/漏算；跨协议转换时按目标协议词表重新编码（如 OpenAI → Anthropic 出口拆出 `cache_read_input_tokens`），流式与非流式全覆盖；`request_logs` 自 v021 迁移起拆分 `cache_read_tokens` / `cache_write_tokens` 列（`cache_tokens` 保留为两者之和，兼容旧查询）

### 🛡️ OAuth 订阅账号风控防护
订阅上游（ChatGPT Codex、Claude Code、Copilot、Gemini）会画像 HTTP 客户端，PoolGate 从四个层面降低被识别为第三方工具的概率：
- **官方客户端画像**（`services/client_profiles.rs`）：每家上游一份完整官方 CLI 指纹（User-Agent / originator / anthropic-beta / x-app / Editor-Version / x-goog-api-client），且内部自洽（杜绝「官方 originator + PoolGate UA」这种自相矛盾的组合）；Claude Code 请求自动注入必需的系统前缀（客户端已带则不重复）
- **会话粘性路由**：见上——同一会话不换号，避免「多身份共用一个凭证」的典型特征
- **行为节流**：订阅类账号（codex_oauth / claude_oauth / oauth / token / copilot_pat 等）按账号维度施加最小请求间隔（200ms）+ 随机抖动（0–120ms），消除机器般的完美节奏；计费 API Key 账号不受影响
- **封控信号识别**：403 / 特征文案（unusual activity / flagged / suspended / deactivated…）被识别为账号级事件——自动置为 `suspended` 健康状态（不参与路由、也不被 60s 自动恢复反复打）、写入 critical 告警、并在日志中留痕，而不是当作普通失败继续转移重试
- **刷新卫生**：token 只在 401 后刷新且按账号串行化（Codex / Claude 双通道），避免刷新风暴本身成为信号
- 诚实提示：订阅账号走自动化网关本身违反多数厂商 ToS，以上手段只能降低被识别概率、无法保证零风险；建议只挂愿意承担风险的账号、每家少挂几个、重负载分散

### 📊 可观测性
- **请求日志**：完整的请求/响应日志，支持按状态、模型、账号、客户端密钥等维度筛选
- **用量分析**：吞吐、延迟、成功率与成本趋势，Tokens 支持导出 CSV
- **实时路由拓扑**（2026-08 按设计稿重构）：Gateway → Protocol → Pool → Provider 四层画布，带列标题胶囊（`网关 · 1` `协议 · 4` …）；节点卡片按层定制（网关状态块 / 协议状态点+统计 / 路由池策略徽标+资源 / 厂商账号·延迟·成功率）；曲线边路由让每条关系走独立通道不合并；活跃路由为蓝色虚线 + 沿路径流动的箭头；异常分支一键筛选（告警计数胶囊）；悬浮式厂商详情卡（当前路径 / 指标卡 / 关联账号）；底部图例（正常/告警/故障 + 动态流转）；未入池的孤立厂商自动锚回本列，杜绝节点重叠
- **程序日志**：滚动写入 `app.YYYY-MM-DD.log`，保留 7 天，应用内分级查看（ERROR / WARN / INFO）
- **托盘命令卡**：380×720 毛玻璃面板，实时展示网关状态、今日/7 天/30 天/本月 Tokens、流量 Sparkline、每日用量热力图与路由池概览

### 🖥️ 菜单栏与系统托盘
- **单一菜单栏图标**（macOS，按设计稿抠图）：云朵-P（`icons/cloud-p.png`）在浅色和深色菜单栏中都使用**白色实心 Logo**，P 与符号保持透明镂空，与系统其它菜单栏图标一致；右侧保留彩色状态点 + 状态色光晕。状态点按综合严重度着色：绿=在线、蓝=流量活跃、橙=额度告警、红=网关离线。Windows/Linux 托盘保持彩色应用图标 + 右下角状态点不变
- **主文本三选一**（设置中切换，固定展示不轮播）：`今日 240K` / `工具 Claude Code` / `模型 claude-sonnet-4…`，名称超长自动截断；完整信息（状态 · 今日 Tokens · Top1 工具/模型 · 额度剩余百分比）在悬浮提示中
- **左键**打开/关闭托盘命令卡；**右键**呼出原生菜单：
  - 网关运行状态 + **启动 / 关闭网关**（只控制网关，不会退出应用）
  - 实时行：`网关流量 · 最近 5 分钟`、`活跃会话`
  - 已开启路由池清单、**网关 Tokens 统计**（今日/7 天/30 天/本月可切换，含输入/输出/缓存明细）
  - 刷新运行数据 / 显示托盘面板 / 打开 PoolGate / 复制 Agent 配置 / 退出 PoolGate（⌘Q）
  - **局域网访问地址**子菜单（LAN 监听时）：一键复制 `http://<本机 IP>:9800` 入口，同事无需打开设置页即可拿到地址；仅本机监听时给出「去设置开启局域网」提示
- 菜单每 10 秒按实时状态重建，启停网关后立即刷新

### 🧭 双产品模式
PoolGate 使用一个安装包提供两种运行形态，首次启动时会先选择模式：

- **Gateway 完整模式**：完整展示模型供应商、账号池、路由池、请求日志、网关分析，并强制包含 Token Monitor。适合需要本地模型网关，同时希望查看本机 Agent 用量的用户。
- **Monitor 专注模式**：只展示 Token Monitor 总览、工具、模型、会话、趋势、额度与托盘入口；不展示 Gateway 导航、网关启停、账号池或路由配置，Rust 后端也会拒绝启动网关，并停止 Gateway 专属 OAuth 刷新循环。

模式保存在本地设置中，不会删除数据库或凭证。Gateway 用户可在「设置 → 通用 → 产品模式」切换；Monitor 用户可在 Monitor 仪表盘右上角「模式」或托盘「配置」进入模式设置。切换需要自动重启应用，确保托盘、后台任务和权限边界从启动阶段生效。

### 📈 Token Monitor（本地工具用量监控）
- **独立全屏仪表盘**：工具栏「Token 仪表盘」或托盘深链进入，含总览 / 工具 / 模型 / 会话 / 项目 / 趋势 / 额度 / 状态 标签页
- **采集本地编码工具用量**：文件监听 + 轮询采集器，支持 Claude Code、Codex CLI、Cursor、Zed、GitHub Copilot、OpenCode、Cline、Kilo Code、Qwen Code、Kimi、Grok Build 等 20+ 工具，按 `source_fingerprint` 去重
- **数据源隔离**：Token Monitor 只统计本地工具的用量事件（`usage_event`），**不**统计网关流量（`request_logs`）——两者各自独立统计、互不重复
- **额度监控**：账号额度窗口快照（如 Codex 5 小时滚动窗口）、告警阈值设置、系统通知 + 应用内 Toast

### 🎨 桌面体验
- **Tauri 2.0**：原生桌面应用（macOS / Windows / Linux），启动即常驻系统托盘
- **主题**：随系统 / 浅色 / 深色（含托盘卡片），保存后立即生效
- **玻璃效果**：托盘/侧栏工具栏/仪表盘卡片的透明度与毛玻璃模糊可调
- **关闭按钮行为**：隐藏到托盘（默认）或退出程序；支持开机自启
- **账号脱敏显示**：默认掩码展示（`u***@example.com`），可切换完整显示
- **快捷键**：`⌘K` 全局搜索、`⌘1-6` 快速切换页面、`⌘⇧P` 启停网关、`⌘,` 设置、`⌘R` 刷新

## 技术栈

### 前端
- **框架**：React 18 + TypeScript
- **样式**：TailwindCSS 4
- **状态管理**：TanStack Query v5
- **图表**：Recharts
- **拓扑图**：@xyflow/react + ELK.js（懒加载，不进首屏包）
- **构建工具**：Vite 6

### 后端
- **桌面框架**：Tauri 2.0（tray-icon、macOS private API、vibrancy 毛玻璃）
- **Web 框架**：Axum (Rust)
- **异步运行时**：Tokio
- **HTTP 客户端**：Reqwest
- **数据库**：SQLite (rusqlite, bundled)，WAL 模式，版本化迁移（`migrations/001–021`，含 Token Monitor 用量事件 / 会话 / 自定义工具 / 请求日志缓存拆分）
- **凭证安全**：系统钥匙串（keyring），本地凭证库兜底
- **日志**：Tracing（stdout + 滚动文件双通道）

## 快速开始

### 环境要求
- Node.js 20+
- pnpm 11+
- Rust 稳定版（edition 2021）
- 系统依赖：见 [Tauri Prerequisites](https://v2.tauri.app/start/prerequisites/)

### 开发模式

```bash
# 安装前端依赖
pnpm install

# 启动开发服务器（自动拉起 Tauri 桌面应用）
pnpm run tauri dev
```

### 构建发布版

```bash
# 类型检查 + 前端构建 + Tauri 打包
pnpm run tauri build

# 产物位置
# macOS: src-tauri/target/release/bundle/
# Windows: src-tauri/target/release/bundle/msi/
# Linux: src-tauri/target/release/bundle/deb/
```

## 使用说明

### 首次启动与模式选择

新用户首次打开 PoolGate 会先选择 **Gateway 完整模式** 或 **Monitor 专注模式**。Gateway 模式选择后继续进入三步向导；Monitor 模式直接进入 Token Monitor，不会出现账号导入、路由池或网关配置向导。

Gateway 模式的三步向导全部走真实流程、无需 Docker：

1. **导入账号**：选择服务商类型（OpenAI 兼容 / Anthropic / Gemini）、填写 Base URL，粘贴 API Key（每行一个，支持 `名称=密钥`），一键导入并自动创建服务商
2. **创建路由池**：自动按服务商协议建池（如「默认池 · OpenAI」），空模型账号自动从上游拉取模型，并为每个池生成专属接入密钥
3. **复制配置**：每池一份含真实密钥的 Agent 配置一键复制，直接启动网关即可使用

已有账号与路由池的老用户升级后默认视为 Gateway 用户，不会再次看到模式选择或向导；也可以随时点「跳过」。

Monitor 模式下若需要完整网关能力，请通过「模式」设置切换到 Gateway，确认后应用会重启。

### 让 Agent 走 PoolGate

在设置页「工具配置」可一键复制，或手动配置：

```bash
# Claude Code
export ANTHROPIC_BASE_URL=http://127.0.0.1:9800

# Codex CLI
export OPENAI_BASE_URL=http://127.0.0.1:9800

# 其他 OpenAI 兼容客户端
export OPENAI_API_BASE=http://127.0.0.1:9800
```

若设置了网关访问密钥，客户端还需携带鉴权头（`Authorization: Bearer` / `x-api-key` / `x-goog-api-key`）；`/health` 探针始终免鉴权。

### 局域网共享（小团队多人使用）

面向「小团队共享 API Key 池」的辅助场景（主场景为个人使用，见「产品定位」）：

1. 设置页「代理 → 监听地址」选择 **局域网共享**（需先设置网关访问密钥，未设置时会被拒绝）
2. 设置页或托盘右键菜单会列出本机局域网 IP（如 `192.168.1.8`）；让同事把各自 Agent 工具的 Base URL 指向 `http://<该 IP>:9800`
3. 同事请求时携带网关访问密钥（`Authorization: Bearer <key>`）即可访问路由池；`/health` 始终免鉴权

> ⛔ **共享边界**：仅 **API Key / 上游密钥类**账号可以共享。OAuth 订阅账号（Claude / Gemini / Copilot / Codex / Grok 等）受上游 ToS 限制，仅限本机本人使用（导入时也会提示），不要加入共享路由池（路由层按请求来源隔离订阅账号的能力在规划中，见「下一步任务」）。

### 数据存储

- **数据库**：`{应用数据目录}/.poolgate/gateway.db`（SQLite，WAL 模式）
- **应用标识迁移**：从旧版 `com.poolgate.app` 升级到当前 `com.poolgate.desktop` 时，需在关闭应用后将旧目录中的 `gateway.db` 与 `credentials.vault.json` 迁移到当前目录。Gateway / Monitor 模式切换只修改模式设置，不会删除供应商、账号或路由池数据。
- **凭证库**：`{应用数据目录}/.poolgate/credentials.vault.json`（敏感信息优先存系统钥匙串）
- **程序日志**：`{应用数据目录}/.poolgate/logs/app.YYYY-MM-DD.log`（保留 7 天）

## 项目结构

```
poolgate/
├── src/                          # React 前端
│   ├── pages/                    # 页面（每页一个目录）
│   │   ├── Dashboard/            # 指挥中心（实时路由拓扑与网关态势）
│   │   ├── AccountPool/          # 模型供应商（含 ImportCenter / OAuthLogin）
│   │   ├── AgentGroups/          # 路由池
│   │   ├── Logs/                 # 请求日志
│   │   ├── Analytics/            # 用量分析
│   │   ├── Settings/             # 系统设置（含产品模式切换）
│   │   ├── Providers/            # 服务商配置
│   │   ├── Wizard/               # 首启向导（导入账号 → 建池 → 复制配置）
│   │   ├── AppLogs.tsx           # 程序日志查看
│   │   ├── TopologyFullscreen.tsx# 拓扑全屏
│   │   └── TokenMonitor/         # Token Monitor 仪表盘（总览/工具/模型/会话/项目/趋势/额度/状态）
│   ├── components/               # 共享组件
│   │   ├── ui/                   # 基础 UI（Card/Badge/Toast/主题/搜索…）
│   │   ├── topology/             # 拓扑图组件
│   │   ├── tray/                 # 托盘面板组件
│   │   └── token-monitor/        # Token Monitor 组件
│   ├── hooks/                    # Tauri 命令 Hook
│   └── lib/                      # 命令与工具封装
├── src-tauri/                    # Rust 后端
│   ├── migrations/               # SQLite 版本化迁移（001–021）
│   └── src/
│       ├── db/                   # 数据库层（schema/accounts/providers/groups/logs/settings/…）
│       ├── proxy/                # Axum 网关（server/router/auth/stream/concurrency/health）
│       │   └── protocol/         # openai/anthropic/responses/gemini 协议适配与转换
│       ├── services/             # 业务服务（导入/导出/OAuth/健康检查/托盘/菜单栏/…）
│       ├── commands/             # Tauri IPC 命令
│       └── token_monitor/        # Token Monitor（collector/ 采集器、quota/ 额度监控）
└── public/                       # 静态资源
```

## 开发指南

### 代码规范

- **前端**：TypeScript 严格模式，组件按页面/功能目录组织
- **后端**：`cargo fmt` + `cargo clippy`，遵循 Rust 惯用法
- **数据库**：所有表结构变更走 `migrations/` 增量迁移（`db/schema.rs` 中递增 `user_version`）

### 测试与检查

```bash
# 后端单元测试（含菜单栏/托盘/路由/数据库迁移等）
cd src-tauri && cargo test

# 后端类型检查
cd src-tauri && cargo check

# 前端类型检查
npx tsc --noEmit
```

## 常见问题

### Q: 为什么选择 Tauri 而不是 Electron？
A: Tauri 使用系统 WebView，打包体积小、内存占用低、原生性能更好，且能直接调用系统钥匙串与原生菜单栏/托盘能力。

### Q: 支持哪些模型提供商？
A: 支持所有 OpenAI / Anthropic / Gemini 兼容 API，包括 OpenAI、Anthropic、Google、Azure、国内各大模型厂商，以及通过自定义 HTTP 连接器接入的任何服务；还支持 Claude / Gemini / Copilot / Grok / Antigravity 等 OAuth 订阅账号。

### Q: 账号凭证安全吗？
A: 凭证优先存储在系统钥匙串（Keyring）中，本地凭证库仅作兜底，不会上传到任何外部服务。

### Q: 可以把账号池共享给同事或小团队吗？
A: **API Key 类账号可以**：设置页开启「局域网共享」（需先设置网关访问密钥），同事用 `http://<你的局域网 IP>:9800` + 各自的客户端密钥接入即可。**OAuth 订阅账号不行**：Claude / Gemini / Copilot / Codex / Grok 等订阅账号受上游 ToS 限制，仅限本机本人使用，不可共享。

### Q: 菜单栏「关闭网关」会退出应用吗？
A: 不会。它只停止网关代理进程，PoolGate 继续在托盘运行；退出应用请使用菜单底部的「退出 PoolGate」。

### Q: 网关流量和 Token Monitor 的数字会重复吗？
A: 不会。两者数据源分离：菜单栏/托盘/仪表盘的 Tokens 来自网关请求日志（`request_logs`，只统计走 PoolGate 的请求）；Token Monitor 只统计本地工具的用量事件（`usage_event`）。

## 当前状态与下一步（2026-08）

### 已完成

- **首启三步向导**（2026-08-18）：导入账号（粘贴 API Key 自动建服务商）→ 自动按协议建池（空模型账号自动从上游拉模型 + 生成池专属密钥）→ 复制含真实密钥的 Agent 配置；`pg.onboarded` 记录完成状态，已有账号+池的老用户自动跳过
- **监听地址切换（局域网共享）**（2026-08-18）：设置页「仅本机 / 局域网共享」切换，网关按 `127.0.0.1` / `0.0.0.0` 绑定并自动重启；局域网模式强制要求网关访问密钥（未设密钥不开放、局域网中不可清除密钥、鉴权中间件兜底拒绝开放网关）；设置页实时列出本机局域网 IP 供同事配置；托盘状态行按监听模式显示对应地址
- **托盘局域网地址入口**（2026-08-18）：右键菜单新增「局域网访问地址」子菜单——LAN 监听时逐条复制 `http://<本机 IP>:9800`，同事无需打开设置页即可拿到入口；仅本机监听时提示去设置开启
- **实时路由拓扑重构**：对齐 2026-08-02 设计稿（四层画布 + 悬浮详情卡 + 动态流转箭头），并在真实数据下验证布局（22 个厂商、9 个入池、13 个未入池）
- **macOS 菜单栏图标**：云朵-P 抠图，浅色和深色菜单栏都使用白色实心 Logo，P 与符号透明镂空；彩色状态点 + 状态色光晕保留
- **Token Monitor**：独立仪表盘（总览/工具/模型/会话/项目/趋势/额度/状态）、20+ 本地工具采集、去重与额度监控
- **托盘命令卡**：毛玻璃面板、实时 Tokens、流量 Sparkline、每日热力图、路由池概览
- **缓存符对齐**（2026-08-16）：规范 usage 五元组 + 流式/非流式全路径解析 + 跨协议词表回填 + `request_logs` 读/写缓存拆分（迁移 021）
- **OAuth 风控防护**（2026-08-16）：官方客户端画像（Codex/Claude/Copilot/Gemini）、Claude Code 适配器接入 `/v1/messages` 代理路径（系统前缀 + 指纹 + 401 刷新重试）、会话粘性路由、订阅账号行为节流、封控信号识别与自动停用
- **测试修复**：`proxy::runtime` / `services::import` 两个既有失败测试已修复（runtime 按账号聚合为拓扑账号层的有意设计，cockpit base_url 原样存储由路由时归一），`cargo test` 全绿（295 通过）

### 已知问题

- 拓扑快照目前**只包含已入池的厂商**（后端按池引用过滤，13 个未入池厂商不显示）；截图曾出现厂商卡片与网关节点重叠（疑为旧构建，需新构建复验）
- 拓扑厂商详情依赖真实桌面端 Tauri IPC；开发验证应使用桌面端构建，不再随源码发布浏览器 mock 预览页
- 客户端画像的版本字符串（codex_cli_rs / claude-cli / GeminiCLI 版本号）需要随官方客户端升级定期维护，否则可能被上游按版本黑名单拒绝
- 封控识别的特征文案清单是启发式的，新话术需要持续补充（`detect_ban_signal`）

### 下一步任务

1. 用真实数据 + 新构建复验拓扑布局（重点：未入池厂商、网关列重叠），必要时修复布局逻辑
2. 厂商卡片告警徽标接入真实告警数据（`alerts` 表，现已包含封控 critical 告警）与点击联动
3. 路由池节点展示其实际支持的协议徽标（unified / both / chat / responses…），与决策链心智模型对齐
4. 「未入池厂商」可视化口径：是否在拓扑中显示、如何进入详情卡
5. 桌面端整体回归：托盘命令卡 / Token Monitor / 菜单栏在最新构建下的联调（重点验证：Claude OAuth 走网关的系统前缀注入、Codex 指纹、会话粘性下的故障转移）
6. 可选增强：Logs / Analytics 页面展示 `cache_read_tokens` / `cache_write_tokens` 拆分；节流参数做成设置项
7. 局域网共享边界：路由层按「请求来源」区分本机/远端，订阅类（OAuth）账号只允许本机请求路由，防止同事通过局域网消耗个人订阅额度

## 许可证

MIT License

## 致谢

- [Tauri](https://tauri.app/) - 构建更小、更快、更安全的桌面应用
- [Axum](https://github.com/tokio-rs/axum) - Ergonomic and modular web framework
- [React](https://react.dev/) - 用于构建用户界面的 JavaScript 库
- [TailwindCSS](https://tailwindcss.com/) - Rapidly build modern websites
