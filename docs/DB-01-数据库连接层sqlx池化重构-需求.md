# DB-01 数据库连接层 sqlx 池化重构：需求文档

文档状态：立项（2026-09-09）。立项编号为 **DB-01**（基础设施段外特批迭代）：F-03 编号已被预绑定「生产部署 / 真实接入评估」语义，G-02 已被预绑定「后续可视化/运维」语义，均不占用；本迭代为**纯基础设施重构**（数据库连接层实现替换），不新增任何业务能力、不改变任何对外可观察行为，按 UI-01 段外特批先例以独立前缀立项，与 AIDLC 手册 §11 无冲突。

现状：`crates/core/src/db.rs` 基于 `tokio-postgres 0.7`，**每次数据库操作新建一条连接**并 `tokio::spawn` 一个连接任务，操作结束后 `drop(client)` + `abort()` 该任务。单写者域（PAPER / order / execution / accounting / control / reconciliation）在整个域生命周期内独占一条连接，而该域内每个事务又要另开连接；F-02 烟测单个 run 内串行持有多个域，观察循环长时间运行时连接反复建立销毁。本迭代把连接层替换为 **`sqlx 0.8` + `PgPool` 连接池**，一次建池、多次复用，并干净切换（clean cutover）：删除全部 `tokio-postgres` 旧路径，不留并行双实现。

## 1. 目标

1. 连接层由「每操作一条新连接 + spawn 连接任务」改为「按数据库 URL 缓存的 `PgPool`，连接复用」；
2. **对外行为零变化**：全部 `pub(crate)` / `pub` 函数签名保持 `database_url: &str` 入口不变，`src-tauri` 命令层零改动；
3. **池化引入的两个新危险被证明已消除**：会话级 advisory 锁泄漏、事务状态泄漏（详见设计文档 §3）；
4. 全部金额仍走 `Decimal`（`sqlx` 的 `rust_decimal` feature 提供 `Type`/`Encode`/`Decode`），禁止转浮点；
5. G-01 只读查询层保持**零写路径**与「不抢领域单写者锁」语义。

核心原则（不可丢弃）：**行为等价的实现替换**——本迭代不改任何 SQL 语义、不改任何业务判定、不改任何错误文案的可观察含义、不新增业务能力；**安全边界不变**——`external_order_calls=0` 恒真、无真实/测试网订单路径、无真实订单凭据。

## 2. 需求

