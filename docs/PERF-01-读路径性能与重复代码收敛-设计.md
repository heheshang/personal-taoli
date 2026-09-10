# PERF-01 读路径性能与重复代码收敛：设计文档

文档状态：待所有者评审（2026-09-10）。对应需求：`docs/PERF-01-读路径性能与重复代码收敛-需求.md`。
前置迭代：DB-01（sqlx 池化，`verified`）。

**流程偏差**：本设计系事后补正，其内容与已交付代码逐条对应；详见需求文档 §流程偏差声明。设计决策 D1–D8 均在实现过程中由实库实测确定，证据见 §8。

---

## 1. 模块边界

```
crates/core/src/db.rs                    ← 唯一连接层：池缓存 + 就绪记忆（新增 READY / ready_pool）
crates/core/src/simulation_query.rs      ← G-01 只读查询层：有界读取、前缀和、聚合合并
crates/core/src/market.rs                ← 行情解析与 LevelUpdate 的唯一实现（opts）
crates/core/src/local_book.rs            ← 本地簿：validate_live（新增）
crates/core/src/venues/*.rs              ← 四个 venue 文件删除各自的解析副本
crates/core/src/config.rs                ← 配置加载：旧格式迁移改 Cow 分流
crates/core/src/simulation.rs            ← SimulationEngine 增 Drop
crates/core/src/paper.rs                 ← set_paper_balance 改走 ready_pool
crates/core/migrations/0006_read_path_indexes.sql  ← 新增：5 个纯增量索引
src/components/SimulationTrades.vue      ← 过滤选项来自配置
src/components/SimulationProfitChart.vue ← 消费后端前缀和
src/components/SimulationRidge.vue       ← 直方图+核卷积、单遍极值
```

**边界不变式**：`db.rs` 之外任何模块不得自行构造池或连接；所有连接仍一律来自 `db::pool(url)`。
本迭代新增的约束是：**凡需要「schema 已就绪」的调用方，必须走 `db::ready_pool(url)`，不得自行调用 `migrate()` 后再 `pool()`**——后者正是本迭代要消除的重复劳动。

## 2. 唯一数据契约

### 2.1 新增/变更的 crate 内契约

```rust
// db.rs —— 就绪库单点
pub(crate) async fn ready_pool(database_url: &str) -> Result<PgPool>;
pub(crate) async fn migrate(database_url: &str) -> Result<()>;   // 签名不变，改为委托 ready_pool
static READY: LazyLock<Mutex<HashSet<String>>>;                  // 按 URL 记忆「已迁移 + 已校验」
pub(crate) const SCHEMA_VERSION: i64 = 6;                        // 5 → 6

// simulation_query.rs —— 有界读取
const CUMULATIVE_LIMIT: i64 = 1200;   // 累计曲线窗口
const RIDGE_LIMIT: i64 = 4000;        // 山脊样本窗口
const RUN_PROJECTION: &str;           // simulation_runs 22 列投影清单（列表与详情共用）
fn run_row_at(row: &PgRow, base: usize) -> Result<SimulationRunRow>;
```

**不变契约（逐字保留）**：`pool()`、`migrate_pool()`、`verify_schema<'e, E>()`、`DomainConnection::{acquire,disconnect}`、`SCHEMA_VERSION` 的**用途**（`verify_schema` 仍断言 `MAX(version) == SCHEMA_VERSION`）、五个只读入口的签名、`SimulationOverview`/`SimulationRunDetail`/`FlowAggregate` 的字段集（仅语义修正见 R05）、`Level`/`OrderBookSnapshot` 的 serde 形状。

### 2.2 语义变更（唯一一处对外的字段语义修正）

`FlowAggregate.filled_runs` 的**含义**由「`COMPLETED` 事件去重数」改为「真实双腿成交 run 数」。字段名与类型不变，故 DTO 与前端无需改动，但数值会变（实库实测 403 vs 373）。

依据：`execution.rs` 的决策分支中 `mismatch == Decimal::ZERO` 即 `(Completed, NoAction)`，而 `mismatch` 是两腿成交量的差——**两腿都为零也满足**。原实现里 `filled_runs` 与 `completed_runs` 是两条逐字相同的 SQL，等于把「结算完成」当成「双腿成交」展示在资金流节点上。需求 G01-R04 要求该节点由真实事件聚合，故按 `simulation_runs.bought_quantity > 0 AND sold_quantity > 0` 聚合。

## 3. 状态、错误与降级语义

