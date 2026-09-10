# DB-01 数据库连接层 sqlx 池化重构 任务

文档状态：待评审（2026-09-10）
对应设计：`docs/DB-01-数据库连接层sqlx池化重构-设计.md`
实现顺序依据 AIDLC §4.4：领域模型 → 适配器 → 业务编排 → CLI/运维入口。
每层完成即迁移**全部**调用点并删除旧接口，不留过渡期。

---

## 1. 实现顺序

### 1.1 db.rs 契约（领域模型层）— 已完成，待评审

- [x] `crates/core/Cargo.toml`：`rust_decimal` 去 `db-postgres`；`tokio-postgres` → `sqlx 0.8`
      （`default-features = false`，features = `runtime-tokio`/`postgres`/`rust_decimal`/`json`）
- [x] `src-tauri/Cargo.toml`：删 `tokio-postgres`，`rust_decimal` 去 `db-postgres`
- [x] `db.rs` 全量重写（264 行）：`pool()` / `migrate()` / `migrate_pool()` /
      `verify_schema<E: PgExecutor>` / `DomainConnection{pool, lock_connection, …}` /
      `disconnect()`；纯函数与 `SCHEMA_VERSION=5` 逐字保留
- [x] 编译验证：`cargo build -p personal-taoli-core` 中 **db.rs 零错误**；
      余下 14 处为预期的下游断点（见 §1.2–§1.4 待迁移项）

**验收**：db.rs 自身无编译错误（已证）。

### 1.2 六个单写者域（适配器层）

按依赖顺序逐模块迁移，每模块完成即 `cargo build -p personal-taoli-core --message-format short 2>&1 | grep "<file>:"` 归零。

- [x] **paper.rs**（864 行，21 查询点）
  - `use tokio_postgres::{Row, Transaction}`(:8) → `use sqlx::postgres::PgRow` + `use sqlx::Transaction`
  - `lock_and_check_balances`(:583) / `insert_plan`(:625)：形参 `&Transaction<'_>` →
        `&mut Transaction<'_, Postgres>`；内部每次查询传 `&mut **transaction`（设计 §2.3）
  - `recovered_plan`(:689-691)：`Row` → `PgRow`
  - `set_paper_balance`(:136-193)：`connect`/`close_connection` → `pool(database_url).await?` +
        `verify_schema(&pool)` + `pool.begin()`；`pg_advisory_xact_lock`(:166) 留在事务内；
        删除 :191 的 `close_connection`（借用连接 drop 即归还）
  - `business_row_count`(:743-753)：同上换池
  - `recover_active_plans(&self)`(:260)、`load_plan(&self)`(:281)：`self.inner.client` → `self.inner.pool`
        （`&self` 接收器**不变**，设计 D3）
  - `.inner.client` 8 处（213/262/283/291/299/386/396/406）→ `.inner.pool`；
        其中 213/386/396/406 为跨行链式，改的是 `.client` 所在行
  - `use crate::db::{…}`(:10-13)：去掉 `close_connection`、`connect`，加 `pool`
- [x] **order.rs**（852 行，16 查询点）
  - `Row`(:6) → `PgRow`；`snapshot(row: Row)`(:564) 同步改
  - `lock_fact`(:543) / `append_action`(:551)：`&tokio_postgres::Transaction<'_>` →
        `&mut Transaction<'_, Postgres>`，内部 `&mut **tx`
  - `load(&self)`(:526) / `recover_nonterminal(&self)`(:532)：单行 `self.inner.client.query_one/query`
        → `self.inner.pool`（`&self` 不变）
  - `.inner.client` 9 处（163/183/220/281/349/394/440/528/533）→ `.inner.pool`；
        前七处为跨行链式，改的是 `.client` 所在行
- [x] **execution.rs**（486 行，7 查询点）
  - `Row`(:5) → `PgRow`；`execution_snapshot`(:281) 同步改
  - `load(&self)`(:263)；`.inner.client` 3 处（119/171/266，均跨行链式）→ `.inner.pool`
- [x] **accounting.rs**（271 行，6 查询点）
  - `Row`(:5) → `PgRow`；`event(row: Row)`(:261) 同步改
  - `load_event(&self)`(:147)；`.inner.client` 4 处（94/134/149/205，其中 94/205 跨行）→ `.inner.pool`
- [x] **control.rs**（274 行，8 查询点）
  - `load(&self)`(:180)；`.inner.client` 3 处（101/144/182，其中 101/144 跨行）→ `.inner.pool`
