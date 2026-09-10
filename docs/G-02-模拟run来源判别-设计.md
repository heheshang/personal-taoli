# G-02 模拟 run 来源判别：设计文档

文档状态：待所有者评审（2026-09-10）。对应需求：`docs/G-02-模拟run来源判别-需求.md`。
前置迭代：G-01（仪表盘）、F-02（仿真引擎）、MIG-01（sqlx 迁移）。

**流程偏差**：本设计系事后补正，内容与已交付代码逐条对应。

---

## 1. 模块边界

```
crates/core/migrations/0008_simulation_run_source.sql   ← 新增：source 列 + 回填 + 索引
crates/core/src/simulation.rs                            ← SimulationRunSource；run/报告/持久化透传；烟测横幅
crates/core/src/simulation_query.rs                      ← 投影列 + 概览来源计数
src/types.ts                                             ← SimulationOverview / SimulationRunRow 增字段
src/components/SimulationStatCards.vue                   ← 来源提示（区分全探针/全实时/混合）
src/components/SimulationTrades.vue                      ← 列表行与详情页来源徽章
src/components/SimulationPage.vue                        ← 烟测按钮自我说明
```

**边界不变式**：来源由**调用方**决定并随 run 报告一并持久化；引擎不猜测、不从场所名反推。查询层只读该列，不重新推导。

## 2. 唯一数据契约

```rust
// simulation.rs
pub enum SimulationRunSource { Smoke, Live }     // as_str() -> "SMOKE" | "LIVE"
pub struct SimulationRunReport { …, pub source: SimulationRunSource, … }

pub async fn run_simulation_run(
    database_url: &str,
    event: &DecisionEvent,
    opportunity: &Opportunity,
    config: &SimulationConfig,
    skipped_frames: u64,
    source: SimulationRunSource,        // 新增形参：显式来源
) -> Result<SimulationRunReport>;

// simulation_query.rs
pub struct SimulationRunRow { …, pub source: String }
pub struct SimulationOverview { …, pub smoke_runs: u64, pub live_runs: u64 }
```

**数据库**：`simulation_runs.source TEXT NOT NULL DEFAULT 'LIVE' CHECK (source IN ('SMOKE','LIVE'))`；新增索引 `simulation_runs_source_clock_idx (source, executed_at_ms DESC)`。

**不变契约**：`SimulationScenario` 值域、`FlowAggregate` 字段、三个查询入口签名、`run_simulation_smoke` 的返回结构、`external_order_calls` 约束。

## 3. 状态、错误与降级语义

| 场景 | 旧行为 | 新行为 |
|---|---|---|
| 探针 run 落库 | 与实时 run 无法区分 | `source='SMOKE'` |
| 实时 run 落库 | 同上 | `source='LIVE'` |
| 存量行 | — | 迁移时全部回填 `SMOKE`（依据见 §4 D2） |
| 未来插入未绑定 `source` 的行 | — | 列默认 `LIVE`（引擎现已显式绑定，默认值只是兜底） |
| 概览查询 | 无来源构成 | `smoke_runs` / `live_runs` 两计数 |
| 界面 | 探针与实时相同呈现 | 统计卡来源提示 + 列表/详情徽章 |

## 4. 关键决策

