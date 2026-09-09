# C-05 RPO/RTO实测：需求文档

文档状态：`in_progress`。本阶段实现RPO/RTO测量能力。

## 1. 目标

建立可重复的RPO/RTO测量能力，在隔离环境验证系统的恢复点目标和恢复时间目标。

## 2. 需求

| 编号 | 要求 | 验收结果 |
|---|---|---|
| C05-R01 | 支持RPO/RTO目标配置 | 目标配置功能完整 |
| C05-R02 | 支持RPO/RTO测量 | 测量功能实现完整 |
| C05-R03 | 测量结果可追溯 | 生成结构化测量报告 |
| C05-R04 | 支持RPO/RTO达标验证 | 达标验证功能完整 |
| C05-R05 | 支持多轮测量 | 测量历史可查询 |
| C05-R06 | 测量结果可导出 | 支持JSON格式导出 |

## 3. 明确不做

- 不接入真实数据库进行测量。
- 不修改生产环境配置。
- 不执行真实RPO/RTO测量（仅模拟）。
- 不影响系统正常运行。

## 4. 验证边界

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm run build
cargo build --workspace --release
```

RPO/RTO测试套件必须覆盖所有预定义测试场景，每个测试必须有明确的预期行为和结果验证。
