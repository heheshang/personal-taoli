# B-02 订单事实与 `UNKNOWN` 状态机：任务文档

- 状态：`released`
- 上游：B-01 `verified_paper_only`
- 安全边界：PAPER/模拟适配器；无真实或测试网订单副作用
- A-05：连续观察延期，不因 B-02 解除阶段 A 或实盘门禁

## 2. 任务清单

| 阶段 | 任务 | 产出 | 状态 |
|---|---|---|---|
| 需求 | 定义订单事实、UNKNOWN、调查和成交去重边界 | 需求文档 | released |
| 设计 | 定义状态转换、schema 和事务顺序 | 设计文档 | released |
| 实现 | 增加订单事实 migration 与 Rust 模块 | `crates/core/migrations/0002_*`, `crates/core/src/order.rs` | released |
| 实现 | 增加无网络 PAPER 订单事实烟测入口 | Tauri `run_paper_smoke("B02")` | released |
| 验证 | 覆盖成功、拒绝、UNKNOWN、调查、撤单竞态、成交幂等和恢复 | Rust 测试、临时 PostgreSQL | released |
| 发布 | 更新架构、SDLC 和边界状态 | 发布记录 | released |

## 3. 必须通过的行为

1. 提交前存在持久化订单意图；提交动作有独立事实。
2. 超时/响应丢失进入 `UNKNOWN`，不能直接重发。
3. 查询暂未找到仍是 `UNKNOWN`；查询确认才绑定原订单。
4. 明确拒绝可进入 `DEFINITELY_REJECTED`，不得伪造成成交。
5. 撤单后的成交仍被记录；重复或乱序成交不重复计入。
6. 重启恢复 UNKNOWN、调查、成交和资金责任，不自动发送。
7. 模拟适配器调用计数可观察，真实外部订单调用为 0。

## 4. 发布阻塞

- 任何真实/测试网订单、撤单、私有流调用或真实凭证要求。
- UNKNOWN 被实现成自动重试或自动拒绝。
- 成交去重、恢复或数据库事实无法通过临时 PostgreSQL 验证。
- B-01 `NOT_SENT`、`LOCAL_RESERVED` 和 A-05 独立观察边界被破坏。
