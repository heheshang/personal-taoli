# PERF-01 读路径性能与重复代码收敛：任务文档

文档状态：待所有者评审（2026-09-10）。实现顺序与验收见需求/设计文档；本文件为文件级执行清单与验证证据留存。

**流程偏差**：本任务文档系事后补正——需求确认、设计评审与实现在同一会话内完成，任务清单按已交付改动逐文件回溯整理。所有权确认见需求文档 §5。

## 1. 文件级改动清单

实现顺序按 DB 层 → 查询层 → 热路径 → 前端 → 配置，与设计文档 §1 模块边界一致。

### 1.1 DB 层：`crates/core/src/db.rs`

- `SCHEMA_VERSION` 5 → **6**；新增 `const READ_PATH_INDEXES_MIGRATION: &str = include_str!("../migrations/0006_read_path_indexes.sql");`
- 新增 `static READY: LazyLock<Mutex<HashSet<String>>>`。
- 新增 `pub(crate) async fn ready_pool(database_url: &str) -> Result<PgPool>`：命中 `READY` 直接返回池；否则 `migrate_pool` + 插入 `READY`。
- `migrate()` 改为 `ready_pool(database_url).await.map(|_| ())`（**签名不变**，`SimulationEngine::spawn` 的降级路径不受影响）。
- `migrate_pool` 的迁移列表末尾追加 `("read-path", READ_PATH_INDEXES_MIGRATION)`。

### 1.2 新增迁移：`crates/core/migrations/0006_read_path_indexes.sql`

五个 `CREATE INDEX IF NOT EXISTS` + 末尾 `INSERT INTO schema_migrations(version, applied_at_ms) VALUES (6, ...) ON CONFLICT DO NOTHING;`：

| 索引 | 服务的查询 |
|---|---|
| `audit_events_aggregate_id_pattern (aggregate_id text_pattern_ops)` | `WHERE aggregate_id LIKE 'f02-%'` |
| `audit_events_correlation_id_pattern (correlation_id text_pattern_ops)` | `WHERE aggregate_id LIKE $1 OR correlation_id LIKE $1` |
| `audit_events_occurred_at (occurred_at_ms DESC)` | `ORDER BY occurred_at_ms DESC LIMIT 50` |
| `trade_facts_intent_id (intent_id, occurred_at_ms)` | `WHERE intent_id IN ($1,$2) ORDER BY occurred_at_ms` |
| `balance_snapshots_source_observed_at (source, observed_at_ms DESC)` | `WHERE source='SIMULATION' ORDER BY observed_at_ms DESC LIMIT 1200` |

`text_pattern_ops` 的理由写入文件头注释：实库 collation 为 `en_US.utf8`，默认 btree opclass 无法服务非 C locale 下的 `LIKE` 前缀扫描。

### 1.3 查询层：`crates/core/src/simulation_query.rs`

- 导入：`use crate::db::{ready_pool, to_u64};`（移除 `migrate`/`pool`/`verify_schema`）；`use sqlx::postgres::{PgRow, Postgres};`。
- `open_readonly` 去掉 `verify_schema(&mut *txn)`（就绪性由 `ready_pool` 保证），文档注释同步。
- 三个入口统一为 `let pool = ready_pool(database_url).await?; let mut txn = open_readonly(&pool).await?;`——**顺带修掉 `get_simulation_runs` 原先不迁移的不一致**。
- 新增 `const CUMULATIVE_LIMIT: i64 = 1200;`，累计曲线改窗口函数 + `LIMIT`：
  ```sql
  SELECT executed_at_ms, scanned, simulated FROM (
      SELECT executed_at_ms,
             SUM(scanned_net_profit) OVER (ORDER BY executed_at_ms) AS scanned,
             SUM(simulated_net_profit) OVER (ORDER BY executed_at_ms) AS simulated
      FROM simulation_runs ORDER BY executed_at_ms DESC LIMIT $1
  ) AS recent ORDER BY executed_at_ms
  ```
- 新增 `const RIDGE_LIMIT: i64 = 4000;`，`load_ridge` 改同结构的子查询 + `LIMIT`。
- `load_flow_aggregate`：6 条查询 → 2 条（`execution_events` 用三个 `count(DISTINCT ...) FILTER`，`simulation_runs`/`audit_events` 用标量子查询）；`filled_runs` 改按 `bought_quantity > 0 AND sold_quantity > 0` 聚合。
- 新增 `const RUN_PROJECTION: &str`（22 列）与 `fn run_row_at(row: &PgRow, base: usize) -> Result<SimulationRunRow>`；`read_run_rows` 改用二者。
- `get_simulation_run_detail` 三次同表查询合并为一次（投影列 + `report` + `plan_id`/`buy_intent_id`/`sell_intent_id`/`account_id`），用 `fetch_optional` + `bail!` 保持「缺行失败关闭」与原文案。
- 测试：既有纯逻辑用例全部保留（前缀解析、桶合并、DTO 往返）。

