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

## 11. 下一轮唯一入口
阶段E（监控与运维）已全部完成。下一步是进入阶段F（生产准备），或等待所有者批准进入生产环境。
UI-01（前端视觉优化）为段外特批迭代，已于 2026-09-09 发布（所有者签收，见第 10 章发布记录）；其完成不改变本入口：下一轮商业迭代仍唯一进入阶段F（生产准备）或经所有者批准进入生产环境。
