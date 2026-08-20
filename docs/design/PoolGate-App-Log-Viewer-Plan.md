# PoolGate 程序日志落盘 + UI 查看器实现方案

> 目标：解决程序日志（tracing）仅输出 stdout、GUI 应用不可见导致的排查困难问题。
> 范围：① tracing 落盘（按天轮转文件）；② 侧边栏新增「程序日志」页面（分页查看 / 关键词过滤 / 刷新 / 自动刷新 / 导出）。
> 不改变：request_logs（请求业务日志）、gateway_logs（网关失败事件）现有逻辑；二者与文件日志互补，本方案不合并。

---

## 1. 总体架构

```
Rust 侧                                        UI 侧
┌─────────────────────────────┐              ┌──────────────────────────────┐
│ tracing::info!/warn!/error! │              │ App.tsx navItems 新增入口     │
│        │                    │              │  "程序日志" (observability)   │
│        ▼                    │              │        │                     │
│ tracing_subscriber registry │  invoke      │        ▼                     │
│   ├─ stdout layer (dev)     │  readAppLogs │  pages/AppLogs.tsx           │
│   └─ file layer ────────────┼─────────────▶│   - 日志列表（mono + 级别着色）│
│        │                    │  getAppLog   │   - 关键词过滤 / 级别高亮     │
│        ▼                    │  Info        │   - 手动刷新 + 自动刷新间隔   │
│ .poolgate/logs/app.YYYY-MM-DD.log          │   - 文件路径 / 大小 / 导出    │
└─────────────────────────────┘              └──────────────────────────────┘
```

- 日志文件：`{app_data_dir}/.poolgate/logs/app.YYYY-MM-DD.log`（与 gateway.db 同目录族，符合现有 `.poolgate` 目录惯例）
- 生产环境仅写文件（无终端可看）；开发环境 stdout + 文件双写

---

## 2. Part 1 — tracing 落盘

### 2.1 初始化改造（`src-tauri/src/lib.rs`，`run()` 开头）

现状（约 lib.rs:47）：
```rust
tracing_subscriber::fmt().with_target(true).init();
```

改为 registry + 双 layer（stdout + rolling file），`tracing-appender = "0.2"` 已在 Cargo.toml（第 39 行），无需新增依赖：

```rust
pub fn run() {
    // 日志目录：{app_data}/.poolgate/logs/（与 gateway.db 同目录族）
    let log_dir = ...; // 在 setup 之前无法直接拿 app_data_dir，
                       // 故在 Builder::setup 回调中初始化（见 2.2）
    ...
}
```

**关键点：初始化时机**。`app_data_dir()` 需要 AppHandle，而当前 `run()` 里在 `tauri::Builder::default()` 之前就 `tracing_subscriber::init()`。两种做法：

- **方案 A（推荐）**：把日志初始化移入 `.setup(|app| { init_logging(&app.path().app_data_dir()?.join(".poolgate/logs")); Ok(()) })`。setup 在监听事件前执行，代理/刷新循环（在 `setup` 之后 spawn）产生的日志都能落盘；`run()` 里不再调用 `tracing_subscriber::init()`。**注意：setup 之前若有 tracing 输出会丢失（可接受，启动早期无关键日志）。**
- 方案 B：在 `run()` 里用 `dirs::data_dir()`/`HOME` 推算路径（与 tauri 的 app_data_dir 可能不一致）——不推荐。

实现（方案 A）：

```rust
fn init_logging(log_dir: &std::path::Path) {
    std::fs::create_dir_all(log_dir).ok();
    let file_appender = tracing_appender::rolling::daily(log_dir, "app.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    // guard 必须存活到进程结束，否则缓冲日志丢失 —— 显式泄漏。
    std::mem::forget(guard);

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_writer(std::io::stdout);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(false) // 文件不写 ANSI 颜色
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .with(tracing_subscriber::EnvFilter::new("info"))
        .init();
}
```

### 2.2 轮转与保留策略

