# 个人加密货币套利系统 (Taoli Observer)

个人自用的加密货币套利观察与仿真系统。以模拟/纸面数据为主驱动，叠加真实交易所行情流，评估与追踪套利策略，并为 Agent 评测与迭代执行提供基础设施。

> 说明：本仓库含 AIDLC 迭代执行手册与多份需求/设计/任务文档，`docs/` 下可回溯每个迭代（PORT / G / DB / MIG / PERF 等）。

## 架构概览

Rust workspace（Cargo 多 crate）提供领域核心与桌面壳层，Vue 前端作为桌面界面（Tauri v2 WebView）。

```
┌─────────────────────────────┐
│  src/ (Vue 3 + Element Plus + Vite + TS) │  Tauri WebView 前端
├─────────────────────────────┤
│  src-tauri (Tauri v2 shell)   │  命令桥接 (tauri commands)
├─────────────────────────────┤
│  crates/core (领域核心)       │  执行/账务/行情/Agent/仿真/
│                               │  观察/健康/告警/备份/运维 等
└─────────────────────────────┘
        │
        ▼
   PostgreSQL (sqlx + migrations)
   + 交易所行情流 (Binance / Bybit WS + REST)
```

## 目录结构

- `crates/core/` — 领域与基础设施核心 crate（`personal-taoli-core`）。
  - `src/execution.rs` 执行 / `accounting.rs` 账务 / `reconciliation.rs` / `control.rs` 对账与风控控制
  - `src/paper.rs` 纸面/撮合模拟
  - `src/market.rs` + `venues/`（binance、bybit，REST + WebSocket 流）行情接入
  - `src/agent/` — Agent 接入与评测（`app_server`、`protocol`、`tools`、`trace`、`approval`、`sandbox`（macOS Seatbelt 沙箱策略））
  - `src/simulation.rs` `simulation_query.rs` — 模拟套利运行与查询仪表盘
  - `src/observer.rs` `dashboard.rs` `metrics.rs` `health.rs` `alert.rs` `alert_manager.rs` — 观察/监控
  - `src/backup*.rs` `archive.rs` `runbook*.rs` `operations.rs` `rpo_rto.rs` `fencing.rs` `fault_injection.rs` `notification.rs` `config.rs` `db.rs` — 运维/基础设施
  - `migrations/` — sqlx 数据库迁移（`0001`~`0008`）
- `src-tauri/` — Tauri v2 桌面壳层（`personal-taoli-tauri`），命令定义在 `src/commands/`（`agent`、`simulation`、`agent_live` 等）。
- `src/` — Vue 3 前端：`App.vue`、`components/`（仿真仪表盘、Agent 分析页、报表视图等）、`commands.ts`、`types.ts`、`glossary.ts`、`markdown.ts`。
- `ui/` — 静态 UI 资源（`index.html`）。
- `docs/` — 需求 / 设计 / 任务 文档，AIDLC 迭代执行手册，Agent 评测白皮书。
- `deploy/` — 部署编排（`docker-compose.yml`、`Dockerfile`、`init.sql`、`prometheus.yml`、`grafana/`、`taoli-core.service`、`.env.example`）。
- `data/` — 本地数据（`agent-traces/`、`backup/`、`archive/`）。
- `.cache/` — 行情/数据 API 缓存（含 A 股与基金样本数据）。

## 技术栈

| 层 | 技术 |
| --- | --- |
| 语言 | Rust 2024 edition、TypeScript |
| 桌面壳 | Tauri v2 |
| 前端 | Vue 3、Element Plus、Vite 7、markdown-it |
| 数据库 | PostgreSQL + sqlx（migrate / pool / rust_decimal） |
| 行情 | reqwest（rustls）、tokio-tungstenite（WS 流） |
| 观测 | tracing、metrics、alert_manager、Prometheus/Grafana 部署 |

## 开发与运行

前置：Rust 1.88+、Node 18+、PostgreSQL。

```bash
# 配置环境
cp .env.example .env

# 前端开发
npm install
npm run dev

# 构建前端（vue-tsc 类型检查 + vite build）
npm run build

# Tauri 桌面
npm run tauri dev       # 或 npm run tauri build

# Rust 核心/测试
cargo build -p personal-taoli-core
cargo test --workspace
```

数据库迁移由 sqlx 管理（`crates/core/migrations/`），部署用 `deploy/` 下的编排：

```bash
cd deploy && docker compose up -d
```

## 安全

- 交易所密钥通过 `.env` 注入；`zeroize` 用于敏感内存擦除。
- Agent 执行受限：macOS Seatbelt 沙箱策略位于 `crates/core/src/agent/sandbox/policies/`，含只读/网络/偏好策略（`seatbelt_*.sbpl`）。

## 免责声明

个人自用项目，非投资建议。行情与模拟数据可能存在偏差；请勿据此进行真实投资决策。