| 编号 | 决策 | 依据 |
|---|---|---|
| D1 | 来源作为 `run_simulation_run` 的**显式形参**，而非从 `event` 或场所名推导 | 场所名（`s01-buy`）是实现细节，一旦夹具改名判定即失效；显式传参让 7 个烟测调用点与 1 个实时调用点在代码里各自声明来源，编译器强制不漏 |
| D2 | 回填判定：**迁移时刻已存在的行全部视为探针** | 实测依据：迁移前 613 行 `buy_venue` 全部落在 `s01-buy`…`s07-sell`，且 8000 条 decision 中 accepted 为 0——实时 run 从未存在过。故「既存即探针」不是推测而是已证实事实 |
| D3 | 回填用 **`ADD COLUMN … DEFAULT 'SMOKE'` 后 `SET DEFAULT 'LIVE'`**，不用 `UPDATE` | `0005` 装了 `simulation_runs_immutable`（`BEFORE UPDATE OR DELETE`），`UPDATE` 会被直接拒绝（见 §6 失败记录）。绕开方式有两种：临时 `DROP TRIGGER` 再重建，或改默认值。**选了后者**——前者为一次性便利牺牲一条常驻安全不变量，且若中途失败会留下触发器缺失的窗口（虽有事务保护，但不值得冒险） |
| D4 | 列不可为 NULL，且 CHECK 限定两值 | 与既有枚举列（`scenario`/`direction`）风格一致；NULL 会让「未标注」与「探针」不可区分，正是本迭代要消除的模糊 |
| D5 | 概览新增两个计数字段，而非把 `total_runs` 拆成两个接口 | 界面需要同时表达「总量」与「构成」；拆接口会让统计卡发两次请求 |
| D6 | 界面用**文案 + 警示配色**而非仅一个徽章 | 所有者被误导的场景是「没注意到」，因此必须在统计卡顶部以段落形式陈述结论（含实时为 0 的原因），配色区分 warn/mixed/live |
| D7 | 模拟页合约过滤**不下沉到概览聚合** | 筛选控件位于机会历史面板内，其作用域即该面板；若让统计卡随其变化，则「来源提示」的计数口径会随筛选漂移，反而更难读。所有者明确选择页内独立过滤 |
| D8 | 烟测日志加横幅，但不改其 symbol | 改 symbol 等于把合成簿冒充市场数据；正确做法是声明合成属性 |

## 5. 删除或替换的旧路径

| 项 | 位置 | 替换为 |
|---|---|---|
| 「靠场所名猜来源」的隐式约定 | 全库（无显式代码，靠 `s01-buy` 字面量辨认） | `source` 列 + `SimulationRunSource` 枚举 |
| 统计卡文案「F-02 合成簿仿真」的含混表述 | `SimulationPage.vue` 页头 | 「只读仪表盘 · 无真实/测试网订单路径」+ 按钮旁独立说明 |
| 烟测按钮「运行一次烟测」 | `SimulationPage.vue` | 「运行合成簿烟测」+ 合成属性说明 |
| 无来源计数的概览聚合 | `simulation_query.rs` 的 `agg` 查询 | 追加两个 `COUNT(*) FILTER (WHERE source = …)` |

**无并行真相源**：来源只有列一个来源，界面与查询层均读同一列，不由场所名二次推导。

## 6. 实库证据基线

**迁移首次失败（完整记录，含根因）**

```
第一次 0008（用 UPDATE 回填）：
  Error: failed to apply database migrations
  Caused by:
      0: while executing migration 8: error returned from database: audit_events are immutable
      1: error returned from database: audit_events are immutable
  → 应用侧表现为 SIMULATION_ERROR: failed to apply database migrations

失败后现场核查：
  _sqlx_migrations where version=8        → 0 行          （记账与迁移同事务，一并回滚）
  information_schema…column_name='source' → 0 行          （ADD COLUMN 一并回滚）
  version 1..7                            → 全部 success=t（无 Dirty）
```

**这是 MIG-01 设计 §7 R-3（`Dirty` 人工处置）的首次实测**：结论比预期干净——sqlx 的「每文件 + 其记账同事务」使失败迁移**零残留**，不是 `Dirty`（`success=false`）状态，修正文件后可直接重跑。风险等级相应下调，但 R-3 仍保留：真正的半应用只可能来自 `-- no-transaction` 文件（本项目无）。

**修正后（去掉 UPDATE）**

