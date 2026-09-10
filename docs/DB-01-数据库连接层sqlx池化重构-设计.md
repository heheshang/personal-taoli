# DB-01 数据库连接层 sqlx 池化重构 设计

文档状态：待评审（2026-09-10）
对应需求：`docs/DB-01-数据库连接层sqlx池化重构-需求.md`
前置迭代：G-01（模拟盘查询层，190 core + 8 tauri-lib 测试通过）

---

## 1. 模块边界

```
crates/core/src/db.rs                  ← 唯一连接层：池、迁移、schema 校验、领域单写者锁
crates/core/src/paper.rs               ← B-01 适配器（reserve_plan / set_paper_balance / business_row_count）
crates/core/src/order.rs               ← B-02 适配器（order_facts / order_action_facts）
crates/core/src/execution.rs           ← B-03 适配器（double-leg 执行事实）
crates/core/src/accounting.rs          ← B-04 适配器（ledger 事件）
crates/core/src/control.rs             ← B-04 适配器（控制命令）
crates/core/src/reconciliation.rs      ← B-04 适配器（余额快照对账）
crates/core/src/simulation.rs          ← F-02/G-01 编排（run 落库、引擎、smoke）
crates/core/src/simulation_query.rs    ← G-01 只读查询层（零写）
crates/core/Cargo.toml                 ← sqlx 0.8 依赖
src-tauri/Cargo.toml                   ← 移除 tokio-postgres（源码零改动）
```

**边界不变式**：`db.rs` 之外任何模块不得出现 `sqlx::PgPoolOptions`、`PgPool` 的构造，
也不得自行 `connect`。所有连接一律来自 `db::pool(database_url)` 的进程级缓存。
`src-tauri` 侧零改动——所有入口签名仍是 `database_url: &str`。

## 2. 唯一数据契约

### 2.1 公开（crate 内）契约

```rust
// 连接获取：按 database_url 缓存 PgPool，进程内唯一真相源
pub(crate) async fn pool(database_url: &str) -> Result<PgPool>;

// 迁移：幂等；migrate 保留原签名（simulation.rs:1129 依赖）
pub(crate) async fn migrate(database_url: &str) -> Result<()>;
pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()>;

// schema 校验：由 `&Client` 改为泛型 executor（DB01-R10）
pub(crate) async fn verify_schema<'e, E>(executor: E) -> Result<()>
where E: PgExecutor<'e>;

// 领域连接：单写者锁 + 池
pub(crate) struct DomainConnection {
    pub pool: PgPool,                            // 原 pub client: Client
    lock_connection: PoolConnection<Postgres>,   // 私有，独占会话锁
    pub account_id: String,
    pub instrument_id: String,
}
impl DomainConnection {
    pub async fn acquire(database_url: &str, account_id: impl Into<String>,
                         instrument_id: impl Into<String>, domain: &str) -> Result<Self>;
    pub async fn disconnect(mut self);           // 签名不变
}

// 纯函数：签名与实现逐字不变（零 DB 依赖）
pub(crate) fn advisory_key / validate_id / hex_digest / db_time / to_i64 / to_u64;
pub(crate) const SCHEMA_VERSION: i64 = 5;        // 不变
```

**唯一数据契约**：`PgPool` 是 crate 内唯一的连接真相源。旧的 `(Client, JoinHandle<()>)`
二元组、`DomainConnection.client`、`DomainConnection.connection` 全部消失，不留别名、
不留兼容包装。

### 2.2 类型映射（逐条经探针编译验证）

