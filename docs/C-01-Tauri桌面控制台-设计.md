# C-01 Tauri 桌面控制台：设计文档

文档状态：`verified`。本轮已完成 Tauri 桌面迁移；领域逻辑仍由 Rust 模块负责。

## 1. 架构决策

- `src-tauri/src/main.rs` 是唯一 Tauri 应用入口，不再解析 CLI 参数；只读观察 CLI 是独立 crate `crates/observer-cli`（二进制 `personal-taoli-observer`），与桌面共享 `crates/core`。
- Rust command 直接编排现有 `config`、`account`、`venues`、`local_book`、`scan`、`archive` 和 PAPER 模块。
- Vue + Element Plus + TypeScript 是唯一操作界面；不引入前端状态管理库，页面状态足够小。
- Tauri 使用静态 `ui/` 前端资源；Vue 通过 `@tauri-apps/api/core` 的 `invoke` 调用 command。
- Rust 返回 JSON 可序列化 DTO；前端不接触数据库连接串、环境变量或凭证。

## 2. Command 契约

| command | 输入 | 输出 | 副作用 |
|---|---|---|---|
| `desktop_status` | 无 | 安全模式与版本 | 无 |
| `load_config_summary` | `config_path: Option<String>` | `ConfigSummary` | 读取配置 |
| `account_status` | `config_path: Option<String>` | 两所 `AccountData` | 只读账户元数据请求；无订单 |
| `observe_once` | `config_path`, `archive_path` | `ObserveResult` | 公共行情读取，追加一条归档记录 |
| `replay_observations` | `path: String` | `ShadowReport` | 读取归档与缺口文件 |
| `run_paper_smoke` | `kind: String` | B-01/B-02/B-03 报告 | 仅 PostgreSQL 测试事实；外部订单调用为 0 |

所有 command 都返回统一 `ApiResponse<T>`。成功响应为 `{ success: true, data, error: null }`；失败响应为 `{ success: false, data: null, error: { code, message, retryable } }`。前端统一解包并通过 `ElMessage.error` 展示错误码和上下文。

## 3. 实时观测流程

1. 加载并校验配置。
2. 创建 HTTPS 客户端和 Binance/Bybit 公共行情适配器。
3. 并行加载两所品种规格并校验统一交易对及精度。
4. 启动两条公共订单簿 feed，等待两者同时 `VALID`。
5. 读取只读账户元数据，构建准入拒绝列表。
6. 使用 `scan_pair` 按 `Decimal` 计算两个方向，生成 `DecisionEvent`。
7. 通过现有有界归档 writer 写入一次事件；归档失败返回错误，不把未归档结果报告为完成。
8. 返回报告和不包含盘口内容的 feed 状态摘要。

## 4. 状态与失败关闭

- `SYNCING`、`STALE`、`INVALID` 不生成可执行机会；等待超时直接返回错误。
- 凭证缺失、地区/账户资格未确认、费率回退或费率过期仍可显示观察结果，但机会保持拒绝。
- 任意外部请求失败保留 Rust 错误上下文；不在前端重试或猜测结果。
- PAPER 报告的计划、人工升级和完成状态按现有数据库事实语义展示。

## 5. 前端页面结构

- 顶栏：应用名称、安全模式、刷新配置按钮。
- 概览卡：品种、数量、深度、归档路径、两个 WebSocket URL。
- 操作区：账户状态、一次实时观测、归档回放、三类 PAPER 烟测按钮；运行中禁用重复操作。
- 账户表：交易所、权限、费率来源/值、有效期、资格与拒绝原因。
- 行情与机会表：feed 状态、方向、买卖场所、VWAP、毛利、费用、延迟、净收益、准入 bps、结果。
- 回放/PAPER 报告区：结构化 JSON 细节和关键指标。
- 全局错误提示：`ElMessage.error`，不吞掉 command 错误。

## 6. 删除与替换路径

- 删除根包中的 CLI 专用入口；`src-tauri/src/main.rs` 是唯一桌面二进制入口，`src-tauri/src/lib.rs` 只负责注册 command。桌面不再提供 CLI 形态；只读观察/回放/重连烟测命令行已迁移到 `crates/observer-cli`（`personal-taoli-observer`），引用 `crates/core` 的 `observer`、`venues`、`scan`、`archive` 与 `account`。
- `src-tauri/src/commands/` 按职责拆分为 `system`、`account`、`observation`、`archive`、`paper`、`dto` 和 `support`，命令层不承载行情扫描和归档编排细节。
- `crates/core` 是唯一领域与基础设施 crate；`observer` 负责一次观测编排，`scan` 负责机会计算，`venues` 负责交易所适配，PAPER 模块负责持久化安全核心。
- 前端源码统一位于根目录 `src/`，Tauri 只加载构建产物 `ui/`；前端通过 `@tauri-apps/api` 调用稳定的 command DTO，不直接耦合 Rust 内部模块。
- 配置样例保留在根目录 `config/observer.toml`，数据库迁移归属 `crates/core/migrations/`，测试 fixture 归属 `crates/core/tests/fixtures/`。

## 7. 开发、构建与生成物

- 安装前端依赖：`npm ci`。
- 启动桌面开发环境：`npm run tauri dev`。该命令先启动固定在 `http://localhost:1420` 的 Vite 服务，再启动 Tauri Rust 应用。
- 使用 PAPER 烟测前，启动本地 PostgreSQL，并在项目根目录 `.env` 设置 `TAOLI_DATABASE_URL=postgresql://taoli:taoli@127.0.0.1:55432/taoli`；当前 Docker 容器通过宿主机 `127.0.0.1:55432` 暴露 PostgreSQL。Tauri 启动时自动读取 `.env`，shell 中已存在的环境变量优先。桌面端不会从前端输入、保存或展示该连接串；未配置时 PAPER command 按安全策略返回 `PAPER_ERROR`，不伪造成功。
- 单独构建前端：`npm run build`；产物写入根目录 `ui/`，该目录除 `.gitkeep` 外不进入版本控制。
- Rust 检查与测试：`cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace`。
- `src-tauri/gen/` 是 Tauri 自动生成的 schema 目录，不提交；`src-tauri/icons/icon.png` 是 Tauri 打包所需资源，必须保留。
