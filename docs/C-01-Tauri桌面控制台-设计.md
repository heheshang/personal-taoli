# C-01 Tauri 桌面控制台：设计文档

文档状态：`verified`。仓库已收敛为纯 Rust + Tauri 桌面形态：工作区仅含 `crates/core` 领域库与 `src-tauri` 桌面应用两个成员，无任何 CLI 二进制。

## 1. 架构决策

- 工作区成员只有 `crates/core` 与 `src-tauri`；`src-tauri/src/main.rs` 是唯一可执行入口，全应用不再解析 CLI 参数。此前作为第三个工作区成员的独立 CLI 二进制已整体删除，其能力全部迁入核心库与 Tauri command 面。
- 会话型能力（连续观察的启动/停止/状态、退出时冲刷归档）由 `SessionController` 承载：以 `tauri::manage` 注册为应用状态，命令层与窗口、监督进程共享同一实例。任务取消走一次性 shutdown 通道并等待归档冲刷完成（30 秒上限），不硬杀进程。
- 支持环境变量自动启动：检测到 `TAOLI_OBSERVER_AUTOSTART`（任意非空）即后台启动一次只读连续观察；`TAOLI_OBSERVER_ARCHIVE` 可覆盖归档路径，缺省取配置。桌面进程被 SIGTERM/关窗即冲刷归档后退出。
- Rust command 直接编排现有 `config`、`account`、`venues`、`local_book`、`scan`、`archive`、`observer` 和 PAPER 模块；命令层不承载行情扫描与归档编排细节。
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
| `run_accounting_control_smoke` | `kind: String` | B-04 报告 | 仅 PostgreSQL 测试事实；外部订单调用为 0 |
| `start_continuous_observation` | `config_path`, `archive_path`（均可选） | `ContinuousStatus` | 启动只读连续观察会话；重复启动返回 `InvalidRequest` |
| `stop_continuous_observation` | 无 | `ContinuousStatus` | 停止会话并冲刷归档；空闲时返回 `InvalidRequest` |
| `continuous_observation_status` | 无 | `ContinuousStatus` | 无 |
| `run_reconnect_smoke` | `config_path: Option<String>` | `ReconnectSmokeResult` | 对两所公共 feed 各强制一次重连，等待恢复；无订单 |

`ContinuousStatus` 含 `running`、`archive_path`、`gap_path`、`started_at_ms`、`last_report_at_ms`、`last_report` 与 `error`；`last_report` 为最近一次 `ScanReport` 的 JSON。`ReconnectSmokeResult` 携带两所 `BookFeedStatus` 与固定 `no_orders: true`（界面呈现 READ ONLY）。

所有 command 都返回统一 `ApiResponse<T>`。成功响应为 `{ success: true, data, error: null }`；失败响应为 `{ success: false, data: null, error: { code, message, retryable } }`。前端统一解包并通过 `ElMessage.error` 展示错误码和上下文。

## 3. 实时观测流程

1. 加载并校验配置。
2. 创建 HTTPS 客户端和 Binance/Bybit 公共行情适配器。
3. 并行加载两所品种规格并校验统一交易对及精度。
4. 启动两条公共订单簿 feed，等待两者同时 `VALID`。
5. 读取只读账户元数据，构建准入拒绝列表。
6. 使用 `scan_pair` 按 `Decimal` 计算两个方向，生成 `DecisionEvent`。
7. 通过现有有界归档 writer 写入事件；归档失败返回错误，不把未归档结果报告为完成。
8. 返回报告和不包含盘口内容的 feed 状态摘要。

连续观察复用同一流程并按配置轮询推进：每条结果经 `last_report` 通道实时回传，会话停止或进程退出前归档 writer 必须完成冲刷（窗口关闭与 SIGTERM 两条路径相同）。

## 4. 状态与失败关闭

- `SYNCING`、`STALE`、`INVALID` 不生成可执行机会；等待超时直接返回错误。
- 凭证缺失、地区/账户资格未确认、费率回退或费率过期仍可显示观察结果，但机会保持拒绝。
- 任意外部请求失败保留 Rust 错误上下文；不在前端重试或猜测结果。
- PAPER 报告的计划、人工升级和完成状态按现有数据库事实语义展示。
- 连续观察启动失败（配置缺失、归档不可写等）不会留半开会话：错误沿 `error` 字段与 stderr 上报；自动启动路径在失败时以退出码 1 结束进程，避免监督进程误判为健康。

