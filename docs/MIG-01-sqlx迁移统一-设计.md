# MIG-01 迁移机制交由 sqlx 接管：设计文档

文档状态：待所有者评审（2026-09-10）。对应需求：`docs/MIG-01-sqlx迁移统一-需求.md`。
前置迭代：DB-01（池化）、PERF-01（就绪库单点与读路径索引）。

**流程偏差**：本设计系事后补正，内容与已交付代码逐条对应。与 PERF-01 的差别见需求文档「流程偏差声明」。

---

## 0. 决策记录（所有者前置选择）

本迭代涉及**已发布契约的替换**，方向由所有者从三段式选项中前置选定，不是 AI 自行裁定：

| 决策点 | 选项与代价 | 所有者选择 |
|---|---|---|
| 统一程度 | ① 只用 sqlx 做来源、保留 `schema_migrations`（零契约变更，但保留自建 apply 且 CLI 不可用）② **全量交给 sqlx**（`_sqlx_migrations`，最标准但需改写 4 条已发布需求 + 存量库引导）③ 仅收敛记账样板（改动最小，但无校验和） | **② 全量交给 sqlx** |
| 存量库处置 | ① **先备份 + 引导避免重跑** ② 直接线上验证不备份 ③ 另起临时库验证 | **① 先备份 + 引导** |

代价已在选项中明示并被接受：4 条已发布需求（G01-R10、DB01-R02、DB01-R03、PERF01-R01/R02）需同步改写，已在需求文档 §5.2 与手册中逐条登记。

## 1. 模块边界

```
crates/core/Cargo.toml                    ← sqlx 增 migrate + macros feature
crates/core/src/db.rs                     ← 删除自建迁移器；MIGRATOR + 采纳 + 派生版本号
crates/core/migrations/*.sql              ← 剥离各文件尾部记账语句（0001 另删旧表 CREATE）
crates/core/migrations/0007_*.sql         ← 新增：DROP TABLE schema_migrations
crates/core/src/paper.rs / simulation.rs  ← SCHEMA_VERSION 常量 → schema_version() 函数
```

**边界不变式**：`db.rs` 仍是唯一连接层；迁移仍只在 `ready_pool` 首次访问某 `database_url` 时执行一次；应用代码不直接读写 `_sqlx_migrations`。新增约束是：**迁移文件的任何修改都必须通过新增版本文件完成**——已应用文件的改动现在会硬失败，这是有意的。

## 2. 唯一数据契约

### 2.1 变更后的契约

```rust
// db.rs
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");   // 编译期嵌入整个目录
pub(crate) fn schema_version() -> i64;                        // = 嵌入文件集的最大版本
pub(crate) async fn ready_pool(database_url: &str) -> Result<PgPool>;   // 不变
pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()>;          // 内部换实现
pub(crate) async fn migrate(database_url: &str) -> Result<()>;          // 不变（委托 ready_pool）
```

**删除的契约**：`SCHEMA_VERSION: i64` 常量、`verify_schema` 函数、`migrations` 文件清单、`advisory_key("schema-migration", …)` 迁移锁、`schema_migrations` 表。

**保留的契约**：`pool`、`ready_pool`、`migrate`、`DomainConnection::{acquire,disconnect}`、`advisory_key`（领域锁仍在用）、`hex_digest`、`validate_id`、`db_time`、`to_i64`、`to_u64`，以及全部对外命令签名。

### 2.2 记账表映射

| 自建 `schema_migrations` | sqlx `_sqlx_migrations` | 采纳时的转换 |
|---|---|---|
| `version BIGINT` | `version BIGINT` | 直接对应 |
| `applied_at_ms BIGINT` | `installed_on TIMESTAMPTZ` | `to_timestamp(applied_at_ms / 1000.0)` |
| — | `description TEXT` | 取自嵌入文件名的描述（`0001_paper_core.sql` → `paper core`） |
| — | `success BOOLEAN` | 恒 `TRUE`（能出现在旧表即已成功） |
| — | `checksum BYTEA` | 取自嵌入文件的 SHA-384 |
| — | `execution_time BIGINT` | `-1`（未知；sqlx 自身在测量前也用该值） |

## 3. 状态、错误与降级语义