```
_sqlx_migrations version=8 → success=t
pg_trigger simulation_runs_immutable → 仍存在
UPDATE simulation_runs SET source='LIVE' … → ERROR: audit_events are immutable   ← 保护未被削弱
column_default='LIVE'::text, is_nullable=NO
source 分布：SMOKE 620  （迁移时 613 + 校验期间实时观察循环未产生 run）
```

**打标端到端（实跑烟测）**

```
before: total=613 smoke=613 live=0
run_simulation_smoke → external_order_calls=0
after : total=620 smoke=620 live=0
newest source=SMOKE (must be SMOKE)
```

**页内过滤值域（8 个配置币对）**

```
total=627 smoke=627 live=0
  filter BTCUSDT  -> total=627        ← 探针 symbol 夹具常量
  filter ETHUSDT  -> total=0
  filter BNBUSDT  -> total=0   … 其余同
no filter  -> total=627
```

（0 是**正确**结果：不存在任何 ETHUSDT run——实时 run 为 0，探针 symbol 恒为 BTCUSDT。）

**界面渲染断言（mock IPC，同 G-01/UI-01 做法）**

页面文本实测含：

```
运行合成簿烟测
烟测用合成簿（venues s01–s09，symbol 为夹具常量），不是市场数据
数据来源
全部 620 条为合成簿探针（venues s01–s09，非市场数据）；实时 run 0 条 —— 准入未通过（凭证/资格未配置，见控制台账户状态）
f02-1789033590398-73256 / 合成簿 / 2026-09-10 17:46
```

视觉确认：来源提示为粉红底 + 深红字，与周围白色面板显著区分（视觉模型读图确认）。

## 7. 风险与缓解

| 风险 | 影响 | 缓解 / 状态 |
|---|---|---|
| R-1 「既存即探针」回填若判断有误 | 真实 run 被误标 SMOKE | 已用两条独立证据证实（场所名全为夹具 + accepted 恒 0）。若将来出现例外，该列可被新迁移修正 |
| R-2 默认值 `LIVE` 使「忘记绑定」静默变成实时 | 误标来源 | 引擎已显式绑定；默认值只是兜底。未加编译期强制（`run_simulation_run` 的形参已强制调用方表态，故风险有限） |
| R-3 来源提示文案与实际计数脱节 | 界面说谎 | 计数直接来自同一聚合查询；文案三种分支由计数驱动，无独立硬编码 |
| R-4 概览聚合不随合约筛选变化 | 用户以为筛选生效于全页 | 有意为之（D7）；筛选控件位于列表面板内，作用域即该面板 |
| R-5 回填绕过不可变保护 | 保护被削弱 | **未削弱**：触发器从未被 drop，实测仍拒绝 UPDATE（见 §6） |
| R-6 未实测 `Dirty` 人工处置 | MIG-01 R-3 仍待补 | 本次失败走的是回滚路径而非 `Dirty`；该场景仍未实测，处置动作仍未入 runbook |

## 8. 需求到模块的映射

| 需求 | 落点 | 验证 |
|---|---|---|
| R01 来源列 | `0008_simulation_run_source.sql` | S-G02-1 |
| R02 显式打标 | `simulation.rs`（枚举 + 形参 + 8 个调用点） | S-G02-4 |
| R03 回填 | 同上（DEFAULT 'SMOKE'） | S-G02-2 |
| R04 保护未削弱 | 不 drop 触发器 | S-G02-3 |
| R05 概览计数 | `simulation_query.rs` `agg` | S-G02-4 |
| R06 界面可辨 | `SimulationStatCards` / `SimulationTrades` | S-G02-5 |
| R07 自我说明 | `SimulationPage` / `run_simulation_smoke` 横幅 | S-G02-5 |
| R08 解释实时为 0 | `SimulationStatCards` 的 `provenance` | S-G02-5 |
| R09 页内过滤 | `SimulationTrades`（PERF-01 已修值域） | S-G02-6 |
| R10 语义不变 | 全部 | S-G02-7 + 既有测试 |