## 5. 前端页面结构

- 顶栏：应用名称、安全模式、刷新配置按钮。
- 概览卡：品种、数量、深度、归档路径、两个 WebSocket URL。
- 操作区（READ-ONLY ACTIONS）：账户状态、一次实时观测、归档回放、三类 PAPER 烟测、B-04 账务控制烟测、连续观察开始/停止、重连烟测；运行中禁用重复操作，会话运行状态条常驻显示归档路径与启动时刻。
- 账户表：交易所、权限、费率来源/值、有效期、资格与拒绝原因。
- 行情与机会表：feed 状态、方向、买卖场所、VWAP、毛利、费用、延迟、净收益、准入 bps、结果。
- 回放/PAPER/烟测报告区：结构化 JSON 细节和关键指标。
- 全局错误提示：`ElMessage.error`，不吞掉 command 错误。

## 6. 收敛与删除记录

- 工作区收敛为二：根工作区 `members` 为 `["crates/core", "src-tauri"]`，删除的 CLI 成员能力去向——单次与连续观察由 `observe_once` / `start_continuous_observation` 覆盖；归档回放由 `replay_observations` 覆盖；重连烟测收纳进 `crates/core/src/observer.rs`（`run_reconnect_smoke`），Tauri command 仅做薄封装；账户检查由 `account_status` 覆盖。
- A-05 十四天影子观察改由桌面进程承载：监督进程直接运行 `./target/release/personal-taoli`，以 `TAOLI_OBSERVER_AUTOSTART` + `TAOLI_OBSERVER_ARCHIVE` 自动启动连续观察；停止监督即触发归档冲刷与干净退出。
- `src-tauri/src/commands/` 按职责拆分为 `system`、`account`、`observation`、`archive`、`paper`、`session`、`dto` 与 `support`；`session.rs` 持有 `SessionController` 与会话命令。
- `crates/core` 是唯一领域与基础设施 crate；`observer` 负责一次/连续观测编排与重连烟测，`scan` 负责机会计算，`venues` 负责交易所适配，PAPER 模块负责持久化安全核心。
- 前端源码统一位于根目录 `src/`，Tauri 只加载构建产物 `ui/`；前端通过 `@tauri-apps/api` 调用稳定的 command DTO，不直接耦合 Rust 内部模块。
- 配置样例保留在根目录 `config/observer.toml`，数据库迁移归属 `crates/core/migrations/`，测试 fixture 归属 `crates/core/tests/fixtures/`。

## 7. 开发、构建与生成物

- 安装前端依赖：`npm ci`。
- 启动桌面开发环境：`npm run tauri dev`。该命令先启动固定在 `http://localhost:1420` 的 Vite 服务，再启动 Tauri Rust 应用。
- 使用 PAPER 烟测前，启动本地 PostgreSQL，并在项目根目录 `.env` 设置 `TAOLI_DATABASE_URL=postgresql://taoli:taoli@127.0.0.1:55432/taoli`；当前 Docker 容器通过宿主机 `127.0.0.1:55432` 暴露 PostgreSQL。Tauri 启动时自动读取 `.env`，shell 中已存在的环境变量优先。桌面端不会从前端输入、保存或展示该连接串；未配置时 PAPER command 按安全策略返回 `PAPER_ERROR`，不伪造成功。
- 无人值守运行桌面连续观察：`TAOLI_OBSERVER_AUTOSTART=1 TAOLI_OBSERVER_ARCHIVE=<archive> ./target/release/personal-taoli`（`src-tauri` 产物，release 构建输出于 `target/release/personal-taoli`）；配置仍取 `config/observer.toml`。
- 单独构建前端：`npm run build`；产物写入根目录 `ui/`，该目录除 `.gitkeep` 外不进入版本控制。
- Rust 检查与测试：`cargo fmt --all -- --check`、`cargo check --workspace`、`TAOLI_DATABASE_URL=… cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace --release`。
- `src-tauri/gen/` 是 Tauri 自动生成的 schema 目录，不提交；`src-tauri/icons/icon.png` 是 Tauri 打包所需资源，必须保留。
