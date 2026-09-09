# A-03 WebSocket 本地订单簿：任务文档

## 1. 工作项

- 迭代编号：A-03
- 当前状态：`released`
- 发布日期：2026-09-08
- 目标：以 Binance/Bybit 公共增量流建立失败关闭、自动恢复的统一本地订单簿
- 上游依赖：A-02 品种规格与净收益扫描已发布
- 下游交付：A-04 账户实际费率和 A-05 可回放影子观察均消费本轮本地簿
- 安全边界：无凭证、无订单、无资金副作用

## 2. 关联文档

- 需求：[A-03 WebSocket 本地订单簿：需求文档](A-03-WebSocket本地订单簿-需求.md)
- 设计：[A-03 WebSocket 本地订单簿：设计文档](A-03-WebSocket本地订单簿-设计.md)
- 总体架构：[个人加密货币套利系统：架构与需求设计](个人加密货币套利系统-架构与需求设计.md)
- 流程与追踪：[个人加密货币套利系统：SDLC 迭代执行手册](个人加密货币套利系统-SDLC迭代执行手册.md)

## 3. 任务清单

状态只使用 SDLC 规定的 `proposed`、`ready`、`in_progress`、`blocked`、`verified`、`released`、`rejected`。

| 阶段 | 任务 | 关联需求 | 产出或证据 | 状态 |
|---|---|---|---|---|
| 需求 | 定义两所增量簿、状态、失败关闭和恢复行为 | A03-R01、A03-R02、A03-R03、A03-R04、A03-R05、A03-R06、A03-R07 | A-03 需求文档 | released |
| 分析 | 核对 Binance 快照衔接与 `U/u` 连续性 | A03-R01、A03-R02 | 官方协议；缓冲/衔接边界 | released |
| 分析 | 核对 Bybit snapshot/delta 与 `seq` 语义 | A03-R03 | 官方协议；重启 snapshot 和旧序列边界 | released |
| 设计 | 建立场所无关的绝对数量本地簿 | A03-R01、A03-R02、A03-R03 | `LocalOrderBook`、`LevelUpdate` | released |
| 设计 | 建立统一状态和单写者发布模型 | A03-R04、A03-R05 | `BookState`、`FeedPublisher`、`BookFeed` | released |
| 实现 | 接入 Binance WebSocket 缓冲和 REST 快照衔接 | A03-R01、A03-R02 | `crates/core/src/venues/binance_stream.rs` | released |
| 实现 | 接入 Bybit snapshot/delta、heartbeat 和跨序列过滤 | A03-R03、A03-R05 | `crates/core/src/venues/bybit_stream.rs` | released |
| 实现 | 接入静默检测、错误撤下快照和自动重连 | A03-R04、A03-R05 | 两所 `subscribe` 重连循环 | released |
| 实现 | 接入双方有效启动门禁、单次扫描和主动重连烟测 | A03-R05、A03-R06、A03-R07 | `wait_for_valid_pair`、`run_reconnect_smoke`、`scan_current` | released |
| 验证 | 覆盖快照衔接、断档、乱序、绝对档位和状态边界 | A03-R01、A03-R02、A03-R03、A03-R04、A03-R05 | 发布时 `cargo test --workspace` 21 项通过 | released |
| 验证 | 执行格式和严格静态检查 | A03-R01、A03-R02、A03-R03、A03-R04、A03-R05、A03-R06、A03-R07 | `cargo fmt --all -- --check`；Clippy 零警告 | released |
| 验证 | 执行真实双所公共行情单次扫描 | A03-R06、A03-R07 | BTCUSDT 双向结果；接收偏差 5 ms | released |
| 验证 | 执行真实双所主动重连恢复 | A03-R05 | 两所 generation 1→2、`reconnects=1`、恢复 `VALID` | released |
| 发布 | 更新架构状态、追踪矩阵与剩余风险 | A03-R01、A03-R02、A03-R03、A03-R04、A03-R05、A03-R06、A03-R07 | 总文档 A-03 发布记录 | released |

## 4. 发布时验证顺序

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p personal-taoli-observer --release -- --once --output json
cargo run -p personal-taoli-observer --release -- --reconnect-smoke
```

判定规则：

1. 格式、测试和严格 Clippy 全部成功。
2. `--once` 必须先等两所 `VALID`，然后输出两个方向及新鲜度/接收偏差相关结果。
3. 市场完整成本后无正机会时 `REJECT` 是正确结果，不得修改门槛制造通过。
4. `--reconnect-smoke` 必须看到两所 generation 均增加并重新 `VALID`；只看到连接断开不算恢复成功。
5. 整个验证无需 API key，且程序不存在订单入口。

## 5. 发布证据

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：21 项通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，无警告。
- 真实 `--once --output json`：Binance/Bybit BTCUSDT 本地簿同步成功，接收时间偏差 5 ms，两向因完整成本后净收益为负而拒绝。
- 真实 `--reconnect-smoke`：Binance 与 Bybit 均从 generation 1 进入 generation 2，`reconnects=1`，自动重建并恢复 `VALID`。

## 6. 剩余风险与后续边界

- 24 小时连接生命周期、自然断线频率、宿主机休眠和长期连续性未由 A-03 发布烟测证明；A-05 负责持续观察。
- A-03 不读取真实账户费率，发布时经济结果仍依赖此前配置费率；A-04 负责账户级成本。
- A-03 的恢复仅指公共行情簿重建，不等于未来订单状态恢复、成交对账或资金补偿。
- 任何新增交易所必须实现自己的协议适配器，不能复用 Binance 或 Bybit 序列假设。

## 7. 完成定义

A-03 可保持 `released`，当且仅当：

- A03-R01 至 A03-R07 的行为继续成立；
- 任何非 `VALID` 状态不暴露旧快照；
- 双所扫描仍只消费双方同时有效的数据；
- 真实公共行情启动和主动恢复路径可运行；
- 文档明确区分公共行情恢复、长期稳定性和未来交易恢复；
- 系统仍无订单能力。