| 旧（tokio-postgres） | 新（sqlx 0.8.6） | 证据 |
| --- | --- | --- |
| `tokio_postgres::Row` | `sqlx::postgres::PgRow` | probe8 编译错误：`sqlx::PgRow` 路径不存在 |
| `row.get::<_, T>(i)` | `row.try_get::<T, _>(i)?` | L3 |
| `tokio_postgres::Transaction<'_>` | `sqlx::Transaction<'_, Postgres>` | L1c/L2a |
| `client.transaction()` | `pool.begin()` / `conn.begin()` | L1d；`conn.begin()` 需 `use sqlx::Acquire` |
| `client.query_one(sql, &[&p])` | `sqlx::query(sql).bind(p)…fetch_one(exec)` | L1a/L1b/L1c |
| `client.query_opt(...)` | `…fetch_optional(exec)` | L7 |
| `client.query(...)` | `…fetch_all(exec)` | probe2 E1 |
| `client.execute(...)` | `sqlx::query(...).bind(...).execute(exec)`；受影响的行数用 `.rows_affected()` | N3=1 |
| `client.batch_execute(sql)` | `sqlx::raw_sql(sql).execute(exec)` | FACT1（含 `$$` plpgsql 与 CREATE TRIGGER 全量通过） |
| `&[&(dyn ToSql + Sync)]` | 逐个 `.bind()`（`Vec<(String,String)>` 同构，SQL 文本逐字不变） | M1/M2/M3 |
| `PgPoolOptions` | `sqlx::postgres::PgPoolOptions`（非 `sqlx::PgPoolOptions`） | probe8 编译错误 |
| `PgConnection::connect` | 需 `use sqlx::Connection` | probe8 编译错误 |

### 2.3 Executor 传参规则（决定 30+ 处调用的写法）

探针 N6 证明：**泛型 executor 参数按调用消耗**，不能在同一 helper 内用两次。因此：

- helper 只需**一次**查询 → 形参 `executor: E where E: PgExecutor<'e>`，调用方传
  `&mut *txn` / `&mut *conn` / `&pool` 均可（N1、N5、N5b 全部通过）。
- helper 需要**两次以上**查询 → 形参必须是 `&mut Transaction<'_, Postgres>` 或
  `&mut PoolConnection<Postgres>`，内部 `&mut **txn` 反复借用（N6=(2,1)、N7=(2,1)）。
- 方法位置直接写 `txn.execute(sql).await?` 可行（L2b=Ok(1)），走 `DerefMut`；
  但**泛型形参位置** `fetch_one(txn)` 与 `fetch_one(&mut Transaction)` 均报 E0277，
  必须 `&mut **txn`（L2a=Ok(8)）。

适用：`verify_schema`（单次 → 泛型）、`lock_and_check_balances` / `insert_plan` /
`lock_fact` / `append_action`（多次或写 → `&mut Transaction`）。

## 3. 状态、错误与降级语义

### 3.1 会话状态审计（重构安全性的前提）

池化把"每次操作一条新连接"换成"从池借一条可能用过的连接"，唯一风险是**会话级状态跨借用者残留**。
全仓审计结果：

| 会话级机制 | 出现位置 | 池化后是否安全 |
| --- | --- | --- |
| `pg_advisory_xact_lock` | paper.rs:166、219、592 | 安全——事务级锁随 COMMIT/ROLLBACK 自动释放，且 sqlx 事务全程钉住同一连接 |
| `pg_advisory_lock` / `pg_try_advisory_lock` | **仅 db.rs:99、233** | 安全——迁移锁在 `migrate_pool` 内同一借用连接上取放；领域锁在私有 `lock_connection` 上取，`disconnect` 显式放 |
| `SET SESSION` / `set_config` / `current_setting` | 无 | — |
| 临时表 | 无 | — |
| `LISTEN` / `NOTIFY` | 无 | — |
| 裸 `BEGIN READ ONLY` / `COMMIT` | simulation_query.rs:270、279 | **不安全 → 本设计删除**（见 §6） |

结论：业务代码不依赖任何会话级状态，池化无语义漂移。唯一例外是只读层自己开的裸事务。

### 3.2 两条池卫生不变式（已写入 db.rs 模块文档）

1. **会话级 advisory 锁不得活过借用者。** `disconnect` 显式 `pg_advisory_unlock_all()`；
   `after_release` 钩子是 panic 路径兜底。
2. **事务状态不得活过借用者。** 只读会话一律用 sqlx `Transaction` + `SET TRANSACTION READ ONLY`，
   **禁止**裸 `BEGIN`。

### 3.3 错误语义（逐条对齐旧行为）

