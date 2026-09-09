# B-02 订单事实与 `UNKNOWN` 状态机：设计文档

文档状态：`released`。实现仅覆盖 PAPER/模拟适配器，不连接真实或测试网订单接口。
## 1. 设计结论

B-02 在 B-01 `order_intents` 和 `NOT_SENT` 边界上增加独立订单事实层。数据库是状态机的唯一事实源；模拟适配器只返回可注入的提交、查询、撤单和成交事件，不访问网络。提交前先写 `IN_FLIGHT`，响应丢失只能写 `UNKNOWN`。

## 2. 模块边界

- `crates/core/src/order.rs`：状态枚举、证据校验、PostgreSQL 事实写入、调查队列和确定性 PAPER 烟测。
- `crates/core/migrations/0002_order_facts.sql`：订单事实、调查尝试、成交事实与状态约束。
- `src-tauri/src/commands/paper.rs`：`run_paper_smoke("B02")` 运维入口；在创建 HTTP 客户端前返回。
- `crates/core/src/paper.rs`：保持 B-01 计划与资金预留语义；B-02 不把恢复等同于可发送。

## 3. 数据模型

`order_facts` 以 `intent_id` 为主键，保存当前提交/撤单/对账状态、交易所 `order_id`、最近事实版本和最后错误。`order_action_facts` 保存每次动作的请求摘要、动作类型、结果证据和时间。`trade_facts` 以 `(venue, order_id, trade_id)` 唯一，保存成交数量、价格、费用和来源序号。`order_investigations` 记录 UNKNOWN 的查询与可见性窗口。

所有金额和数量使用 PostgreSQL `NUMERIC` 与 Rust `Decimal`。成交插入和累计量更新必须在同一事务内；重复成交只增加调查/审计事实，不增加累计经济数量。

## 4. 状态转换

```text
NOT_SENT --submit_started--> IN_FLIGHT
IN_FLIGHT --accepted(order_id)--> ACCEPTED
IN_FLIGHT --definitely_rejected--> DEFINITELY_REJECTED
IN_FLIGHT --timeout/transport_lost--> UNKNOWN
UNKNOWN --query_not_found--> UNKNOWN
UNKNOWN --query_found(order_id)--> ACCEPTED
ACCEPTED --cancel_requested--> ACCEPTED + cancel=REQUESTED
ACCEPTED --cancel_unknown--> ACCEPTED + cancel=UNKNOWN
ACCEPTED --cancel_confirmed--> ACCEPTED + cancel=CONFIRMED
```

`UNKNOWN` 不允许 `submit`；只有调查结果 `definitely_not_created` 才能由上层决定新 attempt，且必须产生新的 `attempt_id`。成交事件独立于撤单状态，任何状态都不得删除已记录成交。

## 5. 事务顺序

1. 锁定意图行并检查当前状态和版本。
2. 插入不可变动作事实，更新当前状态；提交前不调用适配器。
3. 模拟适配器返回结果后，在新事务中按证据执行状态转换。
4. 成交事件按唯一键插入；仅首次插入更新累计成交量和对账状态。
5. UNKNOWN 查询暂未找到只写调查事实，保留资金预留。
6. 重启先恢复 UNKNOWN/未终态列表，禁止自动提交。

真实适配器接入前必须另行实现调用前后 fencing、限频、私有流恢复和权限审查；本轮不做。

## 6. 失败关闭

状态证据与当前状态不匹配、同一 `order_id` 绑定不同意图、成交字段变化、累计成交倒退或数据库错误，均进入 `CONFLICT`/人工调查，不静默覆盖。任何 SQL 失败都不使用内存回退。

## 7. 验收映射

| 需求 | 实现 | 验证 |
|---|---|---|
| B02-R01–R03 | 提交事实与 UNKNOWN 转换 | 模拟响应成功/拒绝/超时 |
| B02-R04–R05 | 调查队列与查询证据 | 暂未找到后确认原订单 |
| B02-R06–R07 | 撤单事实和成交唯一键 | 撤单竞态、重复/乱序成交 |
| B02-R08 | 恢复查询 | 断连后 UNKNOWN、成交和预留仍在 |

## 8. 安全边界

`run_paper_smoke("B02")` 需要 `TAOLI_DATABASE_URL` 指向临时 PostgreSQL，只使用内置模拟适配器，输出数据库事实和 `external_order_calls=0`。不输出数据库 URL、密钥或原始认证信息，不改变 A-05 运行门禁。