- [x] **reconciliation.rs**（265 行，4 查询点）
  - `record_snapshot`(:86) 单行 10 bind → 逐个 `.bind()`；`.inner.client` 2 处（86/106，106 跨行）

**统一规则**（设计 §2.2）：`row.get::<_,T>(i)` → `row.try_get::<T,_>(i)?`；
`query_one`→`fetch_one`、`query_opt`→`fetch_optional`、`query`→`fetch_all`、
`execute`→`sqlx::query(..).bind(..).execute(exec)`（需受影响行数处用 `.rows_affected()`）。
SQL 文本已用 `$N` 占位，**逐字不动**。

**14 处事务开启点**（设计 D4，不加 `begin()` 包装）——`client.transaction()` /
`.inner.client.transaction()` → `.pool.begin().await?` / `self.inner.pool.begin().await?`：
paper.rs:161/214、order.rs:184/221/282/350/395/441、execution.rs:120/172、
accounting.rs:95、control.rs:102/145、reconciliation.rs:107。
每处返回的事务变量后续须以 `&mut **txn` 逐条传参（设计 §2.3）；
**禁止**用 lint 豁免、静默默认值或特殊输入分支让任何调用点通过编译。

**验收**：六个模块各自 smoke 全绿；`grep -rn "tokio_postgres" crates/core/src/` 仅剩
simulation.rs / simulation_query.rs（1.3/1.4 未做前）。

### 1.3 simulation.rs（业务编排层，2126 行）

- [x] 4 对 `connect`/`close_connection`：:364/373（`available_balance`）、:429/591（run 落库）、
      :1603/1611（S05 投影）、:1834/1840 → 换 `pool(database_url).await?` + 借用连接
- [x]] `available_balance`(:358)：`query_opt` → `fetch_optional`；`r.get::<_,Decimal>(0)` →
      `r.try_get::<Decimal,_>(0)?`（**保留**"无记录视为 0"语义）
- [x]] `use crate::db::{close_connection, connect, hex_digest, migrate}`(:31) → 去 connect/close_connection，加 pool
- [x]] **必须保留**：`SimulationEngine::spawn`(:1122-1134) 的降级语义——`TAOLI_DATABASE_URL`
      缺失或 `migrate` 失败 → `tracing::error!` + 返回 `None`，不阻断观察循环（U07）
- [x]] **必须保留**：:1778-1790 三核并发持有（PaperCore→recover_active_plans→disconnect，
      OrderCore→load×2→disconnect，ExecutionCore→load→disconnect）；这是 `MAX_CONNECTIONS=16`
      的取值依据，迁移后须仍能并发（U10 观测 `pool.size()`）
- [x]] `offer(&self, DecisionEvent)`(:1183)：`&self` 接收器不变
- [x]] 9 处 `#[test]`（1857/1874/1897/1921/1941/1965/2091/2103/2114）不改断言语义

**验收**：`run_simulation_smoke` 全绿；降级分支与三核并发行为不变。

### 1.4 simulation_query.rs（G-01 只读查询层，1008 行）

**结构变更**（设计 §6、§3.3）：

- [x]] **删除** `open()`(:266) 与 `close()`(:278)；删除裸 `BEGIN READ ONLY`(:270) / `COMMIT`(:279)
- [x]] 三个公开入口各自在一个 `async` 体内完成：`pool(database_url).await?` →
      `pool.begin().await?` → `SET TRANSACTION READ ONLY` → `verify_schema(&mut *txn)` →
      全部查询 → `txn.rollback().await`
      - `get_simulation_overview`(:288-437)：~10 `query_one` + ~13 `query`
      - `get_simulation_runs`(:440-486)：COUNT + `read_run_rows`
      - `get_simulation_run_detail`(:489-634)：run 行 + report + 意图/成交/执行事件/审计/余额快照
      - **约束（N2b）**：任一语句失败后该事务即 aborted，不得复用；错误直接上抛
- [x]] `read_run_rows`(:659-704)：形参 `client: &Client` + `params: &[&(dyn ToSql + Sync)]`
      → `txn: &mut Transaction<'_, Postgres>` + `params: &[(String, String)]`，内部逐个 `.bind()`
      （设计 D7：SQL 文本与 `$N` 编号逐字不变，M1/M2/M3 已证）
- [x]] :462、:479 的 `dyn ToSql` 映射删除，改为 `.bind()` 循环
- [x]] `build_run_filter`(:641-656) 签名与实现**不变**（仍返回 `Vec<(String,String)>`）
- [x]] `use tokio::task::JoinHandle`(:15)、`use tokio_postgres::Client`(:16) 删除
- [x]] `use crate::db::{close_connection, connect, migrate, to_u64, verify_schema}`(:17-18)
      → `{migrate, pool, to_u64, verify_schema}`
