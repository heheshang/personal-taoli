# C-05 RPO/RTO实测：设计文档

文档状态：`in_progress`。实现RPO/RTO测量模块。

## 1. 模块边界

- `crates/core/src/rpo_rto.rs`：RPO/RTO测量核心模块，包含目标配置、测量器和测试套件。

## 2. RPO/RTO目标

| 目标 | 默认值 | 描述 |
|---|---|---|
| RPO | 300秒（5分钟） | 恢复点目标 |
| RTO | 1800秒（30分钟） | 恢复时间目标 |

## 3. 测量器设计

```text
RpoRtoMeasurer {
    target: RpoRtoTarget,
    measurements: Vec<RpoRtoMeasurement>,
}

impl RpoRtoMeasurer {
    pub fn new(target: RpoRtoTarget) -> Self;
    pub fn start_measurement(&self, now_ms: u64) -> RpoRtoMeasurementContext;
    pub fn complete_measurement(&mut self, context: RpoRtoMeasurementContext, actual_rpo_seconds: u64, now_ms: u64) -> RpoRtoMeasurement;
    pub fn get_measurements(&self) -> &[RpoRtoMeasurement];
    pub fn get_latest_measurement(&self) -> Option<&RpoRtoMeasurement>;
    pub fn all_targets_met(&self) -> bool;
}
```

## 4. 测量结果结构

```text
RpoRtoMeasurement {
    measurement_id: String,
    actual_rpo_seconds: u64,
    actual_rto_seconds: u64,
    rpo_target: u64,
    rto_target: u64,
    rpo_met: bool,
    rto_met: bool,
    started_at_ms: u64,
    completed_at_ms: u64,
    duration_ms: u64,
}
```

## 5. 测试套件设计

```text
RpoRtoTestSuite {
    tests: Vec<RpoRtoTest>,
}

impl RpoRtoTestSuite {
    pub fn new_default() -> Self;
    pub async fn run_all(&self) -> Result<Vec<RpoRtoResult>>;
    async fn run_test(&self, test: &RpoRtoTest) -> Result<RpoRtoResult>;
}
```

## 6. 失败关闭

- 测量必须包含完整的元数据。
- 测量结果必须包含RPO/RTO达标验证。
- 所有测试必须在隔离环境运行，不影响生产环境。

## 7. 接入命令

RPO/RTO测量器可通过以下方式调用：

```text
// 创建测量器
let target = RpoRtoTarget::default();
let mut measurer = RpoRtoMeasurer::new(target);

// 开始测量
let context = measurer.start_measurement(1000);

// 完成测量
let measurement = measurer.complete_measurement(context, 200, 2000);

// 检查是否达标
if measurement.rpo_met && measurement.rto_met {
    println!("RPO/RTO达标");
}
```

## 8. 发布限制

RPO/RTO测量模块只用于隔离环境验证，不影响生产环境配置和运行。真实RPO/RTO测量需要独立的需求和审批流程。
