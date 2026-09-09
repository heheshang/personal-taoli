# A-03 WebSocket 本地订单簿：需求文档

## 1. 文档状态

- 迭代编号：A-03
- 状态：`released`
- 发布日期：2026-09-08
- 目标：以 Binance 与 Bybit 公共 WebSocket 增量流维护可用于双向套利观察的本地现货订单簿
- 上游依赖：A-02 已提供品种规格、深度扫描和净收益准入
- 下游依赖：A-04 账户实际费率、A-05 行情归档与连续影子统计

## 2. 范围

### 2.1 本轮必须实现

1. Binance 按官方快照与增量事件衔接规则建立本地簿。
2. Bybit 按 snapshot/delta 语义维护本地簿，并拒绝旧跨序列消息回滚状态。
3. 用统一状态模型表达同步、有效、静默过期和协议失效。
4. 任一行情非 `VALID` 时撤下快照并停止双所扫描。
5. 连接关闭、静默、协议错误、序列错误或主动重连后自动建立新代次。
6. 单次和持续模式只消费双方同时有效的公共实时簿。
7. 提供不需要 API key、不会触发订单或资金副作用的真实烟测入口。

### 2.2 明确不做

- 不读取账户权限、余额或实际费率；这些属于 A-04。
- 不归档行情、决策或故障事件；这些属于 A-05。
- 不提交、查询或撤销订单。
- 不承诺跨连接保留旧簿；每次重连必须清空并重建。
- 不把 REST 深度轮询保留为并行扫描数据源；Binance REST 深度仅用于启动和重建时的快照衔接。
- 不以一次主动重连烟测证明 24 小时连续稳定性、盈利能力或交易恢复能力。

## 3. 功能需求

| 编号 | 用户可观察结果 | 验收方法 | 状态 |
|---|---|---|---|
| A03-R01 | Binance 只有在缓冲事件覆盖 `lastUpdateId + 1` 后才发布本地簿，快照已覆盖的旧事件被丢弃 | 快照覆盖、后继事件衔接和首事件断档用例 | released |
| A03-R02 | Binance 后续事件若越过本地序列下一值，当前簿立即失效；旧事件不会回滚序列 | 后继断档和旧事件边界用例；确认非 `VALID` 无快照 | released |
| A03-R03 | Bybit snapshot 替换当前连接代次的完整本地簿，delta 按绝对数量更新；不递增的 `seq` 被忽略 | 重启 snapshot、绝对更新和旧跨序列用例 | released |
| A03-R04 | 行情状态只使用 `SYNCING`、`VALID`、`STALE`、`INVALID`；仅 `VALID` 携带快照 | 状态转换、撤下快照和重连代次用例 | released |
| A03-R05 | WebSocket 静默、关闭、传输/解析/协议错误、序列断档和主动重连均先失败关闭，再按新代次重建 | 错误路径用例及 `--reconnect-smoke` 真实双所恢复 | released |
| A03-R06 | 启动等待和扫描只在双方同时 `VALID` 时通过；超时或任一侧失效时不产生决策 | `wait_for_valid_pair`、`scan_current` 行为及真实 `--once` 烟测 | released |
| A03-R07 | 整条 A-03 路径只访问公共行情接口，不要求凭证且无订单入口 | CLI 与适配器检查；无凭证真实烟测 | released |

## 4. 状态与数据需求

### 4.1 统一状态

| 状态 | 含义 | 快照 | 扫描资格 |
|---|---|---|---|
| `SYNCING` | 新连接代次正在等待首个可验证本地簿 | 无 | 禁止 |
| `VALID` | 当前代次已按场所协议建立并收到有效更新 | 有 | 仅双方均为该状态时允许 |
| `STALE` | 超过配置静默时间未收到订单簿事件 | 无 | 禁止 |
| `INVALID` | 连接结束、协议/解析错误、序列断档或其他不可继续错误 | 无 | 禁止 |

每次进入新连接尝试时 `generation` 增加；除首次连接外，`reconnects` 增加。任何非 `VALID` 状态必须清空公开快照，禁止消费者沿用旧数据。

### 4.2 本地簿

