# C-06 主备fencing：设计文档

文档状态：`in_progress`。实现主备fencing模块。

## 1. 模块边界

- `crates/core/src/fencing.rs`：主备fencing核心模块，包含节点管理、令牌管理和故障检测。

## 2. 节点角色

| 角色 | 描述 | 状态 |
|---|---|---|
| Primary | 主节点 | 负责处理请求 |
| Standby | 备节点 | 待命状态 |
| Undefined | 未定义 | 初始状态 |

## 3. 节点状态

| 状态 | 描述 | 转换 |
|---|---|---|
| Online | 在线 | 可处理请求 |
| Offline | 离线 | 不可处理请求 |
| Failed | 故障 | 需要恢复 |
| Recovering | 恢复中 | 正在恢复 |

## 4. Fencing管理器设计

```text
FencingManager {
    nodes: HashMap<String, NodeInfo>,
    tokens: Vec<FencingToken>,
    current_token: Option<FencingToken>,
}

impl FencingManager {
    pub fn new() -> Self;
    pub fn register_node(&mut self, node: NodeInfo);
    pub fn get_node(&self, node_id: &str) -> Option<&NodeInfo>;
    pub fn get_nodes(&self) -> &HashMap<String, NodeInfo>;
    pub fn generate_token(&mut self, creator_node_id: &str, now_ms: u64, ttl_ms: u64) -> FencingToken;
    pub fn validate_token(&self, token_id: &str, now_ms: u64) -> bool;
    pub fn get_current_token(&self) -> Option<&FencingToken>;
    pub fn is_primary(&self, node_id: &str) -> bool;
    pub fn failover(&mut self, new_primary_node_id: &str, now_ms: u64) -> Result<()>;
    pub fn update_heartbeat(&mut self, node_id: &str, now_ms: u64) -> Result<()>;
    pub fn detect_failure(&mut self, node_id: &str, now_ms: u64, timeout_ms: u64) -> bool;
}
```

## 5. Fencing令牌结构

```text
FencingToken {
    token_id: String,
    token_value: String,
    created_at_ms: u64,
    expires_at_ms: u64,
    creator_node_id: String,
}
```

## 6. 测试套件设计

```text
FencingTestSuite {
    tests: Vec<FencingTest>,
}

impl FencingTestSuite {
    pub fn new_default() -> Self;
    pub async fn run_all(&self) -> Result<Vec<FencingResult>>;
    async fn run_test(&self, test: &FencingTest) -> Result<FencingResult>;
}
```

## 7. 失败关闭

- 节点注册必须包含完整信息。
- 令牌生成必须包含过期时间。
- 主备切换必须验证新主节点状态。
- 心跳检测必须支持超时配置。
- 所有测试必须在隔离环境运行，不影响生产环境。

## 8. 接入命令

Fencing管理器可通过以下方式调用：

```text
// 创建管理器
let mut manager = FencingManager::new();

// 注册节点
let node = NodeInfo {
    node_id: "node-1".to_string(),
    role: NodeRole::Primary,
    status: NodeStatus::Online,
    last_heartbeat_ms: 1000,
    address: "localhost:8080".to_string(),
};
manager.register_node(node);

// 生成令牌
let token = manager.generate_token("node-1", 1000, 3600000);

// 执行主备切换
manager.failover("node-2", 2000)?;
```

## 9. 发布限制

主备fencing模块只用于隔离环境验证，不影响生产环境配置和运行。真实主备切换需要独立的需求和审批流程。