- [x]] 全部 `row.get(i)` → `row.try_get(i)?`；`to_u64(row.get::<_,i64>(n)).unwrap_or(0)`
      的降级语义**保留**
- [x]] **必须保持**：模块文档承诺的"独立只读连接、不经 `DomainConnection::acquire`、
      不抢领域单写者锁"；页面查询层零写路径；`external_order_calls=0` 恒真（R05/U04）
- [x]] 六个跨 crate 公开签名不变：`SimulationOverview`/`SimulationRunDetail`/`SimulationRunsPage`/
      `get_simulation_overview`/`get_simulation_runs`/`get_simulation_run_detail`
      （`src-tauri/src/commands/simulation_query.rs:2` 精确导入这六项）

**验收**：三个入口对真实库返回与 G-01 基线一致的数据；U04 证明只读路径拒写且
不阻塞并发 F-02 smoke。

### 1.5 依赖清理

- [x]] `grep -rn "tokio_postgres\|tokio-postgres" crates src-tauri Cargo.toml` 全仓为空
- [x]] `cargo tree -p personal-taoli-core | grep tokio-postgres` 无输出（传递依赖亦清除）
- [x]] `cargo tree -p personal-taoli-core | grep -E "sqlx|rust_decimal"` 确认 sqlx 0.8.6 / rust_decimal 1.43.0
- [x]] 无未使用 import（clippy `-D warnings` 兜底）

### 1.6 验证（AIDLC §4.5，四道门禁 + 真实 smoke，全部留存原始输出）

- [x]] `cargo fmt --all -- --check`
- [x]] `cargo test --workspace`
- [x]] `cargo clippy --workspace --all-targets -- -D warnings`
- [x]] `cargo build --workspace --release`
- [x]] 真实 PostgreSQL smoke（`postgresql://taoli:taoli@127.0.0.1:55432/taoli`，
      `TAOLI_DATABASE_URL` 已设，`--nocapture --test-threads=1`）：
      paper / order / execution / accounting / control / reconciliation / simulation 七个 smoke
      + G-01 三个只读入口。断言全部行为标志 `true`、`external_order_calls=0`
      > 注意：DB 模块测试是普通 `#[test]` 而非 `#[tokio::test]`
      > （paper.rs:812-864、order.rs:824-852、execution.rs:477、simulation.rs:1857+），
      > 先读清各 smoke 的实际调用方式再跑
- [x]] U01–U10 验收用例逐条留存输出（映射见设计 §7）

## 2. 文档同步（AIDLC §4.6）

- [x]] 架构文档"当前实现状态"：连接层由 tokio-postgres 即连即断改为 sqlx `PgPool` 池化
- [x]] 手册 §6.4 需求-实现-验证矩阵（约 218 行）：追加 DB01-R01…R11 行，
      格式 `需求 | 可观察结果 | 实现位置 | 验证证据 | 状态`
- [x]] 手册 §10 发布记录（约 446 行）：追加 DB-01 行，格式 `迭代 | 日期 | 交付 | 质量门禁 | 运行边界`
- [x]] 手册 §11 下一步（约 480 行）：更新 F-03 保留状态与下一业务条目
- [x]] 配置样例一致性：`.env.example`（根，`TAOLI_DATABASE_URL=…@127.0.0.1:55432/taoli`）与
      `deploy/.env.example`（`DATABASE_URL=…@localhost:5432/taoli`）字段与代码一致——
      本次未改环境变量名，预期无需变更，仍须核对
- [x]] 删除临时脚本与产物：`rm -rf /tmp/sqlxprobe`；drop database `taoli_probe`
- [x]] 陈述遗留风险（设计 §8：`MAX_CONNECTIONS=16` 上限、只读事务 aborted 约束、
      `try_get` 语义变更）
- [ ] **Owner 签收**：不得自行标记 `verified` / `released`

## 3. 明确不做（防"顺手"扩张）

- 不做连接池遥测/指标导出
- 不做重试、熔断、退避
- 不抽象通用 DB trait
- 不引入 `query!` 编译期宏（无 `DATABASE_URL` 编译期依赖）
- 不引入 `sqlx-cli` / `sqlx::migrate!` 宏（迁移仍走 `include_str!` + `raw_sql`）
- 不改 schema、不改迁移文件、不动 `SCHEMA_VERSION`
- 不动前端（Vue/Tauri）与 `src-tauri` 源码（依赖清理已足）