### 1.4 热路径：解析与本地簿

- `crates/core/src/market.rs`：`parse_levels`/`parse_updates` 签名改 `impl IntoIterator<Item = [String; 2]>`（按值消费）；`LevelUpdate` 补文档注释，成为全仓唯一定义。
- `crates/core/src/local_book.rs`：删除自有 `LevelUpdate`，改从 `market` 导入；`apply` 的 `self.snapshot(1)?` → 新增的 `validate_live()`（只查空边与交叉盘，零分配）；新增两条用例 `apply_rejects_a_crossed_book`、`apply_rejects_emptying_a_side`。
- `crates/core/src/venues/binance.rs` / `bybit.rs`：删除各自 `parse_levels`，改 `market::parse_levels`。
- `crates/core/src/venues/binance_stream.rs` / `bybit_stream.rs`：删除各自 `parse_levels`/`parse_updates`，改 `market::{parse_levels,parse_updates}`；`LevelUpdate` 改从 `market` 导入；`rust_decimal::Decimal` 顶层导入按需清理、测试模块内按需补回。

### 1.5 生命周期与配置

- `crates/core/src/simulation.rs`：新增 `impl Drop for SimulationEngine`（置 `stopped` + `worker.abort()`）；`shutdown` 的文档注释说明「`stopped`+`notify` 唤醒空转 worker，`abort` 才是真终止，`Drop` 覆盖 `?` 早退路径」。两个 helper（`available_balance`、`business_count`）改 `ready_pool`。
- `crates/core/src/paper.rs`：`set_paper_balance` 改 `ready_pool`（去掉 `pool()` + `verify_schema()` 两步）。
- `crates/core/src/config.rs`：`use std::{borrow::Cow, ...}`；`migrate_legacy_toml`/`migrate_legacy_json` 返回 `Cow<'_, str>`，前置 `raw.contains("pairs") || !raw.contains("symbol")` 分流，只有旧格式才做完整 `Value` 解析 + 重序列化。

### 1.6 前端

- `src/components/SimulationTrades.vue`：新增 `symbolOptions`，`onMounted` 内 `loadSymbolOptions()` 从 `getObserverConfig().pairs` 派生；模板 `ElOption` 改 `v-for`（删除两个硬编码 `BTC/USDT` / `ETH/USDT`）。
- `src/components/SimulationProfitChart.vue`：删除 `scannedSum`/`simulatedSum` 累加，直接读后端累计字段。
- `src/components/SimulationRidge.vue`：新增 `BucketStats`（每桶解析+排序一次）；新增 `BINS`/`KERNEL_RADIUS`/`KERNEL` 常量与 `density()`（直方图 + 核卷积，成本与样本数无关）；`median()` 抽取；新增 `stats`/`rows` computed；`overallMedian` 改用 `stats`；单遍求极值替代 `Math.min(...values)`/`Math.max(...values)`。删除改版过程中残留的旧 `rows` computed。

### 1.7 文档同步

- `docs/DB-01-数据库连接层sqlx池化重构-设计.md`：`SCHEMA_VERSION` 行、§5 迁移策略（补 0006 与 `ready_pool` 按进程记忆）、§3.3 的 `expected 5` 行（标注序号现为 6）、§9 将已过期的「待迁移 8 模块」表替换为「已迁移」事实与新增加补说明。
- `docs/个人加密货币套利系统-AIDLC迭代执行手册.md`：§6 追踪矩阵追加 PERF-01、§10 发布记录追加 PERF-01 行、§11 下一轮唯一入口追加说明。
- 本迭代三份文档（需求/设计/任务）。

## 2. 验证证据

### 2.1 五门禁

```bash
cargo fmt --all -- --check                      # PASS
cargo test --workspace                          # 192 core + 8 tauri-lib passed; 0 failed
cargo clippy --workspace --all-targets -- -D warnings   # 零警告
cargo build --workspace --release               # 见 §2.4
npm run build                                   # vue-tsc --noEmit 零错误 + vite 构建成功
```

`cargo test --workspace` 原始输出（节选）：