| 项 | 策略 |
|---|---|
| 轮转 | `rolling::daily`（每天一个文件 `app.YYYY-MM-DD.log`） |
| 保留 | 启动时清理 7 天前的 `app.*.log`（`init_logging` 内扫目录删除） |
| 级别 | `EnvFilter::new("info")`（info/warn/error 全落盘；debug 不写，避免噪音） |
| 写入 | `non_blocking` 异步写入，不影响请求路径 |

### 2.3 脱敏与安全

- 沿用既有 `services::redaction::redact_sensitive`：所有上游错误/含 key 的 tracing 输出已经脱敏（上游错误、OAuth 请求失败等）。
- 鉴权中间件不打 key 明文（只记录 `fingerprint`：名称+后四位）。
- 日志文件权限：随 app_data_dir（当前 `credentials.vault.json` 为 600，目录 700），落盘文件默认继承目录权限。
- UI 展示层再套一层正则脱敏（见 3.3），双保险：`Bearer\s+\S+` / `x-api-key:\s*\S+` / `pg_live_\w+` 替换为 `***`。

### 2.4 改动清单（Part 1）

| 文件 | 改动 |
|---|---|
| `src-tauri/src/lib.rs` | `run()` 移除 `tracing_subscriber::fmt().init()`；新增 `init_logging()`；在 `.setup()` 回调中调用；注册 `log_commands` |
| `src-tauri/src/services/app_log.rs`（新建） | 日志文件定位、tail 读取、脱敏、清理旧文件（见 Part 2） |

---

## 3. Part 2 — UI 日志查看器

### 3.1 后端 command（新建 `src-tauri/src/commands/log_commands.rs`）

```rust
#[derive(serde::Serialize)]
pub struct AppLogPage {
    pub lines: Vec<String>,      // 倒序（最新在前），已脱敏
    pub total: usize,            // 过滤后总行数
    pub page: u32,
    pub page_size: u32,
    pub file_path: String,
    pub file_size: u64,
}

#[tauri::command]
pub fn read_app_logs(
    page: Option<u32>,
    page_size: Option<u32>,
    keyword: Option<String>,
) -> Result<AppLogPage, String>;

#[tauri::command]
pub fn get_app_log_info() -> Result<serde_json::Value, String>; // 路径/大小/行数/今天文件名
```

**读取算法（tail + 过滤 + 倒序分页）**：
1. 定位最新文件：`.poolgate/logs/` 下 `app.*.log` 按 mtime 最新者（rolling::daily 当天文件）。
2. 读全文（轮转后单文件 ≤ ~10MB，内存可接受；若 > 50MB 降级为只读尾部 50MB）。
3. 按 `\n` split；`keyword` 非空则过滤（`contains`，大小写不敏感）。
4. 过滤后数组**倒序**（最新在前），按 `(page-1)*page_size` 切片。
5. 每行执行正则脱敏（3.3）后返回。

注册：`lib.rs` 的 `invoke_handler` 追加 `commands::log_commands::read_app_logs`、`get_app_log_info`。

### 3.2 前端接入

**`src/App.tsx`**：
- `Page` 联合类型加 `"applog"`
- `lazy(() => import("@/pages/AppLogs"))`
- `navItems` 加一项（observability 分区，排在「请求流」之后）：
  ```ts
  { id: "applog", label: "程序日志", shortLabel: "程序日志", description: "网关运行日志与排查", icon: ScrollText, section: "observability" },
  ```
- `renderPage` 的 `pages` 映射加 `applog: AppLogs`

**`src/lib/tauri-commands.ts`**：新增 `AppLogPage` 类型 + `readAppLogs` / `getAppLogInfo`（沿用 `invoke` 封装模式）。

**`src/hooks/use-tauri.ts`**：新增 `useAppLogs(query, refetchInterval?)`（复用 useLogs 的 interval 模式，queryKey 不含 interval）。

**新建 `src/pages/AppLogs.tsx`**（复用请求日志页的视觉与交互模式）：
- 顶部：标题「程序日志」+ 说明（文件路径、大小）+ 导出按钮（复制路径 / 打开日志目录，走 `tauri_plugin_shell` 或 dialog）
- 工具栏：
  - 关键词输入框（防抖 300ms）
  - 级别过滤（全部 / ERROR / WARN / INFO，前端按行前缀匹配）
  - 手动「刷新」按钮（refetch + 旋转态）
  - 「自动刷新」间隔选择器：关闭/3s/5s/10s/30s/60s（localStorage 持久化，与请求日志页同 key 族 `pg_applog_auto_refresh`）