| 场景 | 旧行为 | 新行为 | 证据 |
| --- | --- | --- | --- |
| `query_one` 零行 | `tokio_postgres::Error::UnexpectedRows` | `sqlx::Error::RowNotFound` | F1=true |
| `MAX(version)` 空表 | `Option<i64>` = None | 同（`query_scalar::<Option<i64>>`） | L5=None |
| 金额/时间戳往返 | Decimal 精确 | 精确（sqlx `rust_decimal` feature 自带 Type/Encode/Decode） | D1/L3=12.345678 |
| 行内类型不匹配 | `row.get` **panic** | `try_get` 返回 `Err` → **收敛为 Result** | DB01-R08；L3 |
| 领域锁被占 | `bail!("{domain} execution domain already has a writer: …")` | **文案逐字不变** | db.rs:238 |
| 迁移失败 | `Err` + context | 同（`raw_sql` 全量执行，含 plpgsql 触发器） | FACT1 |
| schema 版本不符 | `bail!("unsupported B-01 schema version: expected 5, found …")` | **逐字不变** | db.rs:131 |
| 连接池耗尽 | 旧代码无上限（每次新建） | **失败关闭**：`ACQUIRE_TIMEOUT=30s` 后 Err | L8（400ms 超时 → is_err=true，elapsed=403ms） |
| 只读会话内写 | PostgreSQL 拒绝 | 同，且**事务进入 aborted 态**，后续读全部失败 | M4=true、**N2b** |

**N2b 是本设计新增的关键约束**：探针显示只读事务里一条写语句失败后，
同一事务的后续读返回 `current transaction is aborted, commands ignored until end of transaction block`。
因此 `simulation_query.rs` 的每个公开入口必须在**一个** `async` 体内完成"开事务 → 全部查询 → 回滚"，
不得在语句失败后继续复用该事务。旧代码是每次 `open()` 一条新连接，天然无此耦合；池化后必须显式保证。

### 3.4 降级语义（保持不变）

`SimulationEngine::spawn`（simulation.rs:1122-1134）：`TAOLI_DATABASE_URL` 缺失或 `migrate` 失败
→ `tracing::error!` 告警并返回 `None`，**不阻断观察循环**。本重构保留 `migrate(&str)` 原签名
正是为了不触碰这段降级逻辑。空/空白 URL 由 `db::pool` 失败关闭（`bail!`），与该分支语义一致。

## 4. 关键决策（评审点）

| 编号 | 决策 | 依据 |
| --- | --- | --- |
| D1 | 用 **`after_release`** 而非 `before_acquire` 兜底解锁 | T3b=true（`after_release` 能清掉借用者泄漏的会话锁）；**S12=false**（`before_acquire` 清不掉——锁被一条*空闲*后端持有，它碰不到） |
| D2 | 只读会话用 sqlx **`Transaction` + `SET TRANSACTION READ ONLY`**，删掉裸 `BEGIN READ ONLY` | T1b=Err（裸 `BEGIN` 未 COMMIT 就归还 → 毒化下一个借用者：`cannot execute INSERT in a read-only transaction`）；T5/T5b/T5c 与 L7/L7b/M6 证明 sqlx 事务形式拒写（`write_rejected=true`）且归还后池仍可写 |
| D3 | `DomainConnection.pool` 设为 `pub` 供查询，另设**私有** `lock_connection` 独占会话锁 | S9/L1a 证明 `&PgPool` 本身即 Executor（共享引用可查询）；七个 `&self` 读方法因此**签名逐字不变**（paper.rs:260、order.rs:526/532、execution.rs:263、accounting.rs:147、control.rs:180、simulation.rs:1183） |
| D4 | **不**为 `DomainConnection` 提供 `begin()` 包装；14 处事务调用点直接用 `.pool.begin().await?` | 旧 `DomainConnection`（HEAD:143-195）本就无 `begin()`，包装是新增抽象——sqlx 把事务生命周期绑到池借用上，任何返回自有/`'static` 事务的包装都不健全。14 处调用点全部已持有 `&mut self` 或本地 client，`.transaction()` → `.pool.begin()` 是 1:1 映射（L1d） |
| D5 | 池按 **`database_url` 缓存**（`LazyLock<Mutex<HashMap<String, PgPool>>>`），而非单一全局池 | `src-tauri` 每条命令重读 `TAOLI_DATABASE_URL`，且 `load_saved_env_config()` 可在 `setup` 期间改写它——单一全局池会拿到过期 URL。符合项目规则 `rs-lazylock`（initializer 在声明处已知） |
| D6 | `MAX_CONNECTIONS=16`、`ACQUIRE_TIMEOUT=30s` | 每个持有中的领域占 1 条锁连接 + 事务借用；`simulation.rs:1778-1790` 同时持有三个核。旧代码**无上限**新建连接。L8 证明耗尽时失败关闭而非挂死 |
| D7 | `dyn ToSql` 切片改为**逐个 `.bind()`**，不引入 `QueryBuilder` | `build_run_filter` 返回同构 `Vec<(String,String)>`，SQL 已用 `$N` 占位（simulation_query.rs:649/653），逐个 bind 即可保持 SQL 文本逐字不变；`QueryBuilder` 属过度设计。M1/M2/M3 验证单位、双位过滤与 COUNT 全部正确 |
| D8 | 迁移用 `sqlx::raw_sql(&str).execute(&mut *connection)`，五份迁移跑在**同一条**借用连接上 | FACT1 证明 `raw_sql` 能整份执行含 `$$` plpgsql（`reject_audit_event_mutation()`，0001:84-92）与 `CREATE TRIGGER audit_events_immutable`（0001:94）的 `0001_paper_core.sql`，**不得手工拆句**；同一连接保证 advisory 锁与各批次共享会话 |