| 场景 | 旧行为 | 新行为 | 依据 |
|---|---|---|---|
| 首次访问某 URL | 迁移 + 校验 | 同（一次） | D1 |
| 之后每次访问同一 URL | **再次迁移 + 校验**（六份 DDL + advisory 锁） | 命中 `READY`，零 SQL | D1 |
| 并发首次访问 | 各自迁移（由 advisory 锁串行） | 同；两条路径都可能执行 `migrate_pool`，锁内串行且全部 DDL 幂等，`READY` 只是跳过后续 | D1 |
| 迁移失败 | `Err` 上抛 | 同（不改降级语义；`SimulationEngine::spawn` 仍为告警 + `None`） | — |
| schema 版本不符 | 原文案 `bail!` | 同（序号取自 `SCHEMA_VERSION`，现为 6） | R02 |
| `READY` 与实际库状态不符（外部 drop 表） | — | **不自动恢复**：本进程认为已就绪。记录为已知取舍，见 §7 风险 R-3 | D1 |
| 曲线/山脊超限 | 全量返回 | 截断到上限；曲线取**最近** N 个 run（非最老），仍按时间升序输出 | D3 |
| 详情 `run_id` 不存在 | `bail!` | 同（`fetch_optional` 后 `bail!`，文案不变） | D5 |
| 本地簿增量后交叉/空边 | `snapshot(1)?` 构造后丢弃 | `validate_live()` 判定，错误文案由「snapshot is crossed」调整为「local book is crossed」（见 §7 风险 R-4） | D6 |

## 4. 关键决策

| 编号 | 决策 | 依据 |
|---|---|---|
| D1 | `READY: HashSet<String>` 按 URL 记忆「已迁移 + 已校验」，`migrate` 改为委托 `ready_pool` | 与 `POOLS` 同构（`src-tauri` 每条命令重读 `TAOLI_DATABASE_URL`，单一全局布尔不够）。理由是**成本结构**：DDL 幂等但昂贵，而轮询是 10s 一次。实库实测 191ms → 42ms |
| D2 | 用 `HashSet` 而非 `OnceCell` 或 `AtomicBool` | 需要按 URL 维度记忆；`HashSet` 与既有 `POOLS` 的键空间一致，无新概念 |
| D3 | 曲线用窗口函数在服务端算前缀和，并**先取最近 N 再升序** | 前缀和必须覆盖全部历史（否则数值错），故 `SUM(...) OVER (ORDER BY ...)` 取全量前缀 + `LIMIT` 取最近段。前端因此不必累加，顺带消除 R08 的浮点漂移 |
| D4 | 索引迁移独立为 0006 并提升 `SCHEMA_VERSION` 至 6 | 迁移文件一经发布不可改写（0001–0005 已在不变量基线上）；新增索引必须走新版本号。0006 全部 `CREATE INDEX IF NOT EXISTS`，无表结构变更 |
| D5 | 详情合并为一次 `SELECT`，用 `RUN_PROJECTION` 常量 + `run_row_at(row, base)` 复用列清单 | 原实现三次查同一行、且列清单硬编码在 `read_run_rows` 里；详情若另写列清单必然与列表漂移。`base` 偏移让详情在同一行尾部附加 `report` 与三个 id |
| D6 | 本地簿 `apply` 的 `snapshot(1)?` 换成 `validate_live()` | `snapshot()` 只为触发校验而构造了 2 个 `String` + 2 个 `Vec<Level>`，结果立即丢弃。`validate()` 相对逐档校验只多两条不变量：空边、交叉盘 |
| D7 | `market::parse_levels`/`parse_updates` 改 `impl IntoIterator<Item = [String;2]>` 并删除 6 份私有副本 | 调用方都持有反序列化后的 `Vec<[String;2]>` 所有权，按值消费避免逐档复制（原 `&[[String;2]]` 签名会强制 `into_iter` 前先借用）。`LevelUpdate` 的两个相同定义收敛到 `market.rs`，`local_book` 不再自有定义 |
| D8 | `config.rs` 旧格式迁移改为先做子串分流、返回 `Cow` | 新格式（含 `pairs`）占绝大多数；原实现无条件 `from_str::<Value>` + `to_string` 后交由外层再解析一次，等于双倍解析。分流后新格式零额外分配 |

## 5. 删除或替换的旧路径（显式清单）

