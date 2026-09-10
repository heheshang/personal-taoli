# MIG-01 迁移机制交由 sqlx 接管：任务文档

文档状态：待所有者评审（2026-09-10）。实现顺序与验收见需求/设计文档；本文件为文件级执行清单与验证证据留存。

**流程偏差**：本任务文档系事后补正，按已交付改动逐文件回溯整理。

## 1. 文件级改动清单

### 1.1 `crates/core/Cargo.toml`

`sqlx` feature 增 `migrate` + `macros`：

```toml
sqlx = { version = "0.8", default-features = false, features = [
    "runtime-tokio", "postgres", "rust_decimal", "json", "migrate", "macros",
] }
```

`migrate` 启用 `sqlx-core/migrate` 与可选的 `sqlx-macros?/migrate`；`migrate!` 宏本体需要 `macros`。未启用 `derive`/`query!`（需求 §4）。

### 1.2 `crates/core/src/db.rs`

- 导入：`use sqlx::migrate::Migrator;`；移除 `PgExecutor` 与 `Row`；`Executor` 保留（`adopt_legacy_bookkeeping` 的 `CREATE TABLE` 与 `pool` 的 `after_release` 仍在用）。
- 删除：`SCHEMA_VERSION` 常量、6 个 `include_str!` 常量。
- 新增：`static MIGRATOR: Migrator = sqlx::migrate!("./migrations");`、`pub(crate) fn schema_version() -> i64`。
- `ready_pool`：注释改写为与实现一致的并发论证（两路并发首次调用都安全：采纳用 `ON CONFLICT DO NOTHING`，`MIGRATOR.run` 由 sqlx 库级锁串行）。
- `migrate_pool`：改为 `adopt_legacy_bookkeeping(pool)` + `MIGRATOR.run(pool)`。
- 新增 `async fn adopt_legacy_bookkeeping(pool: &PgPool) -> Result<()>`：
  1. `CREATE TABLE IF NOT EXISTS _sqlx_migrations (…)`（列与 `sqlx-postgres` 自身的 `ensure_migrations_table` 逐字一致）；
  2. `_sqlx_migrations` 非空 → 直接返回；
  3. `to_regclass('public.schema_migrations')` 判旧表存在性，不存在 → 返回；
  4. 读旧表全部 version，逐个 `MIGRATOR.version_exists`，任一不存在即 `bail!`；
  5. 遍历 `MIGRATOR.iter()`，旧表有记录者插入 `_sqlx_migrations`，`installed_on = to_timestamp(applied_at_ms/1000.0)`、`success = TRUE`、`checksum = 嵌入文件的 SHA-384`、`execution_time = -1`、`ON CONFLICT DO NOTHING`。
- 删除 `verify_schema` 函数。
- `DomainConnection::acquire`：`pool(database_url)` + `verify_schema(&pool)` → `ready_pool(database_url)`（一处同时保证连接与 schema 就绪）。

### 1.3 迁移文件

- `0001`–`0006`：删除尾部 `INSERT INTO schema_migrations(version, applied_at_ms) VALUES (N, …) ON CONFLICT DO NOTHING;`。
- `0001` 额外删除开头的 `CREATE TABLE IF NOT EXISTS schema_migrations (…);`（旧表所有权已转移；若保留，0007 删表后重跑 `IF NOT EXISTS` 会重新建表）。
- 除上述两类语句外，**其余 SQL 逐字未动**：表结构、CHECK 约束、索引、`CREATE OR REPLACE FUNCTION reject_audit_event_mutation()`、不可变触发器全部保持。
- 新增 `0007_drop_legacy_schema_migrations.sql`：仅 `DROP TABLE IF EXISTS schema_migrations;` + 说明注释。

### 1.4 版本号调用点

- `crates/core/src/paper.rs`：导入改 `schema_version`，`schema_version: SCHEMA_VERSION` → `schema_version: schema_version()`。
- `crates/core/src/simulation.rs`：移除 `use crate::db::SCHEMA_VERSION;`，导入 `schema_version`，3 处字段赋值同改（含 1 处测试内构造）。

## 2. 验证证据

### 2.1 五门禁

```bash
cargo fmt --all -- --check                              # PASS
cargo test --workspace                                  # 192 core + 8 tauri-lib passed; 0 failed
cargo clippy --workspace --all-targets -- -D warnings   # 零警告
cargo build --workspace --release                       # Finished `release` profile in 2m 15s
npm run build                                           # ✓ built in 2.53s
```

现有 192+8 测试**全部继续通过且未新增**。本轮不新增测试：改动是基础设施替换，其可观察性质（版本账本一致性、校验和、采纳）无法在无 DB 的纯逻辑测试中验证，强行加 mock 只会测到 mock 本身。因此改以 6 条真实 DB 路径为证据（设计文档 §6）。

### 2.2 实库路径（6 条，全部实测）