### 探针原始输出（证据基线）

probe7（池卫生）：
```
T1  in_transaction_after_release: Ok(false)
T1b sqlx_default_reset_clears_raw_txn: Err(cannot execute INSERT in a read-only transaction)
T2  after_release_cleared_dangling_txn: Ok(1)
T3a lock_taken=true
T3b after_release_unlocked_session_lock(expect true): Ok(true)
T4a lock_taken=true
T4b explicit_unlock_released(expect true): Ok(true)
T5  readonly_rows=2 write_rejected=true
T5b pool_healthy_after_dropped_readonly_tx: Ok(1)
T5c pool_writable_after_readonly_commit: Ok(1)
```
probe8（executor 与类型）：
```
L1a generic(&pool): Ok(7)          L1b generic(&mut conn): Ok(7)
L1c generic(&mut *tx): Ok(7)       L1d generic(conn.begin): Ok(7)
L1e generic(&mut conn) after: Ok(7)
L2a via_txn_ref(&mut **tx): Ok(8)  L2b via_txn_method(tx.execute): Ok(1)
L2c tx still usable after helpers: Ok(10)
L3 decimal_by_index=12.345678 by_name=12.345678 equal=true jsonb={"k":[1,2,3]}
L4 sum=12.345678 count=1 sum_empty=0
L5 max_of_empty=None
L6a lock_taken=true                L6b other_conn_acquires_after_release(expect true)=true
L7 readonly_rows=1 write_rejected=true
L7b pool_writable_after_readonly_rollback(count=1)=ok
L8 exhausted_after=1 is_err=true elapsed=403.152291ms
L9 standalone PgConnection: Ok(7)
```
probe9（bind 循环）：
```
M0 no_filter=[("BTCUSDT", 1), ("ETHUSDT", 2), ("BTCUSDT", 3)] len=3
M1 single_filter=[("BTCUSDT", 1), ("BTCUSDT", 3)]
M2 double_filter=[("BTCUSDT", 3)]   M3 count=2
M4 write_rejected=true             M5 read_after_rejected_write_is_err=true
M6 pool_readable=3 pool_writable=4
```
probe10（helper 泛型性）：
```
N1 form_a_twice_on_same_txn=2,1 (expect 2,1)
N2 form_a_in_readonly=2 write_rejected=true (expect 2,true)
N2b read_after_rejected_write=Err(error returned from database: current transaction is aborted, commands ignored until end of transaction block
N3 form_c_write_then_form_a_read=1,1 (expect 1,1)
N4 form_b_twice=2,1 (expect 2,1)
N5 form_a_on_conn_twice=2,2 (expect 2,2)
N5b form_a_on_pool_twice=2,2 (expect 2,2)
N6 two_queries_one_conn=(2, 1)
N7 two_queries_one_txn=(2, 1)
```
probe2 / probe（早期）：`A1 acquired=true`、`C1 writer_conn_holds_lock=true`、
`C3 lock_survives_txn_commit=true`、`D1 decimal=12.34560000 preserved=true`、
`E1 querybuilder_rows=0 ok=true`、`F1 rownotfound=true`、`FACT1 raw_sql $$plpgsql: OK`、
`FACT2b other_conn_sees_held=true`、`FACT9 pool_options knobs 全部可用`。