- 列表：mono 字体、行号、按行内容着色——含 `ERROR`/`panic` 红、`WARN` 橙、`request_id=` 高亮模型名；点击行可复制（可选）
- 分页：上一页/下一页 + 页码（复用请求日志页模式）

### 3.3 展示层脱敏正则（后端统一执行）

```
Bearer\s+[A-Za-z0-9._-]{8,}   → Bearer ***
(x-api-key|Authorization|api[_-]?key)[:=]\s*\S+  → $1: ***
pg_live_[A-Za-z0-9]+          → pg_live_***
at-[A-Za-z0-9]+               → at-***
```

### 3.4 改动清单（Part 2）

| 文件 | 改动 |
|---|---|
| `src-tauri/src/services/app_log.rs`（新建） | 日志目录定位、最新文件选择、tail 读取、正则脱敏、7 天清理 |
| `src-tauri/src/commands/log_commands.rs`（新建） | `read_app_logs` / `get_app_log_info` |
| `src-tauri/src/lib.rs` | `mod` 声明 + `invoke_handler` 注册两个 command |
| `src/App.tsx` | navItems / Page 类型 / lazy / renderPage |
| `src/lib/tauri-commands.ts` | 类型 + 两个 API 封装 |
| `src/hooks/use-tauri.ts` | `useAppLogs` hook |
| `src/pages/AppLogs.tsx`（新建） | 日志查看页 |

---

## 4. 数据流示例

```
curl 触发一次失败请求
  → server.rs: tracing::info!("request_id={} ... -> 503 (...ms)")
  → non_blocking writer 异步写 app.2026-08-04.log
  → UI「程序日志」页 read_app_logs(page=1, keyword="503")
  → 返回最新 50 行（脱敏后）
  → 列表高亮显示该行 → 用户直接定位失败原因
```

---

## 5. 验证方案

| 项 | 方法 |
|---|---|
| Rust 编译 | `cargo check` + `cargo test --lib` |
| 落盘 | `cargo run`（dev）后触发一次请求，检查 `.poolgate/logs/app.*.log` 出现 `request_id=...` 行 |
| 轮转 | 手工改系统日期或调 `rolling::daily` 参数验证文件名；清理逻辑单测 |
| 脱敏 | 单测：构造含 `Bearer xxx` / `pg_live_xxx` 的行 → 断言替换 |
| UI | dev 运行：导航「程序日志」→ 列表/过滤/刷新/自动刷新/分页/导出路径显示 |
| 回归 | 请求日志页、网关启动、托盘行为不受影响（stdout layer 保留 dev 输出） |

---

## 6. 实施顺序

1. `services/app_log.rs` + `commands/log_commands.rs`（后端读取/脱敏/清理）→ cargo test
2. `lib.rs`：`init_logging` + setup 回调 + command 注册 → 手动验证落盘
3. 前端：tauri-commands / use-tauri / AppLogs 页面 / App.tsx 导航 → tsc + dev 验证
4. 全量回归（Rust 测试 + tsc + 手动请求链路）

---

## 7. 风险与取舍

- **日志量**：info 级每请求 1 行 + 后台循环（Token 刷新等）低频行，日文件约 1–10MB，可接受；7 天自动清理。
- **setup 前日志丢失**：启动早期无关键日志（窗口创建、插件加载不落盘），可接受；如需可把初始化提前到 `run()` 用 `HOME` 推算路径（方案 B，不推荐）。
- **guard 泄漏**：`non_blocking` guard 用 `mem::forget` 保持存活（进程生命周期内有效，属标准做法）。
- **不做的事**：不合并 gateway_logs / request_logs；不做日志级别动态调整（后续可在设置页加级别选择，预留 `EnvFilter` 可替换）；不做远程日志。
