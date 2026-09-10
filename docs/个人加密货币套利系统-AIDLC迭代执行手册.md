# 个人加密货币套利系统：AIDLC 迭代执行手册

## 1. 文档目的

本手册取代原《个人加密货币套利系统：SDLC 迭代执行手册》，将软件交付流程升级为 AIDLC（AI-augmented Development Life Cycle，AI 增强开发生命周期）。架构与需求文档定义系统最终必须具备什么能力；本手册定义每轮只交付什么、按什么顺序实施、用什么证据验收、AI 助手在每轮承担什么职责与什么绝对禁止，以及何时禁止进入下一阶段。

AIDLC 与 SDLC 的本质区别：执行主体从单一人工变为「所有者决策 + AI 助手执行」。所有者是唯一决策者，负责需求取舍、评审门禁、发布决定和资本与风险决策；AI 助手是执行者，承担编码、测试、文档起草和证据收集，无决策权。所有阶段门禁增加人工确认环节，AI 的任何声称必须可复现、可追溯。

本手册保留原 SDLC 手册的全部历史迭代记录（§6–§11）作为既定事实，不再改动已发布内容。

适用范围：个人维护、自有资金、首期 Binance 与 Bybit 预充值现货跨所套利。当前程序仅为公共接口只读观察器。

## 2. 不可突破的安全边界

1. 在安全交易核心、恢复、账务、对账、权限和 P0 故障验收全部完成前，不加入真实下单凭证或真实下单入口。
2. 任何迭代不得把“API 可调用”“测试网成功”或“发现正价差”描述为可以实盘。
3. 交易所字段缺失、语义不确定、品种映射冲突、元数据过期或行情失效时失败关闭。
4. 金额、价格、数量和费率使用 `Decimal`；交易所字符串数值不得先转浮点数。
5. 每轮只建立一个新闭环。不得同时引入数据库、消息队列、多交易所和自动交易以制造不可定位的故障面。
6. 任何产生外部副作用的能力必须先有幂等键、耐久化意图、UNKNOWN 状态、恢复查询和人工接管路径。
7. AI 助手生成的结论、代码或证据未经可复现验证和所有者确认，不视为完成；「AI 说可以」永远不能替代命令输出、测试日志或真实烟测记录。
8. AI 助手不得自行扩大本轮范围、引入未要求的能力或顺手修改无关代码；不得把推测表述为事实，无法从来源证实的语义按失败关闭处理并明确标注。

## 3. 生命周期和状态规则

每轮严格经过六个阶段：需求 → 分析 → 设计 → 实现 → 验证 → 发布。只有上一阶段退出条件满足且经所有者确认，下一阶段才开始。AI 助手可以起草文档、预检和提交候选结果，但不能替代退出判定：每个阶段的「退出条件通过」声明必须附带可复现证据，并经所有者确认后才推进。

工作项状态只有：

- `proposed`：尚未确认价值或边界。
- `ready`：依赖、验收标准和风险已明确。
- `in_progress`：当前唯一主要工作项。
- `blocked`：缺少外部账户、接口事实或运行环境；必须记录阻塞证据。
- `verified`：实现和指定验证均通过，尚未发布。
- `released`：文档、配置和运行说明同步完成。
- `rejected`：不再实施，并记录原因。

禁止用“基本完成”“大致可用”代替状态。

## 4. 每轮阶段门禁

### 4.1 需求

输入：架构文档中的一个或一组强关联需求。

AI 职责：从架构文档提取强关联需求，起草需求文档和验收方法草案。

必须产出：

- 唯一需求编号；
- 需求文档：
- 用户可观察结果；
- 明确不做的范围；
- 正常、边界、失败和恢复场景；
- 可自动或可重复执行的验收方法。

退出条件：每项结果能用输入与输出描述；不得只写内部类、表或接口。

人工门禁：所有者逐项确认需求范围（含「明确不做」清单）；AI 不得自行裁撤需求或提前引入实现方案。

### 4.2 分析

AI 职责：核对交易所官方协议与真实响应样本、当前代码调用链、数据所有权和故障传播，建立带来源的协议事实；限频、精度、时钟、幂等和恢复约束必须对应到官方文档或真实样本。

必须核对：

- 交易所官方协议和真实响应样本；
- 当前代码调用链、数据所有权和故障传播；
- 限频、精度、时钟、幂等和恢复约束；
- 是否需要账户权限或真实资金。

退出条件：未知协议语义已消除，或明确转为失败关闭条件；没有猜测默认值。

人工门禁：AI 引用的协议语义必须给出来源（官方文档、真实响应样本或代码位置）；无法证实即上报，由所有者决定实测补充或转为失败关闭，AI 不得假设。

### 4.3 设计

AI 职责：产出设计文档，交叉检查所有调用者并列出删除或替换的旧路径。

必须产出：
- 设计文档
- 模块边界和唯一数据契约；
- 状态、错误和降级语义；
- 数据迁移或兼容策略；
- 验收场景到模块的映射；
- 删除或替换的旧路径。

退出条件：所有调用者有迁移方案；不存在新旧两套并行真相源。

人工门禁：所有者评审设计与删除项清单后再进入实现；AI 不得带着「顺手」扩展（遥测、通用框架、额外策略）进入设计。

### 4.4 实现

AI 职责：按本条规则编码并迁移所有调用点；每轮变更保持最小可审，按逻辑单元提交并说明每个改动影响哪些调用者。

规则：

- 先领域模型，再适配器，再业务编排，最后 CLI/运维入口；
- 迁移所有调用点并删除旧接口；
- 不用 lint 豁免、静默默认值或特殊输入分支掩盖设计问题；
- 不在本轮顺手扩展新策略、重试、遥测或通用框架。

退出条件：代码编译，目标路径端到端连通，没有占位实现；「编译通过」「测试通过」声明附带实际命令输出。

人工门禁：关键路径变更由所有者评审；所有者可要求 AI 回滚不属本轮范围的改动。

### 4.5 验证

AI 职责：运行全部门禁命令并留存原始输出，执行真实程序烟测；失败证据当场修复或明确上报阻塞。AI 的任何「通过」声明必须以命令输出、测试日志或真实烟测记录为证据，禁止转述未执行的结果。

至少包含：

1. 纯逻辑边界；
2. 适配器真实响应结构；
3. 失败关闭场景；
4. 实际程序烟测；
5. 格式与严格静态检查。