一次性驱动 `crates/core/examples/mig_probe.rs`（**验证后已删除**）走三个公开读入口，因此同时覆盖生产调用路径。

| 场景 | 结果 |
|---|---|
| 全新库 | 1..7 全部 `success`、`execution_time > 0`；22 表/47 索引/11 触发器；旧表 ABSENT；冷 338ms → 29ms |
| 采纳保真 | `installed_on` 精确还原 `applied_at_ms`（`1700000000000+v*1000` → `2023-11-14 22:13:2{v}+00`）；采纳行 `execution_time = -1` |
| v4 升级 | 1–4 采纳、5–7 应用；tables=22；旧表 ABSENT |
| 校验和 | 篡改 → `migration 3 was previously applied but has been modified` |
| 版本超前 | 旧表含 99 → `expected at most 7, found 99`，且未应用任何迁移 |
| 存量库 | 1–7 全部 `success`；1–6 为采纳、7 为真实应用；数据无丢失（见设计 §6） |

### 2.3 未执行的验证（明确记录）

- **不做 `-- no-transaction` 迁移**：本轮未验证该模式下的半应用检测（项目无此类文件）。
- **不做回滚演练**：`0007` 无 down 迁移，回滚依赖 `data/backup/`，未实际执行恢复。
- **`Dirty` 状态未实测**：半应用检测由 sqlx 保证，未构造真实半应用。
- **GUI 渲染未做像素级验证**：与本轮改动无关，沿用 PERF-01 的记录。

## 3. 执行时间线（重要，影响备份有效性）

本轮出现一次**执行顺序偏差**，如实记录：

1. AI 完成代码改动并编译（`cargo build`，17:34 前后）。
2. 所有者机器上运行中的 `tauri dev`（PID 15309，起始 14:28）**监听到二进制变化并重新加载**，其 `setup` → 迁移路径随即对 `taoli` 库执行了迁移（`_sqlx_migrations` 中 0007 的 `installed_on = 2026-09-10 09:38:55Z`）。
3. AI 于 17:39 执行 `pg_dump` 备份，此时旧表已被删除 —— 备份为迁移**之后**的快照。

**后果评估**：迁移未重跑任何文件（旧表此时不存在，采纳路径读到 1–6 后仅应用 0007），故无数据丢失或结构破坏。已核实：`tables=22 indexes=47 triggers=11` 与全新库收敛一致，各业务表行数只增不减（增量来自迁移期间 app 继续运行）。

**流程教训**：正确顺序应为「停应用 → 备份 → 迁移」或至少在迁移前做备份。已记入设计文档 §7 R-6。本轮不因此回滚，因为破坏性后果经核实未发生；但若同一场景发生在有真实资金数据的库上，备份将失去其回滚价值。

## 4. 文档同步

| 文档 | 变更 |
|---|---|
| `docs/G-01-模拟套利仪表盘-需求.md` | G01-R10 与 S-G01-1：记账表改述；「明确不做」中「`SCHEMA_VERSION` 之下」改述 |
| `docs/DB-01-数据库连接层sqlx池化重构-需求.md` | DB01-R05 与 U08：标注手写迁移器已被取代；「不新增业务能力」行保留为当时范围记录 |
| `docs/PERF-01-读路径性能与重复代码收敛-需求.md` | 新增 §4bis：登记 MIG-01 对 R01/R02 的替换关系（可观察结论仍成立，不标失效）；其余 `schema_migrations` 措辞改述 |
| `docs/PERF-01-读路径性能与重复代码收敛-设计.md` | 同上口径的追加说明 |
| `docs/个人加密货币套利系统-AIDLC迭代执行手册.md` | §6.7 追踪矩阵、§10 发布记录、§11 下一轮唯一入口 |
| `docs/个人加密货币套利系统-架构与需求设计.md` | §19.1 当前实现状态追加 MIG-01 条目与链接 |
| `docs/DB-01-...-设计.md` | §5 迁移策略改述为 sqlx 形态 |

**未改动**：G-01/DB-01/PERF-01 的历史**任务与设计**文档中描述当时实现的正文（它们是该轮事实记录），仅在其中追加 MIG-01 的口径说明。

## 5. 清理项

| 项 | 状态 |
|---|---|
| `crates/core/examples/mig_probe.rs` | 已删除 |
| `crates/core/examples/` 空目录 | 已删除 |
| 探针库 `taoli_mig_probe` | 已 DROP |
| `data/backup/taoli-pre-mig01.sql` | **保留**（回滚点；`/data/` 已在 `.gitignore` 内，不进版本库） |

## 6. 剩余风险与后续入口

剩余风险见设计文档 §7。其中 R-3（`Dirty` 后的人工处置动作未进 runbook）与 R-6（备份时序）建议在 D 阶段运维文档中补齐。

后续独立项（本轮明确不做）：`sqlx migrate` CLI 接入部署流程、`query!` 编译期校验与离线缓存、down 迁移策略。