本地簿必须保存：venue、symbol、买卖档位、当前场所序列、可用时的源时间戳以及本机接收时间戳。档位规则：

- 价格必须大于 0，数量不能为负；
- 数量为 0 删除该价格档；
- 非零数量替换或插入该价格档；
- 对外快照买价降序、卖价升序；
- 请求深度必须大于 0，输出仍须通过订单簿结构校验。

## 5. 场所协议需求

### 5.1 Binance

1. 先建立 `<symbol>@depth@100ms` WebSocket 并缓冲事件。
2. 并发获取 REST 深度快照；继续缓冲快照请求期间的事件。
3. 丢弃 `u <= lastUpdateId` 的已覆盖事件。
4. 第一条应用事件必须满足 `U <= lastUpdateId + 1 <= u`。
5. 后续旧事件 `u <= local_sequence` 忽略；若 `U > local_sequence + 1`，当前代次失效并重建。
6. 应用事件后使用 `u` 作为本地序列。

### 5.2 Bybit

1. 订阅 `orderbook.<depth>.<symbol>` 公共主题。
2. 连接代次内必须先有 snapshot；delta 先到视为错误。
3. snapshot 完整替换本地簿，即使其更新 ID 小于上一连接代次也必须接受。
4. snapshot 和 delta 都以 `seq` 判断跨序列推进；`seq` 不大于已应用值时忽略。
5. delta 数量是价格档绝对数量，不做加减增量。
6. 优先使用 `cts` 作为源时间，缺失时使用事件 `ts`。

## 6. 失败与恢复场景

| 场景 | 必须行为 |
|---|---|
| 首事件前静默 | 发布 `STALE`，无快照，等待重连后新代次同步 |
| 有效运行中静默 | 撤下旧快照并发布 `STALE` |
| WebSocket close/EOF | 发布 `INVALID`，延迟后新代次重连 |
| 文本解析或 symbol/topic 不匹配 | 发布 `INVALID`，不得跳过错误继续使用旧簿 |
| Binance 序列断档 | 发布 `INVALID`，重新建立 WebSocket 和 REST 快照衔接 |
| Bybit delta 先于 snapshot | 发布 `INVALID`，重新订阅并等待 snapshot |
| 旧 Binance `u` 或旧 Bybit `seq` | 忽略，不回滚已发布状态 |
| 人工 `FeedCommand::Reconnect` | 两所各进入更高 generation，清空快照，重建后恢复 `VALID` |
| 启动期限内双方未同时有效 | CLI 失败，不生成扫描结果 |

## 7. 非功能需求

- **正确性优先**：未知或不连续序列必须失败关闭，不能猜测补齐。
- **单一数据源**：扫描只读取 `BookFeedStatus.snapshot`，不维护第二套 REST 轮询簿。
- **确定性状态**：状态、generation、重连数和错误原因必须通过单一发布者更新。
- **有界新鲜度**：WebSocket 静默和决策时盘口年龄分别受显式配置约束。
- **凭证隔离**：A-03 公共行情路径不读取环境变量中的 API key/secret。
- **可观察性**：状态包含 venue、symbol、generation、reconnects、applied_updates 和失败原因。

## 8. 验收命令与解释

发布时执行：

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p personal-taoli-observer --release -- --once --output json
cargo run -p personal-taoli-observer --release -- --reconnect-smoke
```

发布证据：

- 格式检查通过；
- 21 项测试通过；
- Clippy 严格模式无警告；
- 真实 Binance/Bybit BTCUSDT 本地簿同步后输出双向扫描；当次接收偏差 5 ms，完整成本后两向均正确拒绝；
- 主动重连后两所均从 generation 1 进入 generation 2，`reconnects=1`，并恢复 `VALID`。

上述结果只证明发布时公共实时行情闭环和一次主动恢复可运行。长期连续性由 A-05 独立观察验证。

## 9. 发布判定

A03-R01 至 A03-R07 均已实现并在 2026-09-08 验证，A-03 状态为 `released`。后续改动若改变状态语义、序列衔接、重连行为、扫描门禁或公共接口边界，必须重新执行本文件的相关验收场景并同步设计和任务文档。