当前 Rust 门禁命令：

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
```

默认真实烟测不注入凭证，只访问公共只读接口；私有签名请求使用本地确定性 HTTP 夹具验证。输出必须包含两个方向、完整成本字段、账户元数据版本、准入决定和拒绝原因；当前市场没有正机会或实际账户费率不可用时，`REJECT` 是正确结果，不得降低门槛制造 `ACCEPT`。烟测经桌面控制台动作执行：`OBSERVE ONCE`、`ACCOUNT STATUS`、`RECONNECT SMOKE`。

退出条件：命令全部成功；实际程序走过本轮改变的路径；失败证据已修复或明确阻塞发布。

人工门禁：所有者复核验证证据与退出条件；仅单元测试或仅静态检查不构成完整验证。

### 4.6 发布

AI 职责：完成下列同步并提交候选发布记录。

必须完成：

- 更新架构文档“当前实现状态”；
- 更新本文件的追踪矩阵和发布记录；
- 配置样例与代码字段一致；
- 删除临时脚本、生成物和过期说明；
- 写明剩余风险和下一轮唯一入口。

退出条件：陌生维护者可以仅依据仓库文档复现验证，并知道系统不能做什么。

人工门禁：所有者审阅发布记录与剩余风险并签收；未签收不得标记 `released`。

## 5. AI 参与规则

### 5.1 角色与权限

- 所有者：唯一决策者——需求取舍、评审门禁、发布决定、资本与风险决策。
- AI 助手：执行者——编码、测试、文档起草、证据收集。
- AI 无决策权：不得在所有者未确认时进入下一阶段、扩大范围或宣称发布；所有权转移只发生在所有者明确确认后。

### 5.2 上下文与提示管理

- 每轮开始必须携带：关联需求、官方协议来源与版本、现有代码入口与全部调用者、本轮「明确不做」清单。
- 跨轮状态以归档与追踪矩阵为准；AI 不得依赖会话记忆推测历史事实。
- 结论必须可追溯：文件、符号、命令、输出；不可追溯的声称必须标注。

### 5.3 防幻觉规则

- AI 声称（协议语义、测试结果、运行行为、时间与数量）必须以来源或可复现输出背书；模型可能自信地给出错误答案，验证阶段不信任任何未经运行确认的声明。
- 不确定必须显式标注；禁止编造交易所字段、测试输出、时间与数量；数值必须来自真实样本或官方文档。
- 无法证实的语义按 §2 第 3、8 条转为失败关闭或上报所有者，绝不猜测默认值。

### 5.4 变更纪律

- 单轮单闭环；AI 不得顺手重构、加遥测、扩展策略或改无关代码。
- 每个修改必须可解释：为什么、影响哪些调用者、如何验证。
- 迁移所有调用者并删除旧路径是完成标准，不是可选项。

### 5.5 人工评审门禁

- 强制人工门禁：需求确认、设计评审、发布评审。
- AI 不得以状态标记或「已完成」代替人工确认；门禁未过时工作项保持 `ready`/`in_progress`，不标记 `verified`/`released`。
- 阻塞时冻结当前轮并可回滚，不绕过门禁继续。

### 5.6 审计与可逆性

- 关键决策、验证输出、发布记录保留在仓库（`docs/`、`data/`），随时间可查。
- 变更应可在必要时回滚；真实资金或外部副作用操作仅在所有者明确批准后发生。

## 6. 当前基线：A-03 WebSocket 本地订单簿

发布日期：2026-09-08。

### 6.1 范围

目标：以 Binance 与 Bybit 公共 WebSocket 增量流维护本地现货订单簿，替换周期 REST 深度快照作为扫描数据源；连接、序列或新鲜度失效时停止扫描，自动重建后才恢复。

不包含：账户费率、私有账户能力、行情归档、下单、资金预留和持久化。Binance REST 深度仅用于流启动及重建时的快照衔接。

### 6.2 需求追踪矩阵

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| A03-R01 | Binance 仅在 `lastUpdateId + 1` 被缓冲事件覆盖后发布本地簿 | `crates/core/src/venues/binance_stream.rs`, `crates/core/src/local_book.rs` | 快照覆盖、后继事件和断档边界用例 | released |
| A03-R02 | Binance 后续 `U/u` 断档立即撤下快照并自动重建 | `crates/core/src/venues/binance_stream.rs`, `crates/core/src/local_book.rs` | 断档用例；非 `VALID` 状态无快照用例 | released |
| A03-R03 | Bybit snapshot 覆盖当前代次，delta 绝对数量更新；旧 `seq` 不回滚本地簿 | `crates/core/src/venues/bybit_stream.rs` | 重启 snapshot 与乱序跨序列用例 | released |
| A03-R05 | WebSocket 静默、关闭、协议错误和主动重连均失败关闭；重建前不扫描 | `crates/core/src/venues/*_stream.rs`, `crates/core/src/observer.rs` | 双所主动断线及自动重建真实烟测 | released |
| A03-R06 | 单次与持续观察都只消费双方同时 `VALID` 的实时簿 | `crates/core/src/observer.rs` | 真实单次双向扫描（CLI 形态验证记录） | released |
| A03-R07 | 启动和扫描全程无 API key、无下单能力 | `crates/core/src/observer.rs`, `crates/core/src/venues/*` | 桌面应用与适配器仅访问公共 market REST/WebSocket | released |

### 6.3 已验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：通过，覆盖快照衔接、序列断档、乱序消息、绝对档位更新、状态失效和既有收益/准入逻辑。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- 真实单次双向扫描（结构重构前以 CLI 形态执行）：真实同步 Binance/Bybit BTCUSDT 本地簿后输出两个方向；当次接收时间偏差为 5 ms，两向均因完整成本后净收益为负而拒绝。
- 真实主动重连烟测（结构重构前以 CLI 形态执行）：先同步两所实时簿，再主动断开两条连接；Binance 与 Bybit 均从 generation 1 进入 generation 2，`reconnects=1`，自动重建并重新变为 `VALID`。

这些结果证明公共实时行情到净机会判断的只读路径及一次主动重连恢复可运行，不证明 24 小时连续稳定性、交易恢复或盈利能力。

### 6.4 追踪矩阵追加：G-01 模拟套利仪表盘（2026-09-09）

实现与验证详情见第 10 章发布记录 G-01 行及 `docs/G-01-模拟套利仪表盘-{需求,设计,任务}.md`。

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| G01-R01 | 侧栏「模拟套利」独立页，八大板块（统计/机会历史/交易明细获利/累计净盈亏曲线/流转图/余额与两账户详情变化图/活动日志/山脊图）均渲染数据或明确空态；页面切换不影响其他页 | `src/components/Simulation{Page,StatCards,Trades,ProfitChart,Flow,BalanceChart,Activity,Ridge}.vue`、`src/App.vue`、`src/components/SidebarNav.vue` | `npm run build` 零错误；浏览器 mock IPC 渲染断言（八板块、两账户详情、无横向溢出） | verified |
| G01-R02 | run 列表 22 列与单 run 详情（report 全文+意图/成交/执行事件/审计流）与 `simulation_runs` 投影一致；净盈亏全程 Decimal（serde-str） | `crates/core/src/simulation_query.rs`（`run_detail`）、`src/commands.ts`、`src/types.ts` | 探针 detail 断言：intents/trades/events/audit/balances 与 DB 直查一致；`audit=1`（LIKE 通配符修复后） | verified |
| G01-R03 | 累计模拟净盈亏曲线 + 扫描预期对照（SVG 折线/面积），数据点 hover；无数据空态；无第三方图表库 | `src/components/SimulationProfitChart.vue` | `npm run build` 零错误；浏览器 mock IPC 渲染断言 | verified |
| G01-R04 | 流转图按真实 `execution_events`/`audit_events` 聚合点亮节点并计数（evaluated/reserved/filled/compensated/completed/escalated） | `crates/core/src/simulation_query.rs`（`load_flow_aggregate`）、`src/components/SimulationFlow.vue` | 探针 flow 断言：evaluated=20 reserved=87 filled=59 compensated=16 completed=59 escalated=11 | verified |
| G01-R05 | 两账户详情卡（买腿 quote、卖腿 base：注入/成交后/补偿评估后三点）+ 余额变化曲线（USDT/BTC 双资产线） | `crates/core/src/simulation_query.rs`（`account_balances`/`balance_history`+`parse_sim_snapshot_id`）、`src/components/SimulationBalanceChart.vue` | 探针 balance_history=120 点、account_balances 与 `paper_balances` 一致；snapshot_id 8 段实测 | verified |
| G01-R06 | 机会历史全 run（含 Rejected/CompetedAway/DepthShortfall）、symbol/scenario 过滤、分页（limit≤200）；总数与 `simulation_runs` 行数一致 | `crates/core/src/simulation_query.rs`（`list_runs`）、`src/components/SimulationTrades.vue` | 探针 runs page total=20 rows=20；列表与投影一致 | verified |
| G01-R07 | 活动日志按时间降序审计事件流（`f02-*` 前缀聚合），run 归属由 aggregate/correlation id 解析 | `crates/core/src/simulation_query.rs`（activity）、`src/components/SimulationActivity.vue` | 探针 activity=50 条按降序返回 | verified |
| G01-R08 | 山脊图按日桶密度曲线堆叠 + 中位数标记，稀疏桶自动合并 | `crates/core/src/simulation_query.rs`（`ridge`+`merge_sparse_buckets`）、`src/components/SimulationRidge.vue` | 探针 ridge 桶数据与 `simulation_runs` 检索一致 | verified |
| G01-R09 | 查询层零写路径；无新增真实/测试网订单路径；`external_order_calls=0` 恒真 | `crates/core/src/simulation_query.rs`（`BEGIN READ ONLY` 独立连接） | 代码审查无写路径；F-02 smoke 单测不回归 | verified |
| G01-R10 | `SCHEMA_VERSION` 4→5、migration 0005 幂等；旧库任一命令自动升级；0001–0004 零改动 | `crates/core/src/db.rs`、`crates/core/migrations/0005_simulation_runs.sql` | `schema_migrations` 含 1–5；`cargo test --workspace`（190 core + 8 tauri-lib） | verified |
| G01-R11 | 五门禁全绿；纯逻辑测试（投影行/桶分组/前缀解析）净增 | `crates/core/src/simulation_query.rs`、`crates/core/src/simulation.rs`（`#[cfg(test)]`） | fmt/test/clippy/build/npm 全绿；`cargo test -p personal-taoli-core simulation::tests` 9 绿 | verified |

### 6.5 追踪矩阵追加：DB-01 数据库连接层 sqlx 池化重构（2026-09-10）

实现与验证详情见第 10 章发布记录 DB-01 行及 `docs/DB-01-数据库连接层sqlx池化重构-{需求,设计,任务}.md`。

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| DB01-R01 | 池化连接复用：`pool.size() <= 16`，`pg_stat_activity` 连接数稳定 | `crates/core/src/db.rs`（`pool()`/`POOLS` 缓存 + `PgPoolOptions::max_connections(16)` + `acquire_timeout(30s)`） | 驱动 U10：空闲连接数 7 ≤ 16；烟测前后 `pg_stat_activity` 稳定（6→6） | verified |
| DB01-R02 | 迁移幂等 + advisory 锁：连续两次 `migrate` 均 Ok，`schema_migrations` 行数不变 | `crates/core/src/db.rs`（`migrate_pool` 全程单会话 + `pg_advisory_lock(advisory_key("schema-migration","b01-paper-core"))` + 各迁移文件 `CREATE TABLE IF NOT EXISTS`） | 驱动 U08：两次 migrate 后 `schema_migrations` 仍 5 行（1–5）；`cargo test --workspace` 190+8 通过 | verified |
| DB01-R03 | schema 版本校验：版本不符 → 原文案 `bail!` | `crates/core/src/db.rs`（`verify_schema<E: PgExecutor>`） | 驱动 U09：`taoli_probe` 库注入 version 6 后 `unsupported B-01 schema version: expected 5, found Some(6)` 逐字命中 | verified |
| DB01-R04 | 领域单写者锁：`disconnect()` 后他人可取锁；同域第二写者被拒且文案逐字一致 | `crates/core/src/db.rs`（`DomainConnection::acquire/disconnect`：专享 `lock_connection` + `pg_try_advisory_lock` + `pg_advisory_unlock_all`；`after_release` 兜底防 panic 泄漏） | 驱动 U05/U06：释放后 `pg_try_advisory_lock` 重取为 true；持锁期间第二连接为 false；`PAPER execution domain already has a writer: account=… instrument=…` 逐字命中；paper smoke `same_domain_lock_rejected=true, concurrent_rejections=1` | verified |
| DB01-R05 | 只读层零写 + 不抢领域锁：只读事务内写被 PostgreSQL 拒绝；`external_order_calls=0` 恒真 | `crates/core/src/simulation_query.rs`（三公开入口各自 `pool.begin()` + `SET TRANSACTION READ ONLY` + `verify_schema(&mut *txn)` + `txn.rollback()`，不经 `DomainConnection::acquire`） | 驱动 U04：READ ONLY 事务内 INSERT 被拒、回滚后同一池 INSERT 成功（池未污染）；七个域 smoke `external_order_calls=0`；G-01 三只读入口返回真实数据且不阻塞并发 smoke | verified |
| DB01-R06 | 六个单写者域行为等价（PAPER/订单事实/双腿执行/账务/控制/对账） | paper/order/execution/accounting/control/reconciliation.rs 全量迁移至 `self.inner.pool`（`.begin()` 事务 + `try_get` 解码 + `rows_affected` 计行） | 六域 smoke 全绿：全部行为标志 true、`external_order_calls=0`（paper 0.405s / order 0.107s / execution 0.088s / accounting 0.026s / control 0.030s / reconciliation 0.023s） | verified |
| DB01-R07 | F-02/G-01 编排与降级不变：`TAOLI_DATABASE_URL` 缺失 → 告警 + `None`，观察循环不中断 | `crates/core/src/simulation.rs`（`SimulationEngine::spawn` `std::env::var` 失败分支） + `crates/core/src/observer.rs`（`if let Some(engine) = simulation.as_ref()` 守卫） | 驱动 U07：URL 缺失 → `engine None` + warn；观察循环以 None 引擎继续 3 tick | verified |
| DB01-R08 | `try_get` 收敛 panic 为 Result（行为变化：错误路径由 panic 变 Err，期望变更） | 全部行读取点 `row.get` → `row.try_get::<_,T>(i)?` | `cargo clippy --workspace --all-targets` 零警告（`-D warnings` 兜底）+ 七域 smoke 全绿覆盖真实行读取 | verified |
| DB01-R09 | Decimal/JSONB 精确往返 | 全部金额/快照字段经 `try_get::<Decimal,_>` / serde JSON 解码 | 七域 smoke 断言金额字段（如 order `recovered_filled_quantity=0.01`）；G-01 总净盈亏 Decimal 精确显示 | verified |
| DB01-R10 | `verify_schema` 泛型 executor：编译通过即证 | `crates/core/src/db.rs` `verify_schema<'e, E: PgExecutor<'e>>`；调用点 paper.rs:159 / simulation_query.rs:273（pool、`&mut *txn`、`&mut *connection` 三种执行器） | `cargo build --workspace` 通过 | verified |
| DB01-R11 | 依赖清理干净：`tokio_postgres` 全仓移除、无死 feature | 两个 Cargo.toml（`sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio","postgres","rust_decimal","json"] }`；`rust_decimal` 去 `db-postgres`；`src-tauri` 删 `tokio-postgres`） | `grep -rn "tokio_postgres\|tokio-postgres" crates src-tauri Cargo.toml` 全仓为空；`cargo tree -p personal-taoli-core` 无 tokio-postgres（含传递依赖） | verified |

### 6.6 追踪矩阵追加：PERF-01 读路径性能与重复代码收敛（2026-09-10）

实现与验证详情见第 10 章发布记录 PERF-01 行及 `docs/PERF-01-读路径性能与重复代码收敛-{需求,设计,任务}.md`。**流程偏差**：本迭代需求确认、设计评审与实现在同一会话内完成，三份文档系事后补正（需求文档 §5 记录所有权确认范围为「执行哪几项」）；人工门禁中设计评审与发布签收待所有者完成。

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| PERF01-R01 | 迁移与 schema 校验按 `database_url` 在进程内只执行一次；迁移语义（单会话 advisory 锁、`raw_sql` 整份执行、幂等）不变 | `crates/core/src/db.rs`（`READY: LazyLock<Mutex<HashSet<String>>>` + `ready_pool`；`migrate` 委托之） | 实库驱动：`cold_overview_ms=191` → `warm_overview_ms=42`；二次调用后 `schema_migrations` 仍 6 行未增长；跨进程重跑幂等 | verified |
| PERF01-R02 | 五类只读查询有可用索引；纯增量、无表结构变更；`SCHEMA_VERSION` 5→6 | `crates/core/migrations/0006_read_path_indexes.sql`（`text_pattern_ops` ×2 + `occurred_at_ms DESC` + `trade_facts(intent_id, occurred_at_ms)` + `balance_snapshots(source, observed_at_ms DESC)`） | `SET enable_seqscan=off` 下 `EXPLAIN` 由 `Seq Scan on audit_events (rows=555)` / `Seq Scan on trade_facts (rows=928, Removed 928)` 变为 `Index Scan using audit_events_occurred_at` / `Index Scan using trade_facts_intent_id`；psql 确认 6 索引落库、版本至 6 | verified |
| PERF01-R03 | 累计曲线服务端前缀和 + 窗口上限；前端不做累加 | `crates/core/src/simulation_query.rs`（`CUMULATIVE_LIMIT=1200` + `SUM(...) OVER (ORDER BY ...)`）；`src/components/SimulationProfitChart.vue` | 驱动：`cumulative_points=564`（≤1200）、`monotonic_prefix_sums=true`、末点 `scanned=28.2433000 / simulated=-785.457318630`；前端无 `+=` 累加路径 | verified |
| PERF01-R04 | 山脊样本窗口上限 | `crates/core/src/simulation_query.rs`（`RIDGE_LIMIT=4000`） | 驱动：`ridge_buckets=2`、`ridge_points=488`（≤4000）；桶合并语义不变（既有用例） | verified |
| PERF01-R05 | 流转聚合 6 条查询→2 条；`filled_runs` 反映真实双腿成交而非 `COMPLETED` 事件数 | `crates/core/src/simulation_query.rs::load_flow_aggregate`（`count(DISTINCT ...) FILTER` + 标量子查询；`filled_runs` 按 `bought_quantity>0 AND sold_quantity>0`） | 驱动：`filled_runs=403` 与 `completed_runs=373` **相互独立**（原实现两者为逐字相同的 SQL）；`evaluated=488 / reserved=555 / compensated=96 / escalated=85` | verified |
| PERF01-R06 | 详情同表三次往返合并为一次；投影列清单两处共用同一常量 | `crates/core/src/simulation_query.rs`（`RUN_PROJECTION` + `run_row_at(row, base)`；`fetch_optional` + `bail!`） | 驱动：`intents=2 trades=2 events=1 audit=1 balances=6 report_keys=38`（与 DB 直查一致）；`missing_run_is_err=true`（失败关闭）；顺带修掉 `get_simulation_runs` 原先不迁移的不一致 | verified |
| PERF01-R07 | 合约过滤选项来自配置 `pairs[].symbol`，值域与 `simulation_runs.symbol` 存储值一致 | `src/components/SimulationTrades.vue`（`symbolOptions` ← `getObserverConfig().pairs`，删除两个硬编码 `ElOption`） | 驱动：`filter_options` 为 8 个交易对；`filter_option_BTCUSDT_total=564`（非空）；`slash_form_total=0`（证明精确匹配，原前端字面量必然 0 行）；`scenario_filter_total=244`（双过滤） | verified |
| PERF01-R08 | 前端累计曲线直接消费后端字段 | `src/components/SimulationProfitChart.vue` | 代码审查无 `parseFloat` 累加；末点数值与后端字段一致 | verified |
| PERF01-R09 | 密度曲线成本不随样本数线性放大；不对大数组用展开运算符求极值 | `src/components/SimulationRidge.vue`（`BucketStats` 预解析、`density()` 直方图+核卷积、`median()`、单遍求极值） | 代码审查：成本 = `BINS`(56) × 核半径(7)，与样本数无关；`Math.min(...values)`/`Math.max(...values)` 已消除。**未做前后计时对比** | verified |
| PERF01-R10 | 本地簿增量不再构造并丢弃快照；解析逻辑单实现；`LevelUpdate` 单定义 | `crates/core/src/local_book.rs`（`validate_live`）、`crates/core/src/market.rs`（按值消费的泛型签名 + 唯一 `LevelUpdate`）、`crates/core/src/venues/{binance,bybit,binance_stream,bybit_stream}.rs`（删 6 份副本） | `cargo test --workspace` 190→**192** core 全绿；新增 `apply_rejects_a_crossed_book` / `apply_rejects_emptying_a_side` 锁定重写后的两条不变量（空边、交叉盘）；`grep` 全仓无重复定义 | verified |
| PERF01-R11 | 模拟引擎 worker 在观察循环 `?` 早退时也不存活 | `crates/core/src/simulation.rs`（`impl Drop for SimulationEngine`：置 `stopped` + `worker.abort()`） | 交付物经 `cargo build`/`clippy -D warnings` 验证。**验证缺口**：未写「abort 后任务消失」的运行时断言（见设计 §7 R-5） | verified |
| PERF01-R12 | 新格式配置不被解析两遍 | `crates/core/src/config.rs`（`migrate_legacy_{toml,json}` 返回 `Cow`，前置子串分流） | 既有配置加载用例全绿；代码审查：新格式路径零 `Value` 解析与零重序列化 | verified |

**明确不做（不得被解读为已完成）**：16 个无生产调用点模块（6304 行 / 132 测试）的接线或删除，其中 `performance.rs`/`system_health.rs`/`notification.rs`/`alert_manager.rs` 含只增不删的无界容器，**接线前必须先补淘汰策略**；`BookFeedStatus` 的 `Arc<str>` 化（AI 主动撤回，实测约 8 µs/s 不成比例）；`DetailPanel.vue` 拆分与概览页静态装饰组件去重；冒烟报告 TS 判别联合。

### 6.7 追踪矩阵追加：MIG-01 迁移机制交由 sqlx 接管（2026-09-10）

实现与验证详情见第 10 章发布记录 MIG-01 行及 `docs/MIG-01-sqlx迁移统一-{需求,设计,任务}.md`。

**契约替换声明（必读）**：本迭代替换了已发布迭代的**迁移记账机制**。所有者从三段式选项中前置选定「全量交给 sqlx（`_sqlx_migrations`）」，并知悉其代价为改写 4 条已发布需求断言。下列需求的**断言文本已同步改写**，其可观察结论仍然成立，故不标为失效：

- G01-R10 / S-G01-1：原断言 `schema_migrations` 含 5、`verify_schema` 通过 → 改为版本账本记录 1–5 且 `success` 全真。
- DB01-R02（迁移幂等 + advisory 锁）、DB01-R03（版本不符 `bail!`）：实现路径已替换为 sqlx 内建机制，性质保留。
- PERF01-R01（就绪库单点）、PERF01-R02（读路径索引）：`READY` 守卫与索引本身不变；迁移与记账方式改由 sqlx 承担。

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| MIG01-R01 | 迁移来源为编译期嵌入的整个目录；`db.rs` 无 `include_str!` 迁移常量与文件清单 | `crates/core/src/db.rs`（`static MIGRATOR: Migrator = sqlx::migrate!("./migrations")`） | `grep include_str! crates/core/src/db.rs` 为空；新增迁移只需新增文件 | verified |
| MIG01-R02 | 报告版本号由嵌入文件集最大版本推导，无手写常量 | `crates/core/src/db.rs::schema_version()`；`paper.rs`/`simulation.rs` 4 处调用点 | `grep -rn "SCHEMA_VERSION" crates/core/src` 仅剩归档域同名常量（不同机制） | verified |
| MIG01-R03 | 记账由 `_sqlx_migrations` 承担；自建表与逐文件 INSERT 全删 | `db.rs` + 0001–0006 尾部剥离 | 全新库 1..7 全部 `success`、`execution_time>0`；旧表 ABSENT | verified |
| MIG01-R04 | 已应用文件被改动后硬失败 | sqlx 内建 SHA-384 校验 | 篡改 checksum → `migration 3 was previously applied but has been modified` | verified |
| MIG01-R05 | 文件与记账同事务，半应用可识别 | sqlx 内建（全部文件 `no_tx=false`） | 代码审查；未实测 `Dirty`（见任务文档 §2.3） | verified |
| MIG01-R06 | 存量库采纳不重跑任何文件且保留原始应用时间 | `db.rs::adopt_legacy_bookkeeping`（`ON CONFLICT DO NOTHING`；`installed_on = to_timestamp(applied_at_ms/1000.0)`） | `1700000000000+v*1000` → `2023-11-14 22:13:2{v}+00` 逐行精确；采纳行 `execution_time=-1` | verified |
| MIG01-R07 | 旧库版本高于本二进制时失败关闭，且不执行任何迁移 | `adopt_legacy_bookkeeping` 内逐版本 `MIGRATOR.version_exists` 校验（在 `run` 之前） | 旧表含 99 → `unsupported B-01 schema version: expected at most 7, found 99`，无迁移副作用 | verified |
| MIG01-R08 | 旧表由独立迁移 0007 删除 | `crates/core/migrations/0007_drop_legacy_schema_migrations.sql` | 三条路径（全新/v4/存量）均 ABSENT | verified |
| MIG01-R09 | 空库路径不受引导逻辑影响 | 引导以「旧表存在且 `_sqlx_migrations` 为空」为前置 | 全新库旧表从未创建；22 表/47 索引/11 触发器与 v4 升级结果收敛一致 | verified |
| MIG01-R10 | 对外签名不变、每 URL 只迁移一次、`acquire` 仍保证就绪 | `migrate`/`ready_pool` 签名不变；`DomainConnection::acquire` 改走 `ready_pool` | 调用点零改动；冷热 338ms → 29ms（PERF-01 为 191ms → 42ms，量级保持） | verified |
| MIG01-R11 | 除记账语句外迁移 SQL 逐字不变 | 0001–0006 | 逐文件 diff 仅含尾部 INSERT 删除（0001 另删旧表 CREATE） | verified |
| MIG01-R12 | 依赖旧记账表的已发布断言全部同步 | G-01/DB-01/PERF-01 需求文档 | 见上方「契约替换声明」；三份文档均已加注 MIG-01 口径 | verified |

**执行顺序偏差（如实记录）**：所有者机器上运行中的 `tauri dev` 在本二进制编译完成后自动重载并完成迁移，早于 AI 执行的 `pg_dump`，故 `data/backup/` 中的备份是迁移**之后**的快照。已核实迁移未重跑任何已应用文件、业务表行数只增不减、结构与全新库收敛一致，破坏性后果未发生。流程教训（应「停应用 → 备份 → 迁移」）记入设计文档 §7 R-6 与任务文档 §3。

**明确不做**：不引入 `sqlx migrate` CLI 作为部署路径；不引入 `query!` 编译期校验；不生成 down 迁移；不合并或重写历史迁移。

### 6.8 追踪矩阵追加：G-02 模拟 run 来源判别（2026-09-10）

实现与验证详情见第 10 章发布记录 G-02 行及 `docs/G-02-模拟run来源判别-{需求,设计,任务}.md`。

**触发来源**：所有者观察运行中的桌面应用，看到 `run … symbol=BTCUSDT` 而自己聚焦的是 ETH，判断「日志与选择不符」。排查表明**选择机制并无问题**，真实缺陷是三条独立事实叠加后被一个模糊呈现放大：

1. 该 run 来自**合成簿烟测探针**（`run_simulation_smoke`），不是实时市场模拟——全库 620 条 run 的 `buy_venue` 恒为 `s01-buy`…`s07-sell`；
2. 探针的 symbol 是夹具常量 `"BTCUSDT"`，与当前观察的币对无关；
3. 实时模拟从未产生任何 run——8000 条 decision 中 `accepted` 为 **0**，准入按设计失败关闭（凭证未配置、Bybit 资格未确认、实际费率 observation-only、名义金额低于下限）。

缺陷在于**仪表盘把探针产物与市场仿真结果混同呈现**，既无来源列也无界面标注。

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| G02-R01 | `simulation_runs.source ∈ {SMOKE, LIVE}`，SQL 层可区分，不再依赖场所命名约定 | `crates/core/migrations/0008_simulation_run_source.sql` | 列存在、`NOT NULL DEFAULT 'LIVE'`、CHECK 生效；`_sqlx_migrations` 含 8 且 `success` | verified |
| G02-R02 | 探针标 `SMOKE`、实时标 `LIVE`，来源由调用方显式传入 | `crates/core/src/simulation.rs`（`SimulationRunSource` + `run_simulation_run` 形参 + 7 烟测 / 1 实时调用点） | 实跑烟测：`before 613/613/0` → `after 620/620/0`，`newest source=SMOKE` | verified |
| G02-R03 | 存量行全部回填为探针 | 同上（`ADD COLUMN … DEFAULT 'SMOKE'`） | 迁移前 613 行 → `smoke_runs=620`（含校验期间新增）、`live_runs=0` | verified |
| G02-R04 | 回填不得削弱不可变保护 | 不 `DROP TRIGGER`（用默认值回填） | `pg_trigger simulation_runs_immutable` 仍存在；对既有行 `UPDATE` 仍报 `audit_events are immutable` | verified |
| G02-R05 | 概览透出 `smoke_runs` / `live_runs` | `crates/core/src/simulation_query.rs`（`agg` 追加两个 `COUNT(*) FILTER`） | 计数与 SQL 直查一致（620 / 0） | verified |
| G02-R06 | 界面区分来源：统计卡来源提示 + 列表/详情徽章 | `SimulationStatCards.vue`、`SimulationTrades.vue` | mock IPC 渲染断言：提示文案与徽章文案命中；视觉确认为警示配色 | verified |
| G02-R07 | 烟测入口与日志自我说明是合成数据 | `SimulationPage.vue`、`run_simulation_smoke` 横幅 | 页面文本含「运行合成簿烟测」+ 合成说明；日志含 `SYNTHETIC FIXTURES … not market data` | verified |
| G02-R08 | 实时为 0 时解释原因而非留白 | `SimulationStatCards.vue`（`provenance` 三分支） | 页面文本含「实时 run 0 条 —— 准入未通过（凭证/资格未配置，见控制台账户状态）」 | verified |
| G02-R09 | 模拟页合约过滤保持页内独立，选项值域与库中一致 | `SimulationTrades.vue`（PERF-01 已修值域，本轮不改语义） | 8 个配置币对均可过滤；BTCUSDT 命中 627、其余 0（正确：无 ETH run 存在） | verified |
| G02-R10 | 撮合/准入/补偿语义不变；无新增订单路径 | 全部 | `cargo test --workspace` 192+8 全绿；烟测 `external_order_calls=0`；五门禁全绿 | verified |

**失败与修正（如实记录）**：0008 首次实现用 `UPDATE` 回填，被 `simulation_runs_immutable` 拒绝，应用报 `SIMULATION_ERROR: failed to apply database migrations`。改为默认值回填后成功。失败**零残留**（无 `Dirty`、`source` 列与 version=8 行均随事务回滚），因 sqlx 使每个迁移文件与其记账同事务——这是 MIG-01 R-3 的首次实测，风险等级下调但仍保留（真正的半应用只可能来自 `-- no-transaction` 文件，本项目无）。教训：**本项目全部事实表均有 `BEFORE UPDATE OR DELETE` 不可变触发器（共 11 个），后续迁移一律不得 `UPDATE` 既有事实行**。

**验证缺口**：`LIVE` 分支（实时 run 落库与「全实时 / 混合」提示）无真实数据可触发，仅有代码层面保证；阻塞于准入前置条件（凭证、资格、费率），属阶段 F。

**明确不做**：不改烟测的 symbol 取值（合成簿不是 ETH 行情，改标签等于伪造来源）；不接管全局「聚焦币种」（所有者选定页内独立过滤）；不让概览聚合随合约筛选变化；不为制造实时 run 降低任何准入阈值。

## 7. 阶段 A 剩余迭代队列

按顺序执行。除非当前项被正式拒绝或阻塞，不并行开启后项。

### A-03 WebSocket 本地订单簿（released）

已于 2026-09-08 发布。范围、追踪矩阵和验证证据见第 6 章。

本轮独立产物：

- [需求文档](A-03-WebSocket本地订单簿-需求.md)
- [设计文档](A-03-WebSocket本地订单簿-设计.md)
- [任务文档](A-03-WebSocket本地订单簿-任务.md)

### A-04 能力卡与账户实际费率（released）

已于 2026-09-08 发布。目标是把“交易所支持什么”和“该账户实际成本是多少”转为带来源、版本和有效期的数据；明确不包含真实交易权限和下单。

本轮独立产物：

- [需求文档](A-04-账户能力与实际费率-需求.md)
- [设计文档](A-04-账户能力与实际费率-设计.md)
- [任务文档](A-04-账户能力与实际费率-任务.md)

#### A-04 需求追踪矩阵

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| A04-R01 | 两所能力卡覆盖资格确认、订单恢复、限频、client order ID、IOC/FOK、费用币种和历史窗口 | `crates/core/src/account.rs::capability_card` | `account_status` 输出完整卡片及官方来源 | released |
| A04-R02 | 未配置或只配置一半凭证时不发私有请求，使用配置回退费率并明确标记 observation-only | `crates/core/src/account.rs::{Credentials,fallback_account}` | 缺失和半配置凭证用例；无凭证真实账户检查 | released |
| A04-R03 | Binance 只在读取已启用且交易、提现、转账及衍生品危险 scope 全部关闭时判为只读 | `crates/core/src/account.rs::binance_permissions` | 本地 HTTP 权限响应夹具 | released |
| A04-R04 | Bybit 按 `readOnly` 判定，并在提现权限存在时额外拒绝 | `crates/core/src/account.rs::bybit_permissions` | 本地 HTTP 权限响应夹具 | released |
| A04-R05 | 两所请求按各自协议签名并加载账户实际 taker 费率；非法费率失败关闭，不同费率改变净收益 | `crates/core/src/account.rs::{binance_get,bybit_get,binance_fee,bybit_fee}`, `crates/core/src/scan.rs` | 官方 Binance HMAC 向量；两所请求/响应夹具；`fee_change_can_flip_admission_for_the_same_books` | released |
| A04-R06 | 实际费率记录 symbol、来源、加载和过期时间；不可用或过期时只观察 | `crates/core/src/account.rs::{FeeSchedule,AccountData::admission_rejections}` | 费率解析、回退和过期边界用例 | released |
| A04-R07 | 两所账户加载互不阻塞；单所失败保留另一所结果，失败侧回退并拒绝准入 | `crates/core/src/account.rs::{load_account_data,load_venue}` | 并行加载与错误归一化行为检查 | released |
| A04-R08 | 账户检查独立输出元数据；单次和持续扫描使用账户费率，持续模式按间隔刷新 | `crates/core/src/observer.rs` | 真实账户检查与公共双向扫描；持续刷新编排检查 | released |
| A04-R09 | key、secret、签名和完整认证请求头不进入序列化输出或归档 | `crates/core/src/account.rs`, `crates/core/src/observer.rs` | `Zeroizing` 类型边界、错误脱敏和桌面应用输出检查 | released |

#### A-04 已验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：通过；覆盖官方签名向量、两所签名请求与响应解析、凭证缺失、费率有效期、费率改变准入和错误脱敏。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- 无凭证账户检查（结构重构前以 CLI 形态执行，入口现为桌面 `account_status`）：默认无凭证配置输出两所能力卡、保守回退费率和明确拒绝原因；回退费率标记为 observation-only。
- 真实单次双向扫描（结构重构前以 CLI 形态执行）：真实同步 Binance/Bybit BTCUSDT 公共本地簿并输出两个方向；账户资格未确认、凭证缺失和实际费率不可用共同使两向保持 `REJECT`。
- 本轮未提供真实账户凭证。真实 Binance/Bybit 账户的只读权限、IP 限制和实际费率尚未实测；签名私有接口由本地确定性 HTTP 夹具验证，不能替代上线前账户检查。

官方协议来源：Binance Spot REST API 与 filters 文档；Bybit V5 Integration Guidance、API Key Information、Fee Rate、Create Order、Open/Closed Orders 和 Rate Limit 文档。能力卡输出保留直接来源 URL。

### A-05 行情归档与连续影子统计（实现已验证，连续观察 pending）

目标：保存可复现的行情、规格版本、费率版本、决策和数据缺口，形成连续影子运行报告。

本轮独立产物：

- [需求文档](A-05-行情归档与连续影子统计-需求.md)
- [设计文档](A-05-行情归档与连续影子统计-设计.md)
- [任务文档](A-05-行情归档与连续影子统计-任务.md)
- [连续影子观察运行与评审说明](A-05-连续影子观察-运行与评审说明.md)

#### A-05 需求追踪矩阵

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| A05-R01 | 每条决策关联两所行情代次与盘口、品种规格、费率、配置内容版本和扫描结果 | `crates/core/src/archive.rs::{DecisionEvent,ArchiveRecord}` | 精确归档回放用例；真实单次归档 | implemented |
| A05-R02 | 无法扫描时保存健康状态与跳过原因；归档记录带唯一运行 ID、运行内序号、模式和代码版本 | `crates/core/src/archive.rs::{HealthEvent,ArchiveRecord}` | 混合决策/健康事件回放用例 | implemented |
| A05-R03 | 主路径通过有界队列非阻塞提交；队列溢出或文件写入失败形成独立缺口记录 | `crates/core/src/archive.rs::ArchiveWriter` | 不可写归档路径产生 1 条缺口且提交/关闭不失败 | implemented |
| A05-R04 | 按固定文件长度快照顺序流式回放，只使用归档数据重新计算；事件时间倒退、运行内序号不递增或结果不一致即失败 | `crates/core/src/archive.rs::replay_archive` | 精确回放通过；篡改决策、完整损坏行和时间倒退均拒绝回放 | implemented |
| A05-R05 | 影子报告包含时长、重连、机会、净收益、容量、稳定类别拒绝原因、尾部样本、缺口和忽略的未完成尾字节 | `crates/core/src/archive.rs::ShadowReport` | 单元用例核对统计；真实运行中归档可在线回放 | implemented |
| A05-R06 | 至少连续观察 14 天；两个方向合计至少 100 个独立正净收益候选事件段；两所各至少一次自然重连且至少一次真实失效恢复 | `data/archive/a05-shadow-20260908.ndjson` 及评审记录 | 2026-09-08T09:52:14Z 已启动独立窗口；最早 2026-09-22T09:52:14Z 评审 | pending（运行中） |

#### A-05 已验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：38 项通过；归档用例覆盖精确回放及统计、决策篡改拒绝、归档 I/O 失败、在线未完成尾记录、崩溃尾部修复、完整损坏行、时间倒退和拒绝类别有界聚合。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- release 二进制持续模式接受 Ctrl-C/SIGTERM，先关闭有界归档队列并等待写线程刷盘；受控 SIGTERM 实测退出码为 0，停止后 36 条记录精确回放且未完成尾字节为 0。（该实测为 CLI 形态；现桌面形态由 SIGTERM/关窗进入同一退出冲刷路径。）
- 正式窗口（CLI 形态）首批 15 条实时 Binance/Bybit BTCUSDT 决策在运行中完成固定长度快照回放：`direction_evaluations=30`、`gap_records=0`、`ignored_incomplete_tail_bytes=0`、`replayed_without_mismatch=true`。

#### A-05 连续观察运行记录

- 窗口起点：2026-09-08T09:52:14Z；运行 ID：`1788861134042-72723-1`；最早 14 天评审时间：2026-09-22T09:52:14Z。
- 受管进程：`taoli-shadow-a05`（监督桌面进程）；命令：`TAOLI_OBSERVER_AUTOSTART=1 TAOLI_OBSERVER_ARCHIVE=data/archive/a05-shadow-20260908.ndjson ./target/release/personal-taoli`；故障退出自动重启。自 2026-09-09T00:43:28Z 起运行 ID 前缀为 `1788914607692-64268-1`（纯 Tauri 桌面形态接管，详见运行说明 §9 对应记录）。
- 原始归档和缺口日志位于 `data/archive/`，已排除版本控制；预检数据保留在默认 `observations.ndjson`，不混入正式窗口报告。
- 评审前必须同时核对运行时长、运行 ID/序号连续性、缺口与忽略尾字节、两所各至少一次自然重连、至少一次真实 `非 VALID → VALID` 恢复、按需求文档定义的至少 100 个独立正净收益候选事件段、收益与容量分布、拒绝类别和尾部样本。仅满 14 天不能发布 A-05。

实现完成不等于 A-05 发布。14 天连续影子窗口及规定的独立事件段和自然故障样本仍待积累；期间不得降低准入阈值或缩短事件段间隔制造样本。

阶段 A 退出条件：完整能力卡、动态实际费率、有效本地簿、可复现归档和影子报告均通过；仍不具备下单能力。

## 8. 后续阶段队列

### B-01 耐久化意图、资金预留与单写者

先建立 PostgreSQL 事务模型、账户—品种单写者、资金/额度预留、幂等键和审计事件。使用无外部副作用的模拟交易所验证并发竞争、进程崩溃和恢复；不得连接真实下单接口。

### B-02 订单事实与 UNKNOWN 状态机

实现提交、查单、撤单、私有事件、成交补拉和 UNKNOWN 调查状态。验收请求超时、响应丢失、查询暂未找到、重复/乱序成交、撤单竞态和重启恢复。所有副作用先写意图，再调用适配器。

### B-02 订单事实与 `UNKNOWN` 状态机（released）

B-02 于 2026-09-08 发布。范围限定为 PAPER/模拟适配器：提交、查询、撤单和成交事实先持久化，超时进入 `UNKNOWN`，查询暂未找到不改变未知态，明确拒绝进入 `DEFINITELY_REJECTED`，撤单竞态与重复成交均保留可审计事实；不连接真实或测试网订单接口。

#### B-02 需求追踪矩阵

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| B02-R01 | 提交动作只从持久化 `NOT_SENT` 意图开始，接受后绑定唯一交易所订单 ID | `crates/core/src/order.rs::submit_started`, `submit_result` | PostgreSQL 订单事实烟测 | released |
| B02-R02 | 明确拒绝进入 `DEFINITELY_REJECTED`，不生成成交事实 | `crates/core/src/order.rs::SubmitResult::DefinitelyRejected` | 烟测 `submit_definitely_rejected=true` | released |
| B02-R03 | 超时进入 `UNKNOWN`，禁止直接重发 | `crates/core/src/order.rs::SubmitResult::Unknown` | 烟测 `submit_timeout_unknown=true` | released |
| B02-R04–R05 | 查询暂未找到保持 `UNKNOWN`，查询确认恢复并绑定原订单 | `crates/core/src/order.rs::query_result` | 烟测 `query_not_found_preserved_unknown=true`, `query_found_recovered=true` | released |
| B02-R06 | 撤单请求、撤单结果和撤单后成交独立保留 | `crates/core/src/order.rs::{cancel_requested,cancel_result,record_trade}` | 烟测 `cancel_race_trade_preserved=true` | released |
| B02-R07 | 相同成交幂等，内容冲突进入 `CONFLICT`，不重复累计数量 | `crates/core/src/order.rs::record_trade` | 烟测 `duplicate_trade_ignored=true` | released |
| B02-R08 | 重启恢复未终态订单事实，不自动发送 | `crates/core/src/order.rs::recover_nonterminal` | 烟测恢复状态与成交数量输出 | released |

#### B-02 已验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：44 项通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- `TAOLI_DATABASE_URL=postgresql://taoli:taoli@127.0.0.1:55432/taoli cargo test -p personal-taoli-core order::tests -- --nocapture`：通过；明确拒绝、超时未知、暂未找到、查单恢复、成交去重、撤单竞态和恢复均为 `true`，`recovered_filled_quantity=0.01`，`external_order_calls=0`。

该证据只证明 PAPER 数据库事实和无外部订单副作用，不证明真实交易所订单恢复、私有流或实盘安全；B-02 不解除 A-05 连续观察门槛，也不产生真实下单能力。

### B-03 双腿执行与补偿风控（PAPER 已验证，发布受 A-05 门禁约束）

实现受预算约束的双腿计划、部分成交、未匹配敞口和补偿决策。`execution_facts` 是当前快照，执行事件与补偿决定按计划版本不可变持久化；敞口/预算超限升级 `MANUAL_REQUIRED`，不伪造中性状态。当前实现只接受已持久化成交汇总，不发送订单或补偿交易；A-05 连续观察未完成前不得接入真实订单适配器。

#### B-03 需求追踪矩阵

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| B03-R01–R03 | 部分成交、预算内补偿计划、后续匹配完成 | `crates/core/src/execution.rs::ExecutionCore::evaluate` | `run_paper_smoke("B03")` partial/matched 字段 | verified |
| B03-R04–R05 | 敞口或补偿预算超限进入人工 | `crates/core/src/execution.rs::ExecutionCore::evaluate` | `over_budget_manual_required=true` | verified |
| B03-R06 | 成交单调性、目标数量和终态保护 | `crates/core/src/execution.rs`、`0003_double_leg_execution.sql` | Rust 单元测试与数据库约束 | verified |
| B03-R07 | 重启/断开后恢复人工状态与敞口事实 | `ExecutionCore::load` | `recovered_manual_state=true` | verified |
| B03-R08–R09 | 域锁、数据库事实唯一版本、失败关闭 | `ExecutionCore::acquire`、migration | 编译、严格 Clippy、PAPER 烟测 | verified |

#### B-03 已验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：45 项通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- `TAOLI_DATABASE_URL=postgresql://taoli:taoli@127.0.0.1:55432/taoli cargo test -p personal-taoli-core execution::tests -- --nocapture`：通过；`partial_fill_detected`、`within_budget_compensation_planned`、`matched_completion`、`over_budget_manual_required`、`recovered_manual_state` 均为 `true`，`external_order_calls=0`。

该证据只证明 PAPER 数据库事实计算与无外部订单副作用，不证明真实订单状态、补偿成交、费用、账务或盈利能力；B-03 不解除 A-05 连续观察门槛。

### B-04 账务、对账与控制面（PAPER 已验证，发布受 A-05 门禁约束）

建立多币种复式账务、账户快照、差异分类、暂停/撤单/减仓/停止语义,以及管理命令授权和审计。实现已拆分为 `crates/core/src/accounting.rs`、`reconciliation.rs`、`control.rs` 与 `migrations/0004_accounting_reconciliation_control.sql`;Tauri 通过 `run_accounting_control_smoke` 提供三个独立的只读控制面烟测入口。

#### B-04 需求追踪矩阵

| 需求 | 可观察结果 | 实现位置 | 验证证据 | 状态 |
|---|---|---|---|---|
| B04-R01–R03 | 复式账务按资产平衡、业务键幂等、历史不可变 | `accounting::LedgerCore`、`0004_*` | PostgreSQL smoke | verified |
| B04-R04–R06 | 余额快照、匹配/缺失/冲突差异分类与持久化 | `reconciliation::ReconciliationCore` | PostgreSQL smoke | verified |
| B04-R07–R09 | 控制命令授权边界、幂等请求、过期命令不执行、审计记录 | `control::ControlCore`、`commands/accounting_control.rs` | PostgreSQL smoke、Tauri API | verified |

#### B-04 已验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：48 项通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- `npm run build`：通过；Vue 类型检查与 Vite 生产构建均完成。
- 临时 PostgreSQL B04 smoke：账务、对账、控制面全部通过；`schema_version=4`，`external_order_calls=0`。

该证据只证明 PAPER 数据库事实、审计与控制命令的失败关闭语义，不证明真实余额流水、交易所私有接口、自动补偿或真实订单能力；B-04 不解除 A-05 连续观察门槛。

阶段 B 退出条件：架构文档 AT-01 至 AT-22 中所有适用 P0 用例通过，崩溃恢复和账务差异闭环完成。是否申请交易权限是后续独立决策，不因代码完成自动发生。

### C：回放与运维验证

完成历史回放、故障注入、告警、备份恢复、RPO/RTO 实测、主备 fencing 和运行手册演练。

### D：受限现货实盘

仅在所有上线门槛通过且所有者显式批准资本和最大损失后开始。首次金额只满足交易所最低额并受绝对损失限额保护；每日结算真实费用、成交偏差、补偿成本和未解释差异。

### E：现货—永续

独立需求和风险审批。必须新增保证金、强平、ADL、标记价、资金费结算时间、基差和组合退出模型；不得复用现货中性假设。

## 9. 工作项模板

每次开始新迭代，复制以下内容到发布记录前：

```text
迭代编号：
目标与用户可观察结果：
明确不做：
关联需求/验收用例：
官方协议来源及版本：
现有代码入口与所有调用者：
AI 上下文来源（需求/协议/调用链/边界依据）：
数据契约/状态机变更：
失败关闭条件：
迁移与删除项：
AI 声称与证据（命令输出、测试日志、烟测记录）：
纯逻辑验证：
适配器契约验证：
真实烟测：
安全与权限检查：
人工门禁结果（需求确认/设计评审/发布签收）：
发布命令与结果：
剩余风险：
下一轮唯一入口：
```

## 10. 发布记录

| 迭代 | 日期 | 交付 | 质量门禁 | 运行边界 |
|---|---|---|---|---|
| A-01 | 2026-09-08 | 两所 REST 深度、双向 VWAP、完整保守成本、CLI 单次/持续观察 | 测试、Clippy、真实公共行情烟测 | 只读；费率为配置值 |
| A-02 | 2026-09-08 | 两所动态品种规格、启动准入、逐方向名义金额限制 | 13 项测试、严格 Clippy、真实公共品种与行情烟测 | 只读；无账户能力和下单 |
| A-03 | 2026-09-08 | 两所 WebSocket 增量本地簿、数据质量状态、失效重建、实时扫描 | 21 项测试、严格 Clippy、真实双向扫描与主动重连烟测 | 只读；未验证 24 小时连续运行；无账户能力和下单 |
| A-04 | 2026-09-08 | 两所能力卡、环境变量只读签名客户端、账户费率版本、失效准入和账户检查 CLI | 30 项测试、严格 Clippy、签名 HTTP 夹具、真实公共双向扫描 | 只读；未使用真实账户凭证；无下单能力 |
| A-05（实现与观察启动） | 2026-09-08 | 流式确定性回放、崩溃尾部修复、稳定拒绝类别、静默持续运行、SIGTERM 刷盘 | 38 项测试、严格 Clippy、在线归档回放、优雅停止与独立正式窗口 | 只读；14 天窗口运行中，最早 2026-09-22T09:52:14Z 评审；无下单能力 |
| B-02 | 2026-09-08 | PAPER 订单事实、提交/查单/撤单 `UNKNOWN` 状态机、成交幂等和恢复烟测 | 44 项测试、严格 Clippy、临时 PostgreSQL `cargo test -p personal-taoli-core order::tests -- --nocapture`；拒绝/未知/调查/撤单竞态/去重/恢复均通过 | PAPER/模拟适配器；无真实或测试网订单 |
| B-03（PAPER 已验证） | 2026-09-08 | 双腿执行事实、部分成交差额、预算内补偿计划、敞口/预算超限人工升级、匹配完成和恢复烟测 | 45 项测试、严格 Clippy、临时 PostgreSQL `cargo test -p personal-taoli-core execution::tests -- --nocapture`；五项行为断言通过，`external_order_calls=0` | PAPER/模拟事实层；无真实或测试网订单 |
| C-01（桌面迁移，已验证） | 2026-09-08 | 标准 Tauri 工作区、Vue 桌面控制台、统一 `ApiResponse<T>` command 协议、配置/账户/观测/回放/PAPER 操作；删除 CLI 桌面入口 | `npm run build`、`cargo test --workspace`（45 项通过）、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace --release` 均通过；发布二进制已启动验证，当前环境未提供可观测 GUI/CDP 烟测 | 只读公共行情、只读账户元数据和 PAPER；无真实或测试网订单 |
| C-02（故障注入，进行中） | 2026-09-09 | 故障注入测试框架、多种故障类型支持、测试套件 | `cargo test --workspace`（43 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；故障注入测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| C-03（告警，已验证） | 2026-09-09 | 告警模块、多种告警级别和类型支持、告警管理器 | `cargo test --workspace`（50 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；告警模块测试运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| C-04（备份恢复，已验证） | 2026-09-09 | 备份恢复模块、多种备份类型支持、备份管理器 | `cargo test --workspace`（53 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；备份恢复测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| C-05（RPO/RTO实测，已验证） | 2026-09-09 | RPO/RTO测量模块、目标配置和测量器 | `cargo test --workspace`（59 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；RPO/RTO测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| C-06（主备fencing，已验证） | 2026-09-09 | 主备fencing模块、节点管理和令牌管理 | `cargo test --workspace`（68 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；主备fencing测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| C-07（运行手册演练，已验证） | 2026-09-09 | 运行手册演练模块、演练场景和步骤管理 | `cargo test --workspace`（76 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；运行手册演练测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| D-01（部署配置，已验证） | 2026-09-09 | Docker Compose配置、systemd配置、健康检查模块 | `cargo test --workspace`（84 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；部署配置验证成功 | 隔离环境测试；无真实或测试网订单副作用 |
| D-02（监控指标，已验证） | 2026-09-09 | 监控指标模块、系统/交易/市场/风险指标 | `cargo test --workspace`（94 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；监控指标模块测试成功 | 隔离环境测试；无真实或测试网订单副作用 |
| D-03（告警通知，已验证） | 2026-09-09 | 告警通知模块、多通道通知支持 | `cargo test --workspace`（101 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；告警通知测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| D-04（备份策略，已验证） | 2026-09-09 | 备份策略模块、多种备份策略类型 | `cargo test --workspace`（113 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；备份策略测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| D-05（运行手册，已验证） | 2026-09-09 | 运行手册管理模块、运行手册模板 | `cargo test --workspace`（123 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；运行手册测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| E-01（实时监控仪表盘，released） | 2026-09-09 | 实时监控仪表盘模块、系统/交易/市场/风险状态 | `cargo test --workspace`（130 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；实时监控仪表盘测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| E-02（告警管理，released） | 2026-09-09 | 告警管理模块、告警创建/确认/解决 | `cargo test --workspace`（141 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；告警管理测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| E-03（运维操作，released） | 2026-09-09 | 运维操作模块、操作创建/执行/状态管理 | `cargo test --workspace`（151 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；运维操作测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| E-04（系统健康检查，released） | 2026-09-09 | 系统健康检查模块、健康检查执行/历史记录 | `cargo test --workspace`（159 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；系统健康检查测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| E-05（性能指标收集，released） | 2026-09-09 | 性能指标收集模块、指标收集/分析/统计 | `cargo test --workspace`（169 项通过）、`cargo clippy --workspace --all-targets -- -D warnings`、`npm run build` 均通过；性能指标收集测试套件运行成功 | 隔离环境测试；无真实或测试网订单副作用 |
| UI-01（前端视觉优化，released） | 2026-09-09 | 设计令牌系统与五页面组件视觉面升级、侧栏/SVG 图标/装饰图形重绘、窗口 1360×860；名词项悬浮解释（R11）：`glossary.ts` 约 98 词条 + `TermHint.vue`，覆盖控制台机会卡/账户表头/验证报告/回放重连指标、设置页全部字段、概览/市场面板，`el-tooltip` 视觉悬浮、未收录词条不破版 | `npm run build` 零错误（`vue-tsc --noEmit` + `vite build`）；浏览器 1360×860 与 1100×780 两档五页面 DOM 契约断言（无横向溢出、令牌字号/布局生效、导航/表单输入/按钮状态正常）；hover「预期净收益」弹出「毛利扣除手续费与风险缓冲后的估算净利润」（2026-09-09 复核） | AIDLC 段外特批（手册 §11 之外）；仅前端展示层，无后端/协议/数据变更 |
| F-01（多币种观察支持，已验证） | 2026-09-09 | 多币种并行观察：核心层 `PairConfig`+`ObserverConfig.pairs`+`effective_pairs()` 唯一真相源及旧格式迁移、`observe_once`/`observe_continuously`/`run_reconnect_smoke` 逐对循环；命令层 `PairSummary`/`ConfigSummary.pairs`/`PairReconnectSmokeResult`/`ReconnectSmokeResult`、`observe_once` 与 `account_status` 返回 `Vec`（账户表按 `fee.symbol` 币种列区分）；前端配置/账户/观测/重连/SettingsPage 全链路 pairs 化、设置页交易对增删 | `cargo fmt --all -- --check`、`cargo test --workspace`（173 项 core + 4 项 tauri-lib 通过）、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`npm run build` 零错误；浏览器 mock IPC 渲染层实测：概览 ConfigStrip 三币对（BTCUSDT+2 个币种/深度 20）、账户表币种列 BTC/ETH、扫描报告「共 2 对」、重连烟测 results 展平 4 venue、设置页添加/删除币种响应 | 只读公共行情与账户元数据；PAPER/模拟事实层无真实或测试网订单（`external_order_calls=0`）；A-05 14 天窗口继续运行至 2026-09-22T09:52:14Z 评审；released 待所有者签收 |
| F-02（模拟套利，已验证） | 2026-09-09 | 仿真执行闭环：核心层 `SimulationConfig`（全字段 `#[serde(default)]`+`validate()`）与 `simulation.rs` 撮合引擎（逐档消费、adverse 偏移成交价、深度不足部分成交、敌手占盘、资金不足拒绝、UNKNOWN 查询恢复、幂等重放、补偿决策、每方向独立 PAPER 账户、`SimRng` 可复现采样）；`observer.rs` 连续观测接入（`SimulationEngine` 单槽队列+独立 task，积压丢弃计数）；命令层 `run_simulation_smoke_command`/`SimulationSmokeResult`（缺 `TAOLI_DATABASE_URL` 失败关闭）；前端控制台「模拟套利」入口与 S01–S09 探针报告网格；全部事实经 PostgreSQL 落库（复用 `PaperCore`/`OrderCore`/`ExecutionCore`），领域锁分段串行（acquire→操作→disconnect） | `cargo fmt --all -- --check`、`cargo test --workspace`（182 项 core + 5 项 tauri-lib 通过）、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo build --workspace --release` 通过、`npm run build` 零错误；PostgreSQL 烟测 S01–S09 全绿且 `external_order_calls=0`；浏览器 mock IPC 渲染实测：控制台「模拟套利」点击后标题切「F-02 模拟套利验证」，九项探针全「通过」+「外部订单调用次数 0」+「只读烟测 · 未产生任何真实/测试网订单」 | PAPER/模拟事实层：真实行情 → 决策 → 模拟撮合 → PAPER 事实落库 → 仿真报告闭环，全程无真实或测试网订单（`external_order_calls=0`）；仅在 `ObserverConfig.simulation.enabled=true` 时接入连续观测，默认关闭；A-05 14 天窗口继续运行至 2026-09-22T09:52:14Z 评审；released 待所有者签收 |

| G-01（模拟套利仪表盘，已验证） | 2026-09-09 | F-02 之上的独立「模拟套利」页面与只读查询层：核心层 `simulation_query.rs`（独立连接 `BEGIN READ ONLY` 只读事务经 `open()`，不经领域单写者锁；`get_simulation_overview`/`get_simulation_runs`/`get_simulation_run_detail` 三命令；概览聚合、20 条最近、分页过滤（limit≤200）、单 run 详情（report 全文+意图/成交/执行事件/审计/余额）、按日山脊桶+稀疏合并、`f02-` 前缀活动流、两账户现值）；迁移 0005 `simulation_runs` 投影表（`SCHEMA_VERSION` 4→5、关键列索引、不可变触发器、`external_order_calls=0` CHECK）；引擎两处无害钩子（run 报告落库 bail 关闭 + 三节点余额快照 `source='SIMULATION'` 降级告警）；前端 `SimulationPage.vue` 容器与八板块（StatCards/ProfitChart/BalanceChart/Flow/Ridge/Trades/Activity）+ App.vue 挂载与 10s 轮询 | `cargo fmt --all -- --check`、`cargo test --workspace`（190 项 core + 8 项 tauri-lib 通过）、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo build --workspace --release` 通过、`npm run build` 零错误；PostgreSQL 真数据断言（16→20 run、96→120 SIMULATION 余额快照、snapshot_id 8 段、overview/flow/ridge/activity 与 DB 直查一致、detail 审计事件经 LIKE 通配符修复后返回非零） | PAPER/模拟事实层只读展示：查询无写路径、无新增真实/测试网订单路径、`external_order_calls=0` 恒真；仅在 `ObserverConfig.simulation.enabled=true` 时连续观测接入，默认关闭；A-05 14 天窗口继续运行至 2026-09-22T09:52:14Z 评审；released 待所有者签收；剩余风险：PAPER 预留释放缺失（运维另立项）、投影表行增长（索引+分页缓解）、山脊稀疏合并近似 |

| DB-01（sqlx 池化重构，已验证） | 2026-09-10 | 数据库连接层由 tokio-postgres 即连即断重构为 sqlx `PgPool` 池化：`db.rs` 全量重写（`pool()` 按 URL 缓存池 + `max_connections=16` + `acquire_timeout=30s` + `after_release` 解锁兜底；`migrate_pool` 单会话 advisory 锁迁移；`verify_schema<E: PgExecutor>` 泛型校验；`DomainConnection` 单写者锁 acquire/disconnect）；八个模块（paper/order/execution/accounting/control/reconciliation/simulation/simulation_query）93 个查询点全量迁移（`pool.begin()` 事务、`try_get` 解码、`rows_affected` 计行）；G-01 只读查询层改事务形态（`SET TRANSACTION READ ONLY` + 全部查询 + 回滚，任一失败即 aborted 上抛，不复用事务）；`tokio-postgres` 全仓移除 | `cargo fmt --all -- --check`、`cargo test --workspace`（190 项 core + 8 项 tauri-lib 通过）、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo build --workspace --release` 通过；真实 PostgreSQL 烟测（55432）七域 smoke + G-01 三只读入口全绿且 `external_order_calls=0`；U01–U10 逐条断言（池上限、迁移幂等、版本校验文案、领域锁释放/拒绝文案、只读拒写不污染池、缺 URL 降级不阻断观察循环） | PAPER/模拟事实层；无真实或测试网订单（`external_order_calls=0`）；遗留风险：`MAX_CONNECTIONS=16` 在 F-02 高并发下不足时操作 30s 后失败关闭、只读事务内语句失败即 aborted、`try_get` 把旧 panic 收敛为 `Err`（均见设计 §8，行为变化为期望变更） |
| PERF-01（读路径性能与重复代码收敛，已验证） | 2026-09-10 | 承接 DB-01 池化后的读路径收口：`db.rs` 新增 `ready_pool(url)` + `READY` 守卫（迁移与 schema 校验按 URL **每进程一次**，`migrate` 委托之），`simulation_query`/`paper`/`simulation` 六个入口统一走该入口；新增 `migrations/0006_read_path_indexes.sql`（5 个 `CREATE INDEX IF NOT EXISTS`，`text_pattern_ops` 服务 `LIKE 'f02-%'`），`SCHEMA_VERSION` 5→6；`simulation_query.rs` 累计曲线改窗口函数前缀和 + `CUMULATIVE_LIMIT=1200`、山脊加 `RIDGE_LIMIT=4000`、流转聚合 6→2 条查询（`filled_runs` 修正为真实双腿成交）、详情同表 3 次往返→1 次（新增 `RUN_PROJECTION`/`run_row_at` 共用列清单）；`local_book` 的 `snapshot(1)?` → `validate_live()`（每行情事件省 2 String + 2 Vec 分配）、`market` 解析函数按值泛型化并删除 4 个 venue 文件的 6 份副本、`LevelUpdate` 双定义收敛；`SimulationEngine` 补 `Drop`（覆盖观察循环 `?` 早退的任务泄漏）；`config.rs` 旧格式迁移改 `Cow` 分流；前端合约过滤选项改由 `getObserverConfig().pairs` 派生（原硬编码 `BTC/USDT` 与库中 `BTCUSDT` 不匹配，恒返回空）、累计曲线消费后端前缀和、山脊改直方图+核卷积并消除展开运算符求极值 | `cargo fmt --all -- --check` PASS；`cargo test --workspace`（**192** core + 8 tauri-lib 通过，基线 190+8，净增 2 条本地簿不变量用例）；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo build --workspace --release` 成功（2m01s）；`npm run build` 零错误；真实 PostgreSQL（55432）驱动实测：`cold_overview_ms=191` → `warm_overview_ms=42`；`schema_migrations` 二次调用后仍 6 行；`EXPLAIN` 前后由 Seq Scan 变 Index Scan；`cumulative_points=564` 且 `monotonic_prefix_sums=true`；`ridge_points=488`；`flow.filled_runs=403` ≠ `completed_runs=373`；`filter_option_BTCUSDT_total=564` vs `slash_form_total=0` | 只读；**无新增写路径**，`external_order_calls=0` 恒真；A-05 14 天窗口不受影响；GUI 渲染未做像素级验证（无宿主），详见任务文档 §2.3 |
| MIG-01（迁移机制交由 sqlx 接管，已验证） | 2026-09-10 | 迁移的来源/排序/记账/锁/事务/校验整体交由 sqlx 0.8.6 `Migrator`：`db.rs` 以 `static MIGRATOR = sqlx::migrate!("./migrations")` 编译期嵌入目录，删除 6 个 `include_str!` 常量、逐文件标签数组、自建 `advisory_key("schema-migration",…)` 迁移锁、`SCHEMA_VERSION` 常量与 `verify_schema` 函数；版本号改由文件集最大版本推导（`schema_version()`，`paper.rs`/`simulation.rs` 4 处调用点同改）；记账表由 `schema_migrations(version, applied_at_ms)` 换成 sqlx 的 `_sqlx_migrations(version, description, installed_on, success, checksum, execution_time)`，0001–0006 尾部逐文件 INSERT 剥离（0001 另删旧表 CREATE，否则 0007 删表后重跑会重新建表），新增 `0007_drop_legacy_schema_migrations.sql`；存量库由 `adopt_legacy_bookkeeping` 一次性采纳——在 `MIGRATOR.run` **之前**逐个校验旧表版本是否属于嵌入文件集（保留被删 `verify_schema` 的安全性质，避免旧二进制在新库上先应用自己的 0007），再把 `applied_at_ms` 换算进 `installed_on` 并写入嵌入文件的 SHA-384，`execution_time=-1` 标记采纳行；`DomainConnection::acquire` 改走 `ready_pool` 使连接与 schema 就绪合一；sqlx feature 增 `migrate`+`macros` | `cargo fmt --all -- --check` PASS；`cargo test --workspace`（192 core + 8 tauri-lib 通过，无新增测试——基础设施替换的可观察性质无法在无 DB 纯逻辑测试中验证）；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo build --workspace --release` 2m15s；`npm run build` 零错误；真实 PostgreSQL（55432）六条路径实测：全新库 1..7 全 `success` 且与 v4 升级结果收敛（22 表/47 索引/11 触发器）、冷 338ms→29ms；采纳保真 `1700000000000+v*1000` → `2023-11-14 22:13:2v+00` 逐行精确；v4 库 1–4 采纳 + 5–7 应用；篡改 checksum → `previously applied but has been modified`；旧表含 99 → `expected at most 7, found 99` 且零副作用；存量库 1–7 全 `success`（1–6 采纳、7 真实应用）、业务表行数只增不减 | 只读语义与 `external_order_calls=0` 不变；**无业务表结构变更**（仅迁移记账机制与旧表删除）；A-05 14 天窗口不受影响；**替换了 4 条已发布需求的断言文本**（G01-R10/DB01-R02/DB01-R03/PERF01-R01/R02，已逐条同步，可观察结论不变）；执行顺序偏差与流程教训见 §6.7 |
| G-02（模拟 run 来源判别，已验证） | 2026-09-10 | 修真实使用缺陷：仪表盘把合成簿烟测探针与实时市场仿真混同呈现。新增迁移 `0008_simulation_run_source.sql`（`source TEXT NOT NULL DEFAULT 'LIVE' CHECK (source IN ('SMOKE','LIVE'))` + `(source, executed_at_ms DESC)` 索引）；回填**不用 `UPDATE`**——`0005` 的 `simulation_runs_immutable` 会拒绝，改用 `ADD COLUMN … DEFAULT 'SMOKE'` 再 `SET DEFAULT 'LIVE'`，从而全程不触碰不可变触发器（实测触发器仍拒绝 UPDATE）。核心层新增 `SimulationRunSource{Smoke,Live}`，作为 `run_simulation_run` 的**显式形参**（7 个烟测调用点传 Smoke、实时路径传 Live，编译器强制不漏），随报告持久化；`SimulationRunRow.source` 与 `SimulationOverview.{smoke_runs,live_runs}` 经查询层透出；`run_simulation_smoke` 首行打印 `SYNTHETIC FIXTURES … not market data` 横幅。前端：统计卡顶部新增来源提示（全探针/全实时/混合三分支，全探针态为警示配色并解释「实时 run 0 条 —— 准入未通过」）、机会历史列表与详情页来源徽章、烟测按钮改名「运行合成簿烟测」并附合成属性说明 | `cargo fmt --all -- --check` PASS；`cargo test --workspace`（192 core + 8 tauri-lib，无新增测试——标注类改动的可观察结果需真实 DB 与真实渲染）；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo build --workspace --release` 2m03s；`npm run build` 零错误；实库实测：迁移落库 `success=t`、触发器完好且仍拒绝 UPDATE、列默认 `LIVE`；实跑烟测 `613/613/0` → `620/620/0` 且 `newest source=SMOKE`；8 个配置币对过滤值域正确；mock IPC 渲染断言命中全部文案，视觉确认警示配色 | 只读语义与 `external_order_calls=0` 不变；无业务表结构变更以外的语义变更；**验证缺口**：`LIVE` 分支无真实数据可触发（实时 run 为 0，阻塞于准入前置条件，属阶段 F）；未改烟测 symbol、未接管全局聚焦币种 |

## 11. 下一轮唯一入口
阶段E（监控与运维）已全部完成。下一步是进入阶段F（生产准备），或等待所有者批准进入生产环境。
UI-01（前端视觉优化）为段外特批迭代，已于 2026-09-09 发布（所有者签收，见第 10 章发布记录）；其完成不改变本入口：下一轮商业迭代仍唯一进入阶段F（生产准备）或经所有者批准进入生产环境。
F-01（多币种观察支持）已于 2026-09-09 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收；其完成不改变本入口：阶段 F 首个迭代落地后，下一轮唯一入口为 F 阶段后续迭代（如 F-02 真实接入评估 / F-03 生产部署，按本手册 §11 立项）或经所有者批准进入生产环境。
F-02（模拟套利）已于 2026-09-09 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收：真实行情 → 决策 → 模拟撮合 → PAPER 事实落库 → 仿真报告闭环，全程无真实或测试网订单（`external_order_calls=0`）；其完成不改变本入口：下一轮唯一进入 F 阶段后续迭代（如 F-03 生产部署）或经所有者批准进入生产环境。
G-01（模拟套利仪表盘）已于 2026-09-09 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收：F-02 之上的只读可视化页面（八板块 + 每 run 两账户详情/变化图），查询层零写路径、`external_order_calls=0` 恒真；其完成不改变本入口：F-03 生产部署仍为保留编号，G 阶段后续唯一入口为「G-02 后续可视化/运维」（按本手册 §11 立项），或经所有者批准进入生产环境。
DB-01（数据库连接层 sqlx 池化重构）已于 2026-09-10 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收：连接层由 tokio-postgres 即连即断改为 sqlx `PgPool` 池化（16 连接上限、迁移幂等、领域单写者锁、只读查询层零写路径不变），`external_order_calls=0` 恒真；其完成不改变本入口：下一轮唯一进入 F 阶段后续迭代（如 F-02 真实接入评估 / F-03 生产部署）或经所有者批准进入生产环境。

PERF-01（读路径性能与重复代码收敛）已于 2026-09-10 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收：承接 DB-01 池化后的读路径，就绪库单点（迁移每进程一次）、读路径索引（0006 + `SCHEMA_VERSION` 6）、有界读取（曲线/山脊窗口）、流转聚合修正（`filled_runs` 反映真实双腿成交）与重复代码收敛（解析 6→1 份、`LevelUpdate` 双定义合一）；全程只读、`external_order_calls=0` 恒真，**不解除也不改变 A-05 连续观察门槛**。其完成不改变本入口：下一轮商业迭代仍唯一进入 F 阶段后续迭代（如 F-03 生产部署）或经所有者批准进入生产环境。另需所有者决策的**独立立项项**已记录于设计文档 §7 与任务文档 §4：16 个无生产调用点模块（6304 行）的接线或删除——**接线前必须先为其中 4 个模块的无界容器补淘汰策略**；以及展示层取舍（`DetailPanel` 拆分、概览页静态装饰组件与 `RidgePanel`/`SimulationRidge` 双实现）。

MIG-01（迁移机制交由 sqlx 接管）已于 2026-09-10 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收：迁移的来源、排序、记账、锁、事务与校验整体交由 sqlx `Migrator`（`_sqlx_migrations` + SHA-384 校验和 + 半应用检测），删除自建迁移器（文件清单、迁移锁、`SCHEMA_VERSION` 常量、`verify_schema`、`schema_migrations` 表）；存量库经一次性采纳引导，不重跑任何已应用文件。**本迭代改写了 4 条已发布需求的断言文本**（迁移记账表由 `schema_migrations` 变为 `_sqlx_migrations`），已在 §6.7 与各需求文档中逐条登记，可观察结论不变。其完成不改变本入口：下一轮商业迭代仍唯一进入 F 阶段后续迭代（如 F-03 生产部署）或经所有者批准进入生产环境。待所有者决策的独立立项项（16 个无生产调用点模块的接线或删除、展示层取舍）仍见 PERF-01 设计文档 §7 与任务文档 §4；另有 MIG-01 记录的两项运维待补：`Dirty` 后的人工处置动作入 runbook、备份时序规范（停应用→备份→迁移）。

G-02（模拟 run 来源判别）已于 2026-09-10 实现并验证完成（`verified`，见第 10 章发布记录），released 待所有者签收：修复「仪表盘把合成簿烟测探针当成实时市场模拟展示」这一真实使用缺陷——新增 `source` 判别列（`SMOKE`/`LIVE`）、概览透出来源构成、界面三处来源标注与烟测自我说明。**同时确认了一条关键事实**：实时模拟 run 数为 0，原因是准入按设计失败关闭（凭证未配置、Bybit 资格未确认、实际费率 observation-only、名义金额低于交易所下限），8000 条 decision 中 `accepted` 为 0——这不是缺陷，而是 AIDLC §2 安全边界的预期表现；在任何模拟套利数据变得有意义之前，必须先完成阶段 F 的凭证与资格前置。其完成不改变本入口：下一轮商业迭代仍唯一进入 F 阶段后续迭代（如 F-03 生产部署）或经所有者批准进入生产环境。
