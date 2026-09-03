# PoolGate 开源发布处理清单

> 这份清单记录需要项目维护者、账号持有人或法律顾问手动完成的事项。可以自动化的仓库基础设施已由代码助手补充，但不会代替账号配置、签名、公证、第三方授权或最终发布决策。

## 当前状态

- [x] 已有 MIT `LICENSE`

- [x] 已有较完整的 `README.md`

- [x] 已补充 `SECURITY.md`

- [x] 已补充 `CONTRIBUTING.md`

- [x] 已补充 `CODE_OF_CONDUCT.md`

- [x] 已补充 `CHANGELOG.md`

- [x] 已补充 `.github/workflows/ci.yml`

- [x] 已补充 Pull Request 模板

- [x] 已补充 `package.json` 与 Cargo 仓库元数据

- [x] 工作区改动已提交（commit 850ca5c），工作树干净

- [ ] CI 尚未在 GitHub 上实际运行并变绿

- [x] Rust `fmt --check` 已在本地通过

- [ ] 严格 Clippy 尚未通过；CI 暂以非阻断 Clippy 告警模式运行，严格脚本保留为 `pnpm run lint:rust:strict`

## A. 必须由维护者处理的仓库内容

### 发布快照与文件清理

- [x] git status 干净，所有修改文件已审查并纳入提交

- [x] 未跟踪文件已清理；docs/upgrade-checklist.html 已纳入

- [x] Token Monitor（detect/model/service\_collect）、迁移、测试和 UI 变更已确认纳入

- [x] 删除临时预览 HTML、本地生成图片和无关实验文件；设计文档已整理到 `docs/design/`

- [x] .DS\_Store 已在 .gitignore 中且未被追踪；\*.db / \*.db-shm / \*.db-wal 已忽略

- [x] 源码中 sk-/ghp\_/AKIA 等模式均为 UI 占位符/正则示例，无真实密钥泄露

- [ ] 对当前工作树和完整 Git 历史运行 secret scan，例如 Gitleaks

- [x] 确认 `src-tauri/tests` 等新测试目录确实被 Cargo 执行，而不是只存在于工作区

- [x] 干净提交已形成（850ca5c），等待 release branch 创建

### 包管理和版本

- [x] 决定 pnpm 为唯一前端包管理器

- [x] 已将 `pnpm-lock.yaml`、`pnpm-workspace.yaml` 纳入提交范围

- [x] 已删除并忽略 `package-lock.json`，避免两套 lockfile 产生歧义

- [ ] 缺少 rust-toolchain.toml、.node-version / .nvmrc；建议增加版本锁定文件

- [ ] 统一 `package.json`、Cargo、Tauri 和 Git tag 的版本号

- [ ] 统一 LICENSE、Cargo authors、README 中的版权主体

## B. 必须由维护者配置的 GitHub 内容

- [ ] 在 GitHub 仓库设置中配置私有漏洞报告或安全联系邮箱

- [ ] 检查 `SECURITY.md` 中的报告渠道是否真实可用

- [ ] 配置 Issues、Discussions 和项目标签

- [ ] 为 `main` 设置 branch protection

- [ ] 要求 CI 通过后才能合并

- [ ] 配置 Dependabot 或 Renovate

- [ ] 配置 Secret Scanning / Push Protection（仓库权限允许时）

- [ ] 在 GitHub Release 中维护安装说明和已知问题

- [ ] 决定是否允许外部贡献者直接提交 PR

## C. 必须由维护者或法律顾问处理的风险

### OAuth、上游 API 和 ToS

- [ ] 逐个审查 Claude、Codex、Gemini、Copilot、Grok、Antigravity 等 OAuth 适配器

- [ ] 确认每个 OAuth client ID 的使用授权和再分发边界

- [ ] 确认 OAuth scope、redirect port 和第三方域名已在文档中披露

- [ ] 审查私有或非公开 API endpoint 的使用风险

- [ ] 审查通过自动化网关使用订阅账号是否违反上游 ToS

- [ ] 决定高风险 OAuth 适配器是否改为实验性、可选模块或暂不发布

- [ ] 不使用上游商标暗示官方合作或官方客户端身份

- [ ] 为 README 增加清晰的第三方服务免责声明

### 隐私和数据处理

- [ ] 确认 Token Monitor 扫描的本地目录和文件格式

- [ ] 确认请求日志、程序日志和 Token Monitor 数据的保留期限

- [ ] 确认用户删除数据库、日志、凭证库和采集数据的方法

- [ ] 确认所有外部网络请求的域名、用途和触发条件

- [ ] 确认诊断导出不会包含密钥、token、cookie、完整 prompt 或个人信息

- [ ] 准备公开的隐私说明

### 桌面端权限和安全

- [ ] 审查 Tauri capability 中的文件读写 scope

- [ ] 审查剪贴板读写、Shell open、通知、自动启动和进程退出权限

- [ ] 为生产配置设置明确 CSP，不长期使用 `csp: null`

- [ ] 对自定义 Base URL、Proxy URL 和自定义 Header 做 SSRF / 内网访问威胁建模

- [ ] 验证本机开放模式和 LAN 模式的边界行为

- [ ] 验证日志脱敏覆盖所有 provider 错误和认证头格式

## D. 必须由维护者完成的跨平台发布工作

- [ ] macOS Apple Silicon 构建并验证

- [ ] macOS Intel 构建并验证（如果继续支持）

- [ ] Windows x64 构建并验证

- [ ] Windows ARM64 构建并验证（如果继续支持）

- [ ] Linux x64 构建并验证

- [ ] Linux ARM64 构建并验证（如果继续支持）

- [ ] 配置 macOS Developer ID 签名

- [ ] 完成 macOS notarization

- [ ] 确认 `macOSPrivateApi` 对 notarization 和 Mac App Store 的影响

- [ ] 配置 Windows 代码签名证书

- [ ] 生成每个 artifact 的 SHA256 校验值

- [ ] 生成并发布 SBOM / 依赖许可证清单

- [ ] 决定是否启用 Tauri updater，并配置 updater 签名密钥

- [ ] 创建第一个经过审查的 tag，例如 `v0.1.0`

- [ ] 编写 Release Notes、安装说明、升级说明和回滚说明

## E. 质量门禁

- [ ] `pnpm install --frozen-lockfile`

- [ ] `pnpm run typecheck`

- [ ] `pnpm run build`

- [ ] `pnpm run check:rust`

- [ ] `pnpm run test:rust`

- [ ] `pnpm run fmt:check`

- [ ] `pnpm run lint:rust`

- [ ] `git diff --check`

- [ ] 运行依赖漏洞扫描

- [ ] 运行 secret scan

- [ ] 完成至少一次全新机器安装测试

- [ ] 完成至少一次升级旧版本数据库和凭证库的测试

- [ ] 完成 Gateway 模式和 Monitor 模式的启动回归

- [ ] 完成托盘、菜单栏、OAuth 回调和 LAN 模式回归

## 推荐发布门槛

在以下条件全部满足前，不建议发布“稳定版”二进制：

1. 发布分支干净，所有功能代码、迁移和测试都已纳入审查；
2. GitHub CI 全绿；
3. secret、依赖许可证和第三方 ToS 审查完成；
4. macOS / Windows / Linux 构建、签名和安装流程完成；
5. 隐私、安全和已知限制文档已公开；
6. 至少一名非作者用户完成全新安装和基本功能验证。