| 删除项 | 位置 | 替换为 |
|---|---|---|
| `get_simulation_overview` 内的 `migrate()` + `pool()` + `open_readonly` 内 `verify_schema` | `simulation_query.rs`（旧） | `ready_pool(url)`，`open_readonly` 只开事务 |
| `get_simulation_run_detail` 内的 `migrate()` / `get_simulation_runs` 内裸 `pool()` | `simulation_query.rs`（旧） | 同上（R06 顺带修掉「列表入口不迁移」的不一致） |
| `paper::set_paper_balance` 内 `pool()` + `verify_schema()` | `paper.rs:159-160`（旧） | `ready_pool(database_url)` |
| `simulation.rs` 两个 helper 内 `pool()` | `available_balance` / `business_count` | `ready_pool(database_url)` |
| 累计曲线全表 `SELECT ... ORDER BY executed_at_ms` | `simulation_query.rs:328`（旧） | 窗口函数 + `LIMIT $1` 的子查询 |
| 山脊全表 `SELECT ... WHERE scenario <> 'REJECTED'` | `simulation_query.rs:787`（旧） | 同结构 + `LIMIT $1` |
| `load_flow_aggregate` 6 条查询 | `simulation_query.rs:852-894`（旧） | 2 条（`FILTER` 聚合 + 标量子查询） |
| 详情 3 次同表往返 + 硬编码列清单 | `simulation_query.rs:493/504/511`（旧） | 1 次查询 + `RUN_PROJECTION`/`run_row_at` |
| `venues/{binance,bybit}.rs` 的 `parse_levels` | 各自文件（旧） | `market::parse_levels` |
| `venues/{binance_stream,bybit_stream}.rs` 的 `parse_levels` / `parse_updates` | 各自文件（旧） | `market::{parse_levels,parse_updates}` |
| `local_book.rs` 的 `pub(crate) struct LevelUpdate` | `local_book.rs:132`（旧） | `market::LevelUpdate` |
| `market.rs` 的 `&[[String;2]]` 版本解析签名 | `market.rs:99/111`（旧） | 按值消费的泛型签名 |
| `LocalOrderBook::apply` 的 `self.snapshot(1)?` | `local_book.rs:187`（旧） | `validate_live()` |
| `SimulationEngine` 无 `Drop` | `simulation.rs`（旧） | `impl Drop`：置 `stopped` + `worker.abort()` |
| 前端 `SimulationTrades.vue` 两个硬编码 `ElOption` | `:185-186`（旧） | `symbolOptions`（来自 `getObserverConfig().pairs`） |
| 前端 `SimulationProfitChart.vue` 的 `scannedSum +=` / `simulatedSum +=` | `:43-44`（旧） | 直接读后端累计字段 |
| 前端 `SimulationRidge.vue` 逐点 KDE 与 `Math.min(...values)` | `:68-69/80`（旧） | 直方图 + 核卷积、单遍极值 |

**无并行真相源**：`migrate()` 与 `ready_pool()` 不是两条路径——前者委托后者；`market::LevelUpdate` 是唯一 `LevelUpdate` 定义，`local_book` 只是导入。

## 6. 验收场景到模块的映射

| 需求 | 落点模块 | 验证方式 |
|---|---|---|
| R01 就绪库单点 | `db.rs::ready_pool`/`READY` | S-PERF-1 冷热对比；S-PERF-3 跨进程幂等 |
| R02 读路径索引 | `db.rs` + `0006_read_path_indexes.sql` | S-PERF-2 `EXPLAIN` 前后对比 |
| R03 曲线有界+前缀和 | `simulation_query.rs`（`CUMULATIVE_LIMIT`） | S-PERF-4 |
| R04 山脊有界 | `simulation_query.rs`（`RIDGE_LIMIT`） | S-PERF-5 |
| R05 流转聚合 | `simulation_query.rs::load_flow_aggregate` | S-PERF-6 |
| R06 详情单次往返 | `simulation_query.rs`（`RUN_PROJECTION`/`run_row_at`） | S-PERF-7 |
| R07 前端过滤值域 | `SimulationTrades.vue` | S-PERF-8 |
| R08 前端累计 | `SimulationProfitChart.vue` | S-PERF-4 + 代码审查无 `+=` |
| R09 前端山脊成本 | `SimulationRidge.vue` | 代码审查（成本只与 `BINS` 相关）+ `Math.min(...)` 消除 |
| R10 热路径 | `market.rs`/`local_book.rs`/`venues/*` | S-PERF-9（既有 + 新增用例） |
| R11 生命周期 | `simulation.rs::Drop` | 编译期；无运行时断言（见 §7 风险 R-5） |
| R12 配置加载 | `config.rs` | 既有配置加载用例 + 代码审查 |

## 7. 风险与缓解