| 场景 | 旧行为 | 新行为 | 依据 |
|---|---|---|---|
| 全新库 | 建 `schema_migrations`，逐文件执行 + 逐文件 INSERT | 建 `_sqlx_migrations`，逐文件在事务内执行并写 checksum | D2 |
| 已应用文件被改动 | **静默跳过**（`IF NOT EXISTS` 掩盖） | `MigrateError::VersionMismatch` 硬失败 | D2 |
| 半应用 | 无检测 | `success=false` 触发 `Dirty` 错误 | D2 |
| 迁移锁 | `advisory_key("schema-migration","b01-paper-core")` 的会话锁 + 显式解锁 | sqlx 按**库名哈希**的 advisory lock，`Migrator::locking` 默认开启 | D3 |
| 迁移事务 | 无（每文件 `raw_sql` 直接执行） | 每文件 + 其记账在同一事务；文件以 `-- no-transaction` 开头可退出（本项目未使用） | D2 |
| 迁移失败 | `Err` + 文件标签上下文 | `MigrateError::ExecuteMigration(e, version)` 携带版本号 | — |
| 旧库版本高于本二进制 | `verify_schema` 比对 `MAX(version) == SCHEMA_VERSION` 后 `bail!` | **采纳前**逐个版本校验是否存在于嵌入文件集，否则 `bail!` | D4 |
| schema 版本不符（新形态） | 同上 | 由 sqlx 的 `VersionMissing` / `VersionMismatch` 覆盖；本迭代不额外实现 | D6 |
| `DROP TABLE IF EXISTS schema_migrations` 重复执行 | — | 无副作用（`IF EXISTS`） | D5 |

**D4 的顺序性是关键**：校验必须发生在 `MIGRATOR.run` **之前**。否则旧版二进制会在新库上先应用「自己版本的 `0007 DROP TABLE`」。实测确认拒绝了含 version 99 的库且未执行任何迁移（S-MIG-5）。

## 4. 关键决策

| 编号 | 决策 | 依据 |
|---|---|---|
| D1 | 用 `sqlx::migrate!("./migrations")` 而非运行时 `Migrator::new(Path)` | 编译期嵌入使二进制自包含（部署无需携带 SQL 目录），且路径错误在编译期暴露。代价：新增迁移需重新编译——本项目本就随应用启动迁移，无额外成本 |
| D2 | 只取 `migrate` + `macros` 两个 feature，不启用 `derive`/`query!` | `migrate` feature 启用 `sqlx-macros?/migrate`（可选依赖），`migrate!` 宏需要 `macros`。编译期 SQL 校验（`query!`）需 live DB 或离线缓存，属独立决策（需求 §4） |
| D3 | 接受 sqlx 的锁 key，不做兼容层 | 锁只在迁移期间持有（毫秒级），且旧锁与 sqlx 锁的持有时刻不重叠——运行时并发的两个进程要么都用新代码（同一 key），要么都是迁移已完成状态。无实际风险，故不写兼容 shim |
| D4 | 采纳前逐个版本校验（保留 `verify_schema` 的安全性质） | 否则「旧二进制 + 新库」会先应用自己的 `0007` 再失败。这是把被删函数的安全性质**搬进新路径**，而非丢弃 |
| D5 | 用独立迁移 `0007` 删旧表，不在引导里隐式 `DROP` | 删表是可审计的 schema 变更，应当在版本账本中留下痕迹；且在 `MIGRATOR.run` 前 drop 会打破「采纳 → 应用」的顺序依赖 |
| D6 | 采纳顺序中**先创建 `_sqlx_migrations` 再判空** | 空表检查需要表存在。`CREATE TABLE IF NOT EXISTS` 的 DDL 与 sqlx 自己的 `ensure_migrations_table` 逐字一致（列名、类型、`DEFAULT now()` 均对齐），使采纳行落在 sqlx 随后读取的同一张表上 |
| D7 | 采纳仅在 `_sqlx_migrations` **为空**且旧表存在时执行 | 已是 sqlx 形态的库不应被旧表内容干扰；旧表在 0007 后不存在，二者互斥 |
| D8 | `execution_time = -1` 标记采纳行 | 这是 sqlx 自身的「尚未测量」哨兵值，用它可让运维区分采纳行与真实应用行，无需新增列 |
| D9 | `installed_on` 从 `applied_at_ms` 换算而非取当前时间 | 删表不应丢失历史。实测：`1700000000000 + v*1000` 精确还原为 `2023-11-14 22:13:2{v}+00` |
| D10 | 0007 不提供 down 迁移 | 旧表内容在采纳后已全部进入 `installed_on`，无信息可还原；项目本就无 down 迁移策略（回滚靠备份） |