```
Running unittests src/lib.rs (personal_taoli_core-...)
test result: ok. 192 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
Running unittests src/lib.rs (personal_taoli_tauri_lib-...)
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

测试基线为 190 core + 8 tauri-lib；本迭代净增 2 条（`local_book` 的交叉盘与空边拒绝），故 192。**本迭代未用测试替代实库验证**：新增测试只覆盖 §1.4 重写的校验逻辑（那是最可能被写反的地方，例如 `bids.last_key_value()` 与 `asks.first_key_value()` 的方向），其余改动以实库实测为证据。

### 2.2 实库验证（`taoli-postgres`，127.0.0.1:55432）

一次性驱动 `crates/core/examples/db_verify_probe.rs`（**验证后已删除**）输出：

```
cold_overview_ms=191     warm_overview_ms=42
total_runs=564
cumulative_points=564    monotonic_prefix_sums=true
cumulative_last={ executed_at_ms: 1789025075543, scanned_net_profit: 28.2433000, simulated_net_profit: -785.457318630 }
ridge_buckets=2          ridge_points=488
flow={ evaluated_runs: 488, reserved_runs: 555, filled_runs: 403,
       compensated_runs: 96, completed_runs: 373, escalated_runs: 85 }
balance_history=1200
stored_symbol=BTCUSDT
filtered_total=564 rows=20          slash_form_total=0
scenario_filter_total=244 rows=5
detail run=f02-1789025075543-19836 intents=2 trades=2 events=1 audit=1 balances=6 report_keys=38
missing_run_is_err=true
filter_options=["BTCUSDT","ETHUSDT","BNBUSDT","SOLUSDT","XRPUSDT","DOGEUSDT","ADAUSDT","AVAXUSDT"]
filter_option_BTCUSDT_total=564 rows=20
```

`psql` 直查：

```
schema_migrations → 1,2,3,4,5,6            （二次迁移后仍 6 行，未增长）
pg_indexes 新增   → audit_events_aggregate_id_pattern, audit_events_correlation_id_pattern,
                    audit_events_occurred_at, balance_snapshots_source_observed_at,
                    trade_facts_intent_id
```

索引可用性（均于 `SET enable_seqscan=off` 下）：

```
修复前: Seq Scan on audit_events (rows=555)      / Seq Scan on trade_facts (rows=928, Removed 928)
修复后: Index Scan using audit_events_occurred_at / Index Scan using trade_facts_intent_id
```

### 2.3 未执行的验证（明确记录）

- **GUI 渲染未做像素级验证**：Tauri 桌面应用的 `invoke` 需真实宿主，浏览器 relay 无法驱动 WKWebView；未获所有者授权前不自行弹出 GUI 窗口。前端改动由 `vue-tsc --noEmit` 模板类型检查 + 生产构建 + 各命令契约的实库实测覆盖，**渲染结果本身未经目视确认**。
- **R11（`Drop`）无运行时断言**：仅有编译期保证，见设计 §7 R-5。
- **R09 无基准数值**：山脊重算的成本结构由代码判定（`BINS` × 核半径，与样本数无关），未做前后计时对比。

### 2.4 发布构建

```
cargo clippy --workspace --all-targets -- -D warnings
  → Finished `dev` profile ... in 7.89s          （无任何 warning/error 输出；-D warnings 下即为通过）

cargo build --workspace --release
  → Finished `release` profile [optimized] target(s) in 2m 01s

npm run build
  → vue-tsc --noEmit 零错误；vite 构建成功
    ui/assets/index-B0HHeIPu.css   392.60 kB │ gzip:  54.22 kB
    ui/assets/index-Csay9P_7.js  1,097.87 kB │ gzip: 357.85 kB
```

`npm run build` 输出含 vite 的「chunk 大于 500 kB」提示。该提示在本迭代前既已存在（依赖 element-plus 全量引入），**不是本迭代引入**，也未在本迭代处理——代码分割属独立的展示层决策。

## 3. 清理项

| 项 | 状态 |
|---|---|
| `crates/core/examples/db_verify_probe.rs` | 已删除（`examples/` 目录一并移除） |
| `crates/core/examples/` 空目录 | 已删除 |
| DB-01 设计文档中已过期的「待迁移 8 模块」表 | 已替换为事实 |
| 残留的旧 `rows` computed（`SimulationRidge.vue`） | 已删除 |
| `venues/*` 中因删除解析函数而失效的导入 | 已清理（clippy `-D warnings` 为 0 佐证） |

## 4. 剩余风险与后续入口

剩余风险见设计文档 §7（R-1 曲线窗口起点语义、R-2 `filled_runs` 数值变化需在发布记录说明、R-3 `READY` 不感知外部改库、R-5 R11 无运行时断言、R-6 大表建索引、R-7 collation 耦合）。

后续独立立项项（本迭代明确不做，见需求 §4）：

1. **16 个无生产调用点模块的接线或删除**（6304 行 / 132 测试）——接线前必须先为 `performance.rs`、`system_health.rs`、`notification.rs`、`alert_manager.rs` 的无界容器补淘汰策略。
2. 展示层取舍：`DetailPanel.vue` 拆分、概览页静态装饰组件去留（`RidgePanel` 与 `SimulationRidge` 为同一职责的双实现）。
3. 冒烟报告的 TS 判别联合（后端 `PaperSmokeResult` 已是 tag/content 联合，前端仍按字段探测）。