| 风险 | 影响 | 缓解 / 状态 |
|---|---|---|
| R-1 曲线窗口取「最近 N」改变了曲线起点 | 末尾数值不变（前缀和是全量算的），但**曲线最左端不再是 0 起点**，视觉上起点抬高 | 有意为之：末点仍是真实累计值（实库 28.2433000 / −785.457318630），前端轴标签本就写「首个 run / 最新 run」，在数据被截断时应理解为「窗口首点」。若所有者要求显示绝对累计起点，改为返回 `{total_prefix, points}` 两段 |
| R-2 `filled_runs` 数值变化 | 前端「双腿成交」节点计数会变小（403 vs 373，实库） | 这是**修正**而非回归：需求 G01-R04 要求该节点反映真实成交。需在发布记录中显式说明，避免被误读为数据丢失 |
| R-3 `READY` 不感知库被外部修改 | 若外部 drop 表/降版本，本进程不会重新校验 | 记录为已知取舍：进程级记忆的代价。恢复手段是重启进程。未加 TTL——轮询路径上加 TTL 等于把问题搬回来 |
| R-4 本地簿交叉错误文案变更 | `snapshot is crossed` → `local book is crossed`；`validate()` 的 `insufficient ... depth` 在 `apply` 路径不再可能出现 | 无测试或调用方匹配该文案（已核对）；语义更强（在 apply 处即拒绝）。新增两条用例锁定 |
| R-5 R11 无运行时断言 | `Drop` 的正确性只有编译期保证 | 未写「abort 后任务消失」的断言（需 tokio 运行时计量，成本高于收益）。记录为验证缺口 |
| R-6 0006 在大表上建索引 | 首次升级会持锁建索引（本项目规模：`audit_events` 694 行） | 当前数据量下为毫秒级。数据量大时需改为 `CREATE INDEX CONCURRENTLY`（不能在迁移事务内执行，需另行设计） |
| R-7 `text_pattern_ops` 与 collation 耦合 | 索引仅在非 C collation 下为前缀查询所必需 | 实库 `datcollate = en_US.utf8`，已实测索引被选中；迁移注释记录该前提 |

## 8. 实库证据基线

全部在 `taoli-postgres`（55432，本机 Docker）实测，命令输出原始留存见任务文档 §2。

**R02 索引可用性（修复前 / 修复后，均带 `SET enable_seqscan=off`）**

```
修复前 audit_events 前缀查询：  Seq Scan on audit_events (rows=555, Filter: aggregate_id ~~ 'f02-%')
修复前 trade_facts intent 查询：Seq Scan on trade_facts (rows=928, Rows Removed by Filter: 928)
修复后 audit_events 前缀查询：  Index Scan using audit_events_occurred_at on audit_events
修复后 trade_facts intent 查询：Index Scan using trade_facts_intent_id (Index Cond: intent_id = ANY(...))
```

**R01 冷热对比（driver 输出）**

```
cold_overview_ms=191   warm_overview_ms=42
二次调用后 schema_migrations 行数=6（未增长）
```

**R03/R04/R05/R06 查询输出**

```
total_runs=564   cumulative_points=564   monotonic_prefix_sums=true
cumulative_last={ executed_at_ms: 1789025075543, scanned_net_profit: 28.2433000, simulated_net_profit: -785.457318630 }
ridge_buckets=2  ridge_points=488
flow={ evaluated_runs: 488, reserved_runs: 555, filled_runs: 403,
       compensated_runs: 96, completed_runs: 373, escalated_runs: 85 }
detail run=f02-1789025075543-19836 intents=2 trades=2 events=1 audit=1 balances=6 report_keys=38
missing_run_is_err=true
```

**R07 过滤值域**

```
stored_symbol=BTCUSDT          filtered_total=564   rows=20
slash_form_total=0             （证明为精确匹配，原前端字面量必然匹配 0 行）
scenario_filter_total=244      rows=5（symbol + scenario 双过滤）
filter_options=["BTCUSDT","ETHUSDT","BNBUSDT","SOLUSDT","XRPUSDT","DOGEUSDT","ADAUSDT","AVAXUSDT"]
filter_option_BTCUSDT_total=564（配置派生选项命中非空）
```

**R02 迁移落库（psql）**

```
schema_migrations: 1,2,3,4,5,6
pg_indexes 新增: audit_events_aggregate_id_pattern / audit_events_correlation_id_pattern /
                 audit_events_occurred_at / balance_snapshots_source_observed_at / trade_facts_intent_id
```

**一次性验证驱动**：`crates/core/examples/db_verify_probe.rs`，验证后已删除（§9 清理项）。