## 5. 删除或替换的旧路径（显式清单）

| 删除项 | 位置 | 替换为 |
|---|---|---|
| 6 个 `include_str!` 迁移常量 | `db.rs:29-38`（旧） | `static MIGRATOR` |
| 逐文件标签数组 `[("B-01", …), …]` | `db.rs:129-136`（旧） | `MIGRATOR.run` |
| `advisory_key("schema-migration", "b01-paper-core")` 锁及其取放 | `db.rs`（旧） | sqlx `Migrator` 的库级锁 |
| `pub(crate) const SCHEMA_VERSION: i64 = 6` | `db.rs:28`（旧） | `pub(crate) fn schema_version()`（由文件集推导） |
| `pub(crate) async fn verify_schema<'e, E>` | `db.rs:151-171`（旧） | 采纳路径内的逐版本校验；运行期版本一致性由 sqlx 判定 |
| `DomainConnection::acquire` 内的 `verify_schema(&pool)` | `db.rs:250`（旧） | `ready_pool(database_url)`（迁移与校验合一） |
| `schema_migrations` 表及其逐文件 `INSERT … ON CONFLICT DO NOTHING` | 0001–0006 各文件尾部（旧） | `_sqlx_migrations`；旧表由 0007 删除 |
| 0001 内的 `CREATE TABLE IF NOT EXISTS schema_migrations …` | `0001_paper_core.sql:2-5`（旧） | 删除（重跑 `IF NOT EXISTS` 会与 0007 语义竞争） |
| `paper.rs` / `simulation.rs` 的 `SCHEMA_VERSION` 导入与使用（4 处） | 各文件（旧） | `db::schema_version()` |

**无并行真相源**：`schema_migrations` 在 0007 后被物理删除，不存在两张表并存；`migrate()` 与 `ready_pool()` 仍是委托关系。

## 6. 实库证据基线

全部在 `taoli-postgres`（127.0.0.1:55432）实测。一次性驱动 `crates/core/examples/mig_probe.rs`（**验证后已删除**），它走 `get_simulation_overview` / `get_simulation_runs` / `get_simulation_run_detail` 三个公开入口，因此同时覆盖生产调用路径与迁移路径。

**S-MIG-1 全新库**（`taoli_mig_probe` 空库）

```
first_call_ms=338    second_call_ms=29    stable=true
runs_total=0 rows=0  missing_run_is_err=true

_sqlx_migrations: 1..7 全部 success=t    measured(execution_time>0)=t
legacy schema_migrations: ABSENT
tables=22   indexes=47   triggers=11      ← 与存量库收敛到完全相同的结构
```

**S-MIG-2 采纳保真**（构造 `applied_at_ms = 1700000000000 + v*1000`）

```
version |      installed_on           | execution_time | adopted
      1 | 2023-11-14 22:13:21+00      |             -1 | t
      2 | 2023-11-14 22:13:22+00      |             -1 | t
      …
      6 | 2023-11-14 22:13:26+00      |             -1 | t
      7 | 2026-09-10 09:40:05.977912+00 |       3191125 | f     ← 真实应用，时间已测量
```

**S-MIG-3 v4 升级**（由 0001–0004 + 旧表 1..4 构造）

```
versions=1,2,3,4,5,6,7
adopted(exec=-1)=1,2,3,4        ← 只采纳旧表已有的；5/6/7 为真实应用
tables=22                       ← 与全新库一致
legacy=ABSENT
```

**S-MIG-4 校验和**

```
UPDATE _sqlx_migrations SET checksum='\xdeadbeef' WHERE version=3;
→ Error: failed to apply database migrations
  Caused by: migration 3 was previously applied but has been modified
```

**S-MIG-5 版本超前**

```
旧表含 version 99 →
→ Error: unsupported B-01 schema version: expected at most 7, found 99
（且未应用任何迁移）
```

**S-MIG-6 存量库（taoli，由运行中的 `tauri dev` 进程先行迁移）**