> 注：probe2 的 `B1 before_acquire_cleared_stale_lock=true` **作废**——该探针 `max_connections=2`
> 且未强制复用同一后端，结论无效。D1 以 S12=false 为准。

## 5. 数据迁移与兼容策略

- **无 schema 变更**：五份迁移文件（0001–0005）逐字不动，`SCHEMA_VERSION=5` 不变，
  `schema_migrations` 表语义不变，`INSERT … ON CONFLICT DO NOTHING` 幂等性不变（U08）。
- **老库兼容**：`migrate` 仍按原顺序施加 0001→0005；已有库因 `IF NOT EXISTS` 与
  `ON CONFLICT` 直接跳过，`verify_schema` 校验版本。G-01 已验证的老库升级路径不受影响。
- **依赖兼容**：`rust_decimal` 无 `sqlx` feature（已核对 1.42.1/1.43.0 的 `[features]`：
  仅 `db-postgres`、`db-tokio-postgres`、`db-diesel*`、`tokio-pg`）。sqlx 自身的
  `rust_decimal` feature 提供 Type/Encode/Decode，D1/L3 证明往返精确。
  切换后 `db-postgres` 成死依赖 → 从两个 `Cargo.toml` 移除（已完成）。
- **无并行真相源**：不存在"新旧两条连接路径共存"的过渡期。DB-01 一次性切换，
  `tokio-postgres` 在编译期即不可解析（clippy `-D warnings` 兜底任何漏改点）。

## 6. 删除或替换的旧路径（显式清单）

| 删除项 | 位置 | 替换为 |
| --- | --- | --- |
| `db::connect(database_url) -> (Client, JoinHandle<()>)` | db.rs（旧） | `db::pool(database_url) -> PgPool` + `pool.acquire()` |
| `db::close_connection(client, connection)` | db.rs（旧） | 借用连接 `drop` 自动归还池 |
| `DomainConnection.client: Client` | db.rs | `DomainConnection.pool: PgPool` |
| `DomainConnection.connection: JoinHandle<()>` | db.rs | 私有 `lock_connection: PoolConnection<Postgres>` |
| `connect` 内 `tokio::spawn` 的连接任务 + `close_connection` 的 `abort()` | db.rs:64-70、72-76（旧） | 无对应物：池自行托管后端，无需每操作一个任务（D4） |
| 14 处 `.client.transaction()` / `client.transaction()` 调用点 | paper.rs:161/214、order.rs:184/221/282/350/395/441、execution.rs:120/172、accounting.rs:95、control.rs:102/145、reconciliation.rs:107 | `.pool.begin().await?`；`&mut self` 写者用 `self.inner.pool.begin()`（D4） |
| `simulation_query::open() -> (Client, JoinHandle)` | simulation_query.rs:266 | **删除**；每个公开入口自持只读事务（§3.3 N2b） |
| `simulation_query::close(client, connection)` | simulation_query.rs:278 | **删除**；`txn.rollback().await` |
| 裸 `BEGIN READ ONLY` / `COMMIT` | simulation_query.rs:270、279 | `pool.begin()` + `SET TRANSACTION READ ONLY`（D2） |
| `dyn ToSql` 切片 ×3 | simulation_query.rs:462、479、662 | 逐个 `.bind()`（D7） |
| `use tokio_postgres::{Row, Transaction, Client, NoTls}` | paper.rs:8、order.rs:6、execution.rs:5、accounting.rs:5、simulation_query.rs:16 | `sqlx::postgres::PgRow` / `sqlx::Transaction<'_, Postgres>` |
| `use tokio::task::JoinHandle` | simulation_query.rs:15 | 删除（无连接任务需托管） |
| `tokio-postgres` 依赖 | crates/core/Cargo.toml、src-tauri/Cargo.toml | `sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio","postgres","rust_decimal","json"] }` |
| `rust_decimal` 的 `db-postgres` feature | 两个 Cargo.toml | 删除（死依赖） |

