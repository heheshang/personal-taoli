# MIG-01 迁移机制交由 sqlx 接管：需求文档

文档状态：实现与验证已完成，待所有者确认（2026-09-10）。立项编号 **MIG-01**：与 `DB-01`、`PERF-01`、`UI-01` 同为**段外特批的横切迭代**（不占 A–G 业务阶段编号，§11 业务入口不变）。

**决策来源**：所有者明确选择「全量交给 sqlx（`_sqlx_migrations`）」与「先备份再迁移，用引导避免重跑」（见 `docs/MIG-01-sqlx迁移统一-设计.md` §0 决策记录）。这是本迭代唯一一次需要**改动已发布契约**的变更，影响面在使用手册中逐条登记。

**流程偏差声明**：需求确认、设计评审与实现在同一会话内完成，三份文档系事后补正；与 PERF-01 不同之处在于，本迭代的**方向与两个关键取舍由所有者前置选择**（三段式选项，含各自代价说明），其余实现细节由 AI 决定。人工门禁中设计评审与发布签收仍待所有者完成。

## 1. 目标

迁移机制此前是手写的：`db.rs` 内 6 个 `include_str!` 常量 + 一个硬编码数组、自定义 advisory key（`advisory_key("schema-migration", "b01-paper-core")`）、自建 `schema_migrations(version, applied_at_ms)` 表、`SCHEMA_VERSION: i64` 常量、以及 `verify_schema` 函数；每个迁移文件还要在尾部自己 `INSERT INTO schema_migrations` 并把版本号手抄一遍。

这套机制有四个可指出的问题：

1. **版本号抄写三处**：文件名、文件尾部 INSERT、`SCHEMA_VERSION` 常量。任意一处不同步都是静默故障。
2. **无内容校验**：迁移文件被改动后，旧库上重跑只是 `IF NOT EXISTS` 静默跳过，不会报错。
3. **无半应用检测**：某文件执行到一半失败，下次运行的唯一后果是「看起来已应用」。
4. **不是任何工具认识的标准形态**：`sqlx migrate` CLI、`sqlx prepare`、其他 sqlx 项目都无法对接。

本迭代把迁移的**来源、排序、记账、锁、事务、校验**全部交给 sqlx 0.8.6 的 `Migrator`，并删除自建机制。业务语义零变更。

## 2. 需求

| 编号 | 要求 | 验收结果 |
|---|---|---|
| MIG01-R01 | 迁移来源由 `sqlx::migrate!("./migrations")` 编译期嵌入整个目录；`db.rs` 不再出现任何 `include_str!` 迁移常量、文件清单或逐文件标签数组 | 新增迁移只需新增一个文件；`grep include_str! crates/core/src/db.rs` 为空 |
| MIG01-R02 | 版本号单一来源：对外报告的 `schema_version` 由嵌入文件集的**最大版本推导**，不得手写常量 | `grep -rn "SCHEMA_VERSION" crates/core/src` 仅剩归档域的同名常量（不同的机制）；迁移版本无硬编码 |
| MIG01-R03 | 记账由 sqlx 的 `_sqlx_migrations` 承担（`version`/`description`/`installed_on`/`success`/`checksum`/`execution_time`）；自建 `schema_migrations` 表与其逐文件 INSERT 全部删除 | 旧表最终不存在；每个文件尾部不再有记账语句 |
| MIG01-R04 | 内容校验：已应用迁移文件被改动后必须**硬失败**，不得静默跳过 | 篡改 `_sqlx_migrations.checksum` 后运行报 `migration N was previously applied but has been modified` |
| MIG01-R05 | 一致性：每个迁移文件与其记账写入在同一事务内，半应用状态可被识别 | 由 sqlx 保证（`success=false` → `Dirty` 错误）；本项目全部文件均为 `Simple` 类型、`no_tx=false` |
| MIG01-R06 | 存量库引导：迁移到 sqlx 记账表时不得重跑任何已应用文件，且不丢失已记录的原始应用时间 | 由 `schema_migrations.applied_at_ms` 采纳为 `_sqlx_migrations.installed_on`；仅当 `_sqlx_migrations` 为空且旧表存在时执行 |
| MIG01-R07 | 自我保护保持：旧库版本高于本二进制已知版本时必须失败关闭 | 旧表含 version 99 时报 `unsupported B-01 schema version: expected at most 7, found 99` |
| MIG01-R08 | 旧表清理经独立迁移（`0007`）而非隐式 drop；删表前必须先完成采纳 | 服务启动序列为：创建 sqlx 表 → 采纳旧表 → 运行迁移（含 `0007` DROP） |
| MIG01-R09 | 空库路径不受引导逻辑影响 | 全新库首次调用后 `_sqlx_migrations` = 1..7 全部 `success`，旧表从未创建 |
| MIG01-R10 | 对外行为不变：`migrate(&str)`/`ready_pool(&str)` 签名不变；每个 `database_url` 仍只迁移一次；`DomainConnection::acquire` 仍保证 schema 就绪 | 调用点零改动（仅 `SCHEMA_VERSION` 常量改为函数调用）；冷热耗时差异保持 |
| MIG01-R11 | 不改动任何迁移文件里**除记账语句以外的** SQL：表结构、约束、索引、不可变触发器、`CREATE OR REPLACE FUNCTION` 逐字不变 | 逐文件 diff 仅含尾部 INSERT 删除（0001 另含旧表 CREATE 删除） |
| MIG01-R12 | 已发布文档中依赖旧记账表的断言全部同步，不得留失效断言 | G01-R10、DB01-R02/R03、PERF01-R01/R02 逐条更新并登记替换关系 |

