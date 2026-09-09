# B-02 订单事实与 `UNKNOWN` 状态机：需求文档

- 迭代编号：B-02
- 状态：`released`
- 上游：B-01 PAPER 计划、资金预留与 `NOT_SENT` 意图；A-05 连续观察仍独立延期
- 运行边界：仅 PAPER/模拟适配器；不得连接真实或测试网订单接口

## 2. 目标与可观察结果

系统必须把每条订单意图的提交事实、查询事实、撤单事实、私有成交事件和补拉成交事实持久化，并用显式状态机处理不确定响应。请求超时、连接断开或响应丢失不得推断为拒绝或成交；只能进入 `UNKNOWN`，保留资金预留并要求调查。

## 3. 状态与不变量

- 提交状态：`NOT_SENT | IN_FLIGHT | ACCEPTED | DEFINITELY_REJECTED | UNKNOWN`
- 撤单状态：`NONE | REQUESTED | UNKNOWN | CONFIRMED`
- 对账状态：`PENDING | MATCHED | CONFLICT`
- 成交事件以 `(venue, order_id, trade_id)` 幂等；重复事件不重复计入经济成交。
- 终态拒绝只能由明确拒绝事实产生；`UNKNOWN` 不能直接重发。
- 已知成交不会因旧事件或乱序事件回退；撤单后到达的成交仍必须记录。
- 所有外部副作用前，订单意图和请求摘要已经持久化；本轮模拟适配器调用次数必须可观察且无真实网络。

## 4. 验收场景

| 编号 | 场景 | 结果 |
|---|---|---|
| B02-R01 | 提交成功响应 | `NOT_SENT → IN_FLIGHT → ACCEPTED`，保存交易所订单 ID |
| B02-R02 | 提交明确拒绝 | `IN_FLIGHT → DEFINITELY_REJECTED`，保留事实，不生成成交 |
| B02-R03 | 提交超时/响应丢失 | `IN_FLIGHT → UNKNOWN`，保留预留，不重发 |
| B02-R04 | UNKNOWN 查询暂未找到 | 继续 `UNKNOWN`，记录调查尝试与可见性窗口 |
| B02-R05 | UNKNOWN 查询确认原订单 | 恢复 `ACCEPTED`，绑定唯一交易所订单 ID |
| B02-R06 | 撤单请求后仍成交 | 撤单事实与成交均保留，成交数量进入最终事实 |
| B02-R07 | 私有成交重复/乱序 | 经济成交只计一次，旧状态不覆盖新事实 |
| B02-R08 | 重启恢复 | 未终态订单、调查队列、成交和预留可恢复，不自动重发 |

## 5. 明确不做

- 不连接 Binance、Bybit 或任何真实/测试网私有 API。
- 不实现真实账务、补偿交易、自动释放资金或双腿净敞口处置；这些属于 B-03/B-04。
- 不把 UNKNOWN 查询“暂未找到”解释为拒绝。
- 不在数据库不可写时退化到内存状态。

## 6. 关闭条件

数据库不可写、状态转换证据不足、订单事实冲突或成交摘要不一致时，停止该订单自动动作并保留人工调查状态。任何无法证明“未产生副作用”的提交超时都不得重试提交。

## 7. 验收命令

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
TAOLI_DATABASE_URL=postgresql://... cargo test -p personal-taoli-core order::tests -- --nocapture
```

烟测必须证明提交明确拒绝、超时进入 UNKNOWN、暂未找到保持 UNKNOWN、后续确认恢复、重复成交去重、撤单竞态保留成交、重启恢复以及模拟适配器调用可计数；外部订单调用固定为 0。