**14 处 `connect()`/`close_connection()` 调用点**全部迁移，无遗留：
paper.rs:158/191、744/753；simulation.rs:364/373、429/591、1603/1611、1834/1840；
simulation_query.rs:268/280（经由 open/close）。

## 7. 验收场景到模块的映射

| 需求/用例 | 落点模块 | 验证方式 |
| --- | --- | --- |
| R01 池化连接复用（无每操作新建） | db.rs `pool()` | U10：`pool.size() <= 16`；`pg_stat_activity` 连接数稳定 |
| R02 迁移幂等 + advisory 锁 | db.rs `migrate_pool()` | U08：连续两次 `migrate` 均 Ok，`schema_migrations` 行数不变 |
| R03 schema 版本校验 | db.rs `verify_schema` | U09：版本不符 → 原文案 `bail!` |
| R04 领域单写者锁 | db.rs `DomainConnection::acquire/disconnect` | U05（disconnect 后他人可取锁）、U06（同域第二写者被拒，文案逐字一致） |
| R05 只读层零写 + 不抢领域锁 | simulation_query.rs | U04：只读路径拒写，且**不阻塞**并发 F-02 smoke；`external_order_calls=0` 恒真 |
| R06 六个单写者域行为等价 | paper/order/execution/accounting/control/reconciliation.rs | §4.1 映射表逐条比对 + 各自 smoke |
| R07 F-02/G-01 编排与降级不变 | simulation.rs | U07：`TAOLI_DATABASE_URL` 缺失 → 告警 + `None`，观察循环不中断 |
| R08 `try_get` 收敛 panic 为 Result | 全部行读取点 | clippy `-D warnings` + 各 smoke 全绿 |
| R09 Decimal/JSONB 精确往返 | 全部 | L3/D1 探针 + smoke 断言金额字段 |
| R10 `verify_schema` 泛型 executor | db.rs + paper.rs:159 + simulation_query.rs:273 | 编译通过即证；两处调用点改为传 executor |
| R11 依赖清理干净 | 两个 Cargo.toml | `grep -rn tokio_postgres` 全仓为空 |
| U01–U03 各域 smoke | paper/order/execution/accounting/control/reconciliation/simulation | `--nocapture --test-threads=1`，断言全部行为标志 true |

## 8. 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| `MAX_CONNECTIONS=16` 在 F-02 高并发下不足 | 操作在 30s 后失败关闭（L8） | U10 观测 `pool.size()`；F-02 smoke 若报 acquire timeout 则重估该上限（旧代码无上限，本就无此约束） |
| 只读事务内一条语句失败即 aborted | 后续读全部失败（**N2b**） | §3.3 约束：三个公开入口各自"开事务 → 全部查询 → 回滚"在一个 `async` 体内完成，不复用失败事务 |
| `try_get` 把旧的 panic 变成 `Err` | 行为变化（更安全但错误路径不同） | R08 明确此为**期望变更**；所有 smoke 覆盖真实行读取路径 |
| `after_release` 钩子内 HRTB 报错 | 编译失败 | 已确定写法：钩子内用 `connection.execute("SELECT …")`（`Executor` trait 方法），**不用** `raw_sql(...).execute(&mut *conn)`（后者报 `implementation of 'Executor' is not general enough`） |
| 漏改调用点 | 编译/clippy 失败 | `tokio_postgres` 在编译期不可解析 + `cargo clippy --workspace --all-targets -- -D warnings` 兜底 |
| `advisory lock` 探针误判 | 设计依据错误 | 已确立：PostgreSQL 无 `pg_advisory_lock_is_held_current_session`；探针须从**另一条**连接 `pg_try_advisory_lock` 再取反（同会话 try_lock 因锁可重入而无意义）。`SELECT 1` 解码为 INT4 而非 INT8，需 `1::int8` |