```
_sqlx_migrations: 1..7 全部 success=t
  1–4  installed_on 2026-09-09T07:12Z，execution_time=-1（采纳自 PERF-01 之前的旧表）
  5    installed_on 2026-09-09T12:58Z，execution_time=-1（采纳）
  6    installed_on 2026-09-10T08:52Z，execution_time=-1（采纳）
  7    installed_on 2026-09-10T09:38Z，execution_time=3426625（真实应用）
legacy schema_migrations: ABSENT
数据完整性：simulation_runs 564→578、audit_events 694→708、trade_facts 928→948、
           balance_snapshots 3389→3473、order_action_facts 3507→3581
           全部增量来自迁移校验期间 app 继续运行，无丢失；tables=22 indexes=47 triggers=11
```

**关于备份**：`data/backup/taoli-pre-mig01.sql`（4.4 MB，`pg_dump --no-owner --no-acl`）。该备份**晚于**实际迁移——`tauri dev` 在所有者机器上运行中，本二进制编译完成即被其重新加载并完成迁移（见任务文档 §3 时间线）。备份仍保留作为回滚点，但它记录的是迁移**之后**的状态；实际迁移未重跑任何文件（旧表已不存在，采纳路径读到 1–6 后仅应用 0007），故无数据风险。

## 7. 风险与缓解

| 风险 | 影响 | 缓解 / 状态 |
|---|---|---|
| R-1 已应用迁移文件被意外修改 | 硬失败，应用无法启动 | 这是**有意**的期望行为（R04）。代价是改文件必须先删 `_sqlx_migrations` 对应行或新增版本文件；文档在 `db.rs` 与 0007 注释中说明 |
| R-2 `-- no-transaction` 使半应用不可检测 | 该模式下文件与记账不在同一事务 | 项目全部文件均为 `Simple`/`no_tx=false`；新增迁移不得以该指令开头（约定，未加编译期强制） |
| R-3 `Dirty` 需人工处置 | 半应用后应用无法自动继续 | 需人工检查后修复。**运维动作未文档化到 runbook**，记录为待补 |
| R-4 采纳逻辑只在旧表存在时生效 | 若旧表被提前手工删除，版本账本为空 → 全部文件重跑 | 全部迁移幂等（`IF NOT EXISTS`/`OR REPLACE`/`ON CONFLICT`），重跑是安全的但会重写触发器；实测路径未覆盖「旧表已删 + `_sqlx_migrations` 空」组合 |
| R-5 `to_regclass` 依赖 `search_path` | 异常 search_path 下可能漏判 | 查询显式使用 `'public.schema_migrations'`，不依赖默认路径 |
| R-6 备份晚于迁移 | 备份不是迁移前快照 | 已核实迁移未重跑任何文件、数据无丢失（§6），风险实际未发生；但**流程上应「先停应用、再备份、再迁移」**，本轮未做到，记录为流程教训 |
| R-7 索引版本与 sqlx 的兼容下限 | 未来 sqlx 升级可能变更 `_sqlx_migrations` 结构 | 采用 `semver-compatible` 约束（`0.8`）；升级 sqlx 时需按 S-MIG-1/2 回归 |

## 8. 需求到模块的映射

| 需求 | 落点 | 验证 |
|---|---|---|
| R01 来源 | `db.rs` `MIGRATOR` | S-MIG-1；`grep include_str! db.rs` 为空 |
| R02 版本单一来源 | `db.rs::schema_version()` | `grep SCHEMA_VERSION` 仅剩归档域同名常量 |
| R03 记账 | `_sqlx_migrations`；0001–0006 尾部删除 | S-MIG-1 |
| R04 校验和 | sqlx 内建 | S-MIG-4 |
| R05 一致性 | sqlx 内建（事务） | 代码审查 + 全部文件 `no_tx=false` |
| R06 存量引导 | `db.rs::adopt_legacy_bookkeeping` | S-MIG-2/S-MIG-3/S-MIG-6 |
| R07 版本超前 | `adopt_legacy_bookkeeping` 内的逐版本校验 | S-MIG-5 |
| R08 旧表清理 | `0007_drop_legacy_schema_migrations.sql` | S-MIG-1/2/3：均 ABSENT |
| R09 空库不受引导影响 | 引导仅在旧表存在时执行 | S-MIG-1 |
| R10 对外行为不变 | `migrate`/`ready_pool` 签名；`acquire` 改走 `ready_pool` | S-MIG-6 冷热对比 |
| R11 非记账 SQL 不变 | 逐文件 diff | `git diff` 仅含尾部删除 |
| R12 文档同步 | G-01/DB-01/PERF-01 需求 + 手册 | 见任务文档 §4 |