## 3. 正常、边界、失败与恢复场景

| 场景 | 输入 | 必须观察到 |
|---|---|---|
| 正常 | 空库首次启动 | 七个迁移全部应用并记录；`success` 全真；旧表从未存在 |
| 正常 | 存量 sqlx 库（version 7） | 幂等；不重跑任何文件；引用已删表 `DROP TABLE IF EXISTS` 不报错 |
| 正常 | 半应用（`success=false`） | sqlx 报 `Dirty`，拒绝继续 |
| 边界 | 旧库恰为 v4（G-01 时代的库） | 采纳 1–4 并**保留原始 `installed_on`**；应用 5/6/7；不重跑 1–4 |
| 边界 | 旧库为 v6（PERF-01 时代的库） | 采纳 1–6；应用 7 |
| 失败 | 已应用迁移文件被改 | `VersionMismatch` 硬失败 |
| 失败 | 旧库版本高于本二进制 | 采纳前即 `bail!`（`expected at most 7`）。**不得**先运行迁移再校验——否则旧版二进制会在新库上应用「自己版本的 DROP TABLE」 |
| 失败 | 数据库不可达 / URL 为空 | 失败关闭（DB-01 既有语义不变） |
| 恢复 | 跨进程重启 | 重跑一次，全部命中已记录；行数不变 |
| 恢复 | 从失败点重试 | 一致性由 sqlx 的事务边界保证；`Dirty` 需人工处置（记录为已知运维动作） |

## 4. 明确不做

- **不引入 `sqlx migrate` CLI 作为部署路径**：迁移仍由应用启动时自动执行（`ready_pool`）。CLI 现在*可以*对接（这是本迭代的收益之一），但改变部署形态不在本轮范围。
- **不引入 `sqlx::query!` 编译期校验**：需要 live DB 或 `sqlx-data.json` 离线缓存，属独立决策；本轮只取 `migrate` feature。
- **不生成 down 迁移**：现有文件全部为 `Simple` 类型；回滚仍靠备份（`data/backup/`）。
- **不合并/重写历史迁移**：0001–0006 除了尾部记账语句与 0001 的旧表 CREATE，其余 SQL 逐字保留。
- **不改各域业务语义、不改 `external_order_calls=0` 约束、不触碰 A-05 影子窗口**。
- **不为 `_sqlx_migrations` 增加业务侧读取**：应用代码不直接查询该表，只经 sqlx。

## 5. 验收方法

### 5.1 门禁命令

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
npm run build
```

### 5.2 验收用例

| 编号 | 场景 | 必须观察到 | 对应 |
|---|---|---|---|
| S-MIG-1 | 全新库 | `_sqlx_migrations` = 1..7 且 `success` 全真、`execution_time > 0`；22 表 / 47 索引 / 11 触发器；旧表 ABSENT | R01/R03/R09 |
| S-MIG-2 | 存量库采纳 | 篡改场景下 `installed_on` 精确等于 `applied_at_ms` 换算值；采纳行 `execution_time = -1` 标记 | R06 |
| S-MIG-3 | v4 升级 | 1–4 采纳（`-1`）、5–7 应用；旧表被 0007 删除 | R08/R09 |
| S-MIG-4 | 校验和 | 篡改后硬失败，文案含 `previously applied but has been modified` | R04 |
| S-MIG-5 | 版本超前 | 旧表含 99 时 `bail!`，且**未**执行任何迁移 | R07 |
| S-MIG-6 | 幂等 | 重复调用不改变行数与内容；冷热耗时差异可测 | R10 |
| S-MIG-7 | 门禁 | §5.1 五条命令全部通过 | 全部 |

### 5.3 发布签收

| 项目 | 状态 | 备注 |
|---|---|---|
| 需求文档 | 待所有者确认 | 本文件（事后补正） |
| 设计文档 | 待所有者评审 | `docs/MIG-01-sqlx迁移统一-设计.md` |
| 任务文档 | 待所有者评审 | `docs/MIG-01-sqlx迁移统一-任务.md` |
| 实现 | 已完成 | 5 文件改动 + 1 新增迁移 |
| 验证 | 已完成 | 见设计文档 §6 证据表（含 6 条 DB 路径实测） |
| 发布 | 待签收 | 手册 §6/§10/§11 更新；已发布需求文本同步 |