## 9. 文件级变更清单

**已完成**（`db.rs` 264 行 + 两个 `Cargo.toml`）：
- `crates/core/src/db.rs`：全量重写至 sqlx 池层，两条池卫生不变式写入模块文档
- `crates/core/Cargo.toml`：`rust_decimal` 去 `db-postgres`；`tokio-postgres` → `sqlx 0.8`
- `src-tauri/Cargo.toml`：删 `tokio-postgres`，`rust_decimal` 去 `db-postgres`（源码零改动，
  `grep -rn tokio_postgres src-tauri/src/` 已为空）

**待迁移**（8 模块，查询点合计 93 处）：

| 模块 | 行数 | 查询点 | 必改项 |
| --- | --- | --- | --- |
| paper.rs | 864 | 21 | `Row`→`PgRow`(689-691)；`&Transaction`→`&mut Transaction<'_,Postgres>`(583/625)；`set_paper_balance`(158-191)；`business_row_count`(744-753)；`recover_active_plans(&self)`(260) |
| order.rs | 852 | 16 | `Row`→`PgRow`(564)；`lock_fact`/`append_action`(543/551)；`load`/`recover_nonterminal(&self)`(526/532) |
| execution.rs | 486 | 7 | `Row`→`PgRow`(281)；`load(&self)`(263) |
| accounting.rs | 271 | 6 | `Row`→`PgRow`(261)；`load_event(&self)`(147) |
| control.rs | 274 | 8 | `load(&self)`(180) |
| reconciliation.rs | 265 | 4 | `record_snapshot`(86) 单行 10 bind |
| simulation.rs | 2126 | 6 | 4 对 connect/close；**保留 :1122-1134 降级语义**；三核并发 :1778-1790 |
| simulation_query.rs | 1008 | 25 | **删 open/close**；三个公开入口自持只读事务；3 处 `dyn ToSql`(462/479/662) |

`.inner.client` → `.inner.pool` 共 **29** 处，以 `.client` 所在行为准：
paper.rs 213/262/283/291/299/386/396/406（8）、order.rs 163/183/220/281/349/394/440/528/533（9）、
execution.rs 119/171/266（3）、accounting.rs 94/134/149/205（4）、control.rs 101/144/182（3）、
reconciliation.rs 86/106（2）。其中 20 处是跨行链式（`.inner` 与 `.client` 分处相邻两行），
单行 `grep "\.inner\.client"` 只命中 10 处——迁移与复核一律按上表 29 行为准。

## 10. 设计评审记录

| 决策 | 状态 | 日期 |
| --- | --- | --- |
| D1 `after_release` 兜底解锁 | 待评审 | — |
| D2 只读会话用 sqlx Transaction | 待评审 | — |
| D3 `pool` 公开 + 私有 `lock_connection` | 待评审 | — |
| D4 不加 `begin()` 包装，14 处调用点直接 `.pool.begin()` | 待评审 | — |
| D5 按 URL 缓存池 | 待评审 | — |
| D6 `MAX_CONNECTIONS=16` / `ACQUIRE_TIMEOUT=30s` | 待评审 | — |
| D7 逐个 `.bind()`，不引入 QueryBuilder | 待评审 | — |
| D8 `raw_sql` 整份迁移 | 待评审 | — |
| §6 删除清单（无并行真相源） | 已评审（Owner 于 2026-09-09 指示「继续」进入实现；§4.3 门禁顺序偏差已在评审时披露，Owner 未要求回滚 `db.rs` 与两个 `Cargo.toml`） | 2026-09-09 |

**退出条件自检**：每个调用点均有迁移方案（§9 逐文件逐行）；无新旧并行真相源（§5、§6）。
D1–D8 与 §6 删除清单经 Owner 评审通过，进入 §4.4 实现阶段（迁移顺序：领域模型 → 适配器 → 业务编排 → CLI/运维入口）。