| 编号 | 要求 | 验收结果 |
|---|---|---|
| DB01-R01 | 连接池：`db.rs` 提供按 `database_url` 键缓存的 `PgPool`（`PgPoolOptions`，上限 16、获取超时 30s）；同 URL 重复调用复用同一池，不同 URL 各自独立 | 同一 URL 连续两次 `pool()` 返回 `ptr_eq` 的池；两个不同 URL 得到两个池；`pool.size()` 在一次烟测后 > 1 且 <= 16（连接确实被复用，未逐操作新建） |
| DB01-R02 | 入口签名不变：`migrate(&str)`、`DomainConnection::acquire(&str, …)`、`run_*_smoke(&str)`、`get_simulation_*(&str, …)` 全部保持 `database_url: &str`；`src-tauri` 不出现 `sqlx` / `tokio_postgres` 任何直接依赖 | `grep -rn "sqlx\|tokio_postgres" src-tauri/src` 为空；`cargo build --workspace --release` 通过；四个查询/烟测命令行为与重构前一致 |
| DB01-R03 | 会话锁不泄漏：单写者域 `disconnect()` 必须显式 `pg_advisory_unlock_all()`；池 `after_release` 钩子再兜底一次，覆盖 panic / `?` 提前返回路径 | 域 A `acquire` → `disconnect` 后，另一会话 `pg_try_advisory_lock(同 key)` 返回 `true`；构造「持锁后提前 `?` 返回」路径，锁同样被释放（探针 T3b/T4b 已证明两种机制均有效） |
| DB01-R04 | 事务状态不泄漏：G-01 只读路径改用 sqlx `Transaction` + `SET TRANSACTION READ ONLY`，**不得**使用裸 `BEGIN READ ONLY`（裸事务被释放后仍挂在后端上，会污染下一个借用者） | 只读事务内 `INSERT` 被 PostgreSQL 拒绝；事务 drop（不 commit）后，同一池的下一次 `INSERT` 成功；探针 T1b 记录裸 `BEGIN` 的污染现象作为反例证据 |
| DB01-R05 | 迁移行为等价：五个迁移文件（0001–0005）仍按 B-01/B-02/B-03/B-04/G-01 顺序、在同一会话、同一 `pg_advisory_lock(schema-migration)` 下执行；`raw_sql` 单次执行整个文件（含 `$$` plpgsql 与 `CREATE TRIGGER`）；`verify_schema` 仍要求 `MAX(version) == 5`；迁移锁执行完显式释放 | 空库一次 `migrate` 后 `schema_migrations` 含 1..5、全部表与不可变触发器存在；重复 `migrate` 幂等；version ≠ 5 时 `bail!` 文案与重构前一致 |
| DB01-R06 | 单写者语义保持：同一 `account_id\0instrument_id` 的第二个域 `acquire` 必须失败，文案保持「`{domain} execution domain already has a writer: account=… instrument=…`」 | F-02 烟测的 S06/S08 等重复获取断言仍为真；手动并发两次 `acquire` 第二次返回该错误 |
| DB01-R07 | `Decimal` / `JSONB` 保真：金额字段经 sqlx 往返不损失精度（`NUMERIC` ↔ `Decimal`），`JSONB` ↔ `serde_json::Value` 往返保真 | 探针 D1（`12.34560000` 原样往返）/ D2（JSONB 保真）为证据；烟测报告中全部 `Decimal` 字段与重构前一致 |
| DB01-R08 | 动态参数改写：`simulation_query.rs` 三处 `&[&(dyn tokio_postgres::types::ToSql + Sync)]` 改为 `sqlx::QueryBuilder::<Postgres>`，生成的 SQL 与绑定顺序与重构前一致 | 探针 E1（混合类型 `push`/`push_bind`）为证据；G-01 三个查询命令返回与重构前相同的板块数据；过滤条件（symbol/scenario/limit/offset）仍生效 |
| DB01-R09 | 失败关闭（fail closed）：URL 缺失或空白时报错而非静默降级；连接失败文案保持「failed to connect to the B-01 PostgreSQL database」；池耗尽时 30s 超时报错而非永久挂起 | 空白 URL → `bail!`；停掉 PostgreSQL → 连接错误文案不变；用满 16 条连接后第 17 个请求在 30s 内返回超时错误 |
| DB01-R10 | 旧路径删除干净：`tokio-postgres` 从 `crates/core/Cargo.toml` 与 `src-tauri/Cargo.toml` 移除；`rust_decimal` 的 `db-postgres` feature 移除（切换后成为死依赖）；`connect` / `close_connection` / `JoinHandle` 管线 / `DomainConnection.client` 字段全部删除 | `grep -rn "tokio_postgres\|tokio-postgres\|JoinHandle" crates/core/src src-tauri/src` 为空（`Cargo.lock` 由 `cargo update` 收敛）；`cargo build --workspace` 无未使用依赖警告 |
| DB01-R11 | 可验证性：既有 190 core + 8 tauri-lib 测试不回归；四门禁全绿；真实 PostgreSQL 烟测（B-01/B-02/B-03/B-04/ACCOUNTING/RECONCILIATION/CONTROL/F-02 + G-01 三查询）全部行为标记为真、`external_order_calls=0` | `cargo fmt --all -- --check`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace --release` 四门禁原始输出全绿；烟测报告与 G-01 查询原始输出留档 |

## 3. 明确不做

- **不新增任何业务能力**：不加新表、不加新字段、不改 `SCHEMA_VERSION`（仍为 5）、不改任何迁移 SQL 内容。
- **不做重试 / 熔断 / 指数退避**：连接失败仍是一次性失败并返回原错误；池化不等于自动重试。
- **不加遥测 / 指标 / 连接池监控面板**：不引入 `pool.size()` 上报、不加 tracing span、不加健康检查端点。
- **不做通用数据库抽象层**：不引入 `trait Database`、不引入仓储模式、不做多后端（SQLite/MySQL）支持；`PgPool` 就是具体类型。
- **不改任何 SQL 语句语义**：查询文本、`WHERE` 条件、`ORDER BY`、`ON CONFLICT` 子句逐字保留；仅改绑定与执行 API。
- **不改错误文案的可观察含义**：`anyhow::Context` 字符串保持原样（`src-tauri` 与烟测断言依赖它们）。
- **不改前端**：Vue/Tauri 页面、DTO、IPC 命令签名零改动。
- **不接入真实或测试网订单**；`external_order_calls=0` 恒真；不解除 A-05 影子窗口。
- **不改 G-01 只读层的「不抢领域单写者锁」设计**：只读查询仍不经 `DomainConnection::acquire`。
- **不引入 `sqlx-cli` / 编译期查询校验（`query!` 宏）**：本仓库无 `DATABASE_URL` 构建期约定，全部走运行时 `query`/`query_scalar`，避免把数据库可用性变成编译前提。

## 4. 验收方法

### 4.1 行为等价映射（重构前 → 重构后）

| 维度 | 重构前（tokio-postgres） | 重构后（sqlx 0.8） | 等价性证据 |
|---|---|---|---|
| 连接获取 | 每操作 `connect()` + `tokio::spawn` | `pool(url).await`（缓存复用） | DB01-R01；探针 I1/N1 |
| 连接释放 | `drop(client)` + `abort()` + `await` | 归还池 + `after_release` 清理 | DB01-R03；探针 T3b |
| 迁移执行 | `client.batch_execute(SQL)` | `sqlx::raw_sql(SQL).execute(&mut *conn)` | DB01-R05；探针 FACT1（`$$` plpgsql + `CREATE TRIGGER` 单次执行成功） |
| 单行查询 | `query_one(sql, &[&p])` | `query(...).fetch_one(exec)` | 探针 F1：零行 = `Err(sqlx::Error::RowNotFound)` |
| 可选查询 | `query_opt(sql, &[&p])` | `fetch_optional(exec)` | 语义一致（`Option<Row>`） |
| 多行查询 | `query(sql, &[&p])` | `fetch_all(exec)` | 语义一致（`Vec<Row>`） |
| 行取值 | `row.get::<_, T>(i)` | `row.try_get::<T, _>(i)?` | 类型错误由 panic 变为 `Result`（更严格，不放宽） |
| 事务 | `client.transaction()` → `Transaction<'_>` | `conn.begin()` / `pool.begin()` → `Transaction<'_, Postgres>` | 探针 T5；drop 不 commit → 自动回滚（Q2/T5b） |
| 只读事务 | 裸 `batch_execute("BEGIN READ ONLY")` | sqlx `Transaction` + `SET TRANSACTION READ ONLY` | DB01-R04；探针 T5（写被拒）+ T5b/T5c（池仍健康） |
| 动态参数 | `&[&(dyn ToSql + Sync)]` | `QueryBuilder::<Postgres>` | DB01-R08；探针 E1 |
| `Decimal` | `rust_decimal` `db-postgres` feature | `sqlx` `rust_decimal` feature | DB01-R07；探针 D1 |
| `JSONB` | `tokio-postgres` `with-serde_json-1` | `sqlx` `json` feature | 探针 D2 |
| 单写者锁 | `pg_try_advisory_lock` on 独占连接 | `pg_try_advisory_lock` on 专用锁连接 | DB01-R06；探针 C2（他人被阻）/ C3（锁跨 commit 存活） |

### 4.2 验收用例

| 用例 | 步骤 | 通过判据 |
|---|---|---|
| U01 门禁 | 依次执行四门禁命令 | 四条命令原始输出全绿，零 warning（clippy `-D warnings`） |
| U02 单测不回归 | `cargo test --workspace` | 190 core + 8 tauri-lib 全通过；无新增忽略 |
| U03 真实烟测 | `TAOLI_DATABASE_URL=postgresql://taoli:taoli@127.0.0.1:55432/taoli cargo test -p personal-taoli-core -- --nocapture --test-threads=1`（DB 相关用例）+ 经 `src-tauri` 命令入口跑 B01/B02/B03/B04/ACCOUNTING/RECONCILIATION/CONTROL/F-02 | 每份报告的全部行为标记为 `true`；`external_order_calls == 0`；`Decimal` 字段数值与重构前一致 |
| U04 G-01 只读 | 调 `get_simulation_overview` / `get_simulation_runs` / `get_simulation_run_detail` | 三板块返回真实数据；只读事务内写被拒；查询不抢领域锁（并发跑 F-02 烟测时查询不阻塞） |
| U05 锁不泄漏 | 域 `acquire` → `disconnect` → 另一会话 `pg_try_advisory_lock(同 key)` | 返回 `true`（锁已释放） |
| U06 单写者互斥 | 同 `account_id\0instrument_id` 并发两次 `acquire` | 第二次 `bail!`，文案含 `already has a writer` |
| U07 失败关闭 | 空白 URL；PostgreSQL 停机 | 空白 URL 立即报错；停机时错误文案为 `failed to connect to the B-01 PostgreSQL database` |
| U08 幂等迁移 | 空库 `migrate` 两次 | 第二次不报错；`schema_migrations` 仍为 1..5；`verify_schema` 通过 |
| U09 旧路径清零 | `grep -rn "tokio_postgres\|tokio-postgres" crates/core src-tauri` | 源码与两个 `Cargo.toml` 均无命中 |
| U10 池复用 | 一次完整 F-02 烟测后读 `pool.size()` / `num_idle()` | `size <= 16` 且烟测期间未出现「每操作新建连接」的连接数暴涨（PostgreSQL `pg_stat_activity` 计数稳定） |

### 4.3 发布签收

| 项 | 状态 |
|---|---|
| 设计评审（§4.3 门禁，含删除清单） | 待办 |
| 实现（§4.4：领域模型 → 适配器 → 编排 → 入口） | 待办 |
| 四门禁 + 真实烟测证据（§4.5） | 待办 |
| 文档同步（手册 §6.4 矩阵 / §10 发布记录 / §11 下一轮入口 / 架构文档「当前实现状态」） | 待办 |
| 临时产物清理（探针 crate、`taoli_probe` 库） | 待办 |
| 所有者签收 | 待办 |
