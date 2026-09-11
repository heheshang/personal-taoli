# PORT-01 codex 接入：任务文档

文档状态：**待所有者评审**（2026-09-10）。对应需求：`docs/PORT-01-codex接入-需求.md`；对应设计：`docs/PORT-01-codex接入-设计.md`。

**前置条件**：设计文档 §0 的决策（接入方式 / 运行边界 / 版本策略）经所有者确认。**未确认前，本任务文档不启动任何实现任务**（手册 §5.5 设计评审门禁）。

**拆分原则**：遵守手册 §2.5「每轮只建立一个新闭环」。下列每一轮都是一个可独立验证、可独立失败的闭环；**不得合并轮次**，不得在同一轮内同时引入进程集成与沙箱下沉。

---

## 轮次总览

| 轮次 | 目标（单一闭环） | 依赖 | 状态 |
|---|---|---|---|
| PORT-01-A | 只读握手：宿主能启动 `codex app-server` 并完成 `Initialize` | 决策确认 | **`verified`**（见下） |
| PORT-01-B | 自定义工具注入：模型能调用**一个**本项目只读工具并拿到结果 | A | **`verified`**（见下） |
| PORT-01-C | 人机门禁：所有副作用动作经所有者审批，审批决策落盘 | B | **`verified`**（见下） |
| PORT-01-D | 可复现：会话全量落盘 + 按 thread id 回放 | C | **`verified`**（见下） |
| PORT-01-E | 前端接入：一个可用的只读分析页 | D | **`verified`**（见下） |
| PORT-01-F | macOS 沙箱下沉为自有组件 | 决策 ④ | **`verified`**（见下） |
| PORT-01-G | 领域指令注入随版本管理 | B | **`verified`**（见下） |
| PORT-01-H | 外部 skill 接入（注册根 + 可写根） | G | **`verified`**（见下） |

> 所有者决策（2026-09-10）：**接入方式 ① app-server 进程集成**；**运行边界 = 只读分析**。
> 后续追加决策（2026-09-10）：**升级为 ④（① + ②，F 轮次）**，移植范围取 **② `.sbpl` 策略文本 + 文件系统策略子集**。

---

## PORT-01-A 只读握手

**目标**：宿主进程能拉起 `codex app-server`（stdio），完成 `Initialize`，并**干净关闭**。不发送任何 turn、不注册任何工具。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**，不得标记 `released`。

**范围**
- 新增 `crates/core/src/agent/{mod.rs, app_server.rs, protocol.rs}` 与 `crates/core/Cargo.toml` 的 tokio feature 增补。
- 子进程以**环境变量白名单**方式启动（`env_clear()` 后仅注入必要变量）；不得继承交易密钥。
- 实现 JSON-RPC over stdio 的请求/响应/通知三通道与握手。
- 进程退出、被杀死、协议错误三类路径都不得让宿主挂起。

**明确不做**：不注册工具、不发 turn、不写任何会话文件、不接前端。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| `Initialize` 往返成功，能力字段与 schema 一致 | ✅ | `agent::tests::handshake_succeeds_against_the_real_runtime`；实测返回 `{"codexHome":"/Users/shang/.codex","platformFamily":"unix","platformOs":"macos","userAgent":"taoli/0.154.0 (Mac OS 26.6.2; arm64) dumb (taoli; 0.1.0)"}` |
| 子进程环境**不含** `TAOLI_*` 密钥 | ✅ | `child_env_never_carries_host_credentials`（断言实际 `child_env()` 输出无 `TAOLI_` 前缀、无哨兵值）；另 `env_whitelist_contains_no_forbidden_name` 守住白名单本身；实测泄漏计数 = **0** |
| 外部杀死子进程后宿主返回错误而非挂起 | ✅ | `a_killed_child_fails_requests_instead_of_hanging`（组杀 → 等待 EOF → 20s 内返回 `Closed`/`Transport`） |
| EOF 时全部在途请求被失败（非悬挂） | ✅ | `eof_fails_every_pending_request_with_the_stderr_tail`（3 个在途请求全部收到带 stderr 尾巴的 `Closed`） |
| 二进制缺失时快速失败且给出可操作提示 | ✅ | `connect_fails_fast_when_the_binary_is_absent`（`Spawn` 错误，消息含 "Install codex"） |
| 关闭后无孤儿进程残留 | ✅ | `shutdown_leaves_no_process_in_the_agent_group`（按 `ps -g <pgid>` 断言组内为空） |
| `cargo check --workspace --all-targets` | ✅ | Finished，无 error/warning |
| `cargo test -p personal-taoli-core` | ✅ | **204 passed; 0 failed**（含新增 12 个 agent 测试） |
| 只读边界：`agent` 模块不引用交易写路径 | ✅ | `grep -rnE` 检索 `order::` / `execution::` / `account::` 于 `crates/core/src/agent/` → 无匹配 |

### 实现中发现的两个真实缺陷（均已修复，非设计预判）

1. **`codex` 是 shim，真 runtime 是孙进程。** 本机 `~/.bun/bin/codex` 为 Node 脚本，再 spawn 原生二进制；实测进程树 `node(shim) → codex(native) → git`。仅对直接子进程发信号会留下**正在运行的 agent runtime 并继续持有管道**。
2. **优雅关闭不回收孙进程。** 关闭 stdin 后 shim 正常退出，但 codex 派生的 `git`（拉取插件市场 `github.com/openai/plugins.git`）仍留在组内。

**修复**：`process_group(0)` 独占进程组；spawn 时捕获 pgid（因 reap 后 `Child::id()` 为 `None`）；`shutdown` 在优雅等待后**无条件清扫进程组**；`Drop` 做 best-effort 清扫。详见设计文档 §3.5。

> 这两个缺陷由验收测试暴露（首次运行 `a_killed_child...` 与 `shutdown_leaves_no_process...` 均失败），而非事后推测；修复后以「组内无残留进程」作为可验证不变量。

---

## PORT-01-B 自定义工具注入

**目标**：模型在会话中调用**一个**本项目只读工具，宿主执行并回结果。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**。

**范围**
- 新增 `crates/core/src/agent/{tools.rs, shadow_tool.rs}`，扩展 `app_server.rs`（`start_thread` / `start_turn` / `respond` / `respond_error` / `next_message`）与 `protocol.rs`（server→client 请求分类、`experimentalApi` 能力、thread/turn 参数构造）。
- 工具机制与具体工具分离：`tools.rs` 是机制（spec 构造、注册表、唯一路由点），`shadow_tool.rs` 是具体工具与其策略。
- 只读边界：工具仅调用既有核心读路径，不引用 `order`/`execution`/`account`。

**明确不做**：审批（轮次 C）、落盘/回放（轮次 D）、前端（轮次 E）。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 模型调用本项目自有工具并取回结果 | ✅ | `agent::tests::turn_calls_the_registered_read_only_tool`；实测最终消息 `"The report contains **284 records**."` |
| 工具返回与**直接调用同一核心读路径**一致 | ✅ | `agent::shadow_tool::tests::summary_matches_a_direct_replay_of_the_same_read_path`：逐字段比对该工具载荷与测试自行调用 `replay_archive` 的结果（`records` / `decision_records` / `last_event_at_ms` / `reconnects` … 全等） |
| 未注册工具被拒绝且不降级 | ✅ | `agent::tests::turn_refuses_a_tool_the_dispatcher_does_not_have`（回 `success:false` + 可用工具名；turn 仍正常完成而非挂起） |
| 陈旧数据失败关闭 | ✅ | `agent::shadow_tool::tests::stale_archive_fails_closed_instead_of_reporting_old_data`（容差 0 → 拒绝并给出两侧数值） |
| 归档缺失失败关闭 | ✅ | `agent::shadow_tool::tests::missing_archive_fails_closed_with_the_path` |
| 只读边界 | ✅ | `crates/core/src/agent/` 对 `order::` / `execution::` / `account::` 无匹配 |
| `cargo fmt --all -- --check` | ✅ | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | clean |
| `cargo test --workspace` | ✅ | **221 passed**（core）+ **8 passed**（tauri） |
| `npm run build` | ✅ | 零错误 |

### 实际报文（取自一次真实会话）

```jsonc
// thread/start 时广告给运行时的工具
[{"description":"Replay the Taoli observation archive and return the continuous shadow statistics …",
  "inputSchema":{"additionalProperties":false,"properties":{},"required":[],"type":"object"},
  "name":"taoli_shadow_report","type":"function"}]

// server → client：模型发起调用
{"id":0,"method":"item/tool/call","params":{"arguments":{},"callId":"call_00_Umb9cR7aB7h58tLwxcPS4804",
 "namespace":null,"threadId":"01a08ba4-8768-72b1-80b2-6a1dcc4f9226","tool":"taoli_shadow_report",
 "turnId":"01a08ba4-87a7-7bc2-bcbb-4eec677e54bc"}}

// client → server：宿主应答（载荷即核心读路径的 JSON 摘要）
{"id":0,"result":{"contentItems":[{"type":"inputText","text":"{\"schema_version\":1,\"records\":284,
 \"decision_records\":284,…}"}],"success":true}}
```

**交叉核对**（同一次运行）：直读 `replay_archive` 得 `records=284, decision_records=284, last_event_at_ms=1788913117097`；工具载荷三项全等 → `true/true/true`。

### 实现中发现的两个未预见约束（均已处理）

1. **`dynamicTools` 需要 `experimentalApi` 能力。** 不带时运行时明确拒绝：
   `{"code":-32600,"message":"thread/start.dynamicTools requires experimentalApi capability"}`。
   源码佐证 `app-server-protocol/src/protocol/v2/thread.rs:138` 的 `#[experimental("thread/start.dynamicTools")]`。该字段只在 `--experimental` schema 输出中（159 个 `ClientRequest` 变体 vs 默认 99）。
2. **server→client 请求带 `id` 且带 `method`。** 首版 `classify` 先判 `id`，会把这类请求误判为「响应」，导致服务器**永久等待** `item/tool/call` 的应答。修正为先判 `method`；已有回归测试 `classify_reads_server_requests_before_responses` 锁定。

> 第 2 条由对活体运行时的探针暴露，而非事后推测——这正是「先实测再实现」的价值。

### 测试对环境的两点依赖（诚实声明）

- `data/` 在 `.gitignore` 中，**不随仓库分发**。依赖归档的测试在归档缺失时**跳过**（打印 `skipping: …`）而不是失败，以保证新克隆可绿；因此这些测试的绿在本机是实证，在 CI 上需先产出归档。
- 依赖 `codex` 二进制的测试同样在二进制缺失时跳过。

---

## PORT-01-C 人机门禁

**目标**：所有会产生副作用的动作经所有者审批后才执行；拒绝不被绕过；超时失败关闭。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**。

**范围**
- 新增 `crates/core/src/agent/approval.rs`：审批请求归类、宿主裁决模型、线缆映射、审计日志。
- `run_turn` 改为接收 `TurnContext { tools, approvals, audit, limits }`（取代轮次 B 的三个位置参数），并对审批类 server request 给出裁决应答。
- `start_thread` 增加 `ApprovalPolicy` 参数（原因见下）。

**明确不做**：不做批量放行；不做「本会话内记住选择」的持久放行；不做 execpolicy / 网络策略修订——三者都会在无新的人工门禁下扩大未来权限，因此**在类型上不可达**。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 拒绝审批 → 动作未执行，agent 收到 declined | ✅ | `agent::tests::a_refused_command_is_denied_and_the_turn_survives`；模型最终回「The command was not run because the execution request was rejected.」 |
| 审批超时 → 失败关闭（不放行） | ✅ | `agent::tests::an_approval_that_times_out_is_refused_not_granted`（decider 永不返回 → 记录 `source = Timeout`、`decision = Deny`，且实测 `waited_ms ≥ 1000`） |
| 落盘记录条数与实际请求数一致 | ✅ | 同上两个测试均断言 `ApprovalLog::read_all(path) == outcome.approvals`；证据脚本实测 `records=1 matches outcome: true` |
| agent **不能**通过重试绕过拒绝 | ✅ | `agent::tests::a_refusal_cannot_be_bypassed_by_asking_again`：逐个断言每次请求均被判 Deny，且 `decider.seen == approvals.len()`（无请求被自动放行） |
| 单动作、无长期放行 | ✅ | `agent::approval::tests::approval_is_single_action_and_never_standing`：遍历 4 种 kind × 2 种裁决，断言载荷中不含 `acceptForSession` / `approved_for_session` / `execpolicy` / `networkPolicy` |
| 代际词汇正确（v2 vs v1） | ✅ | `each_kind_maps_to_the_vocabulary_of_its_protocol_generation`：v2 → accept/decline，v1 → approved/denied，双向断言 |
| `cargo fmt --all -- --check` | ✅ | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | clean |
| `cargo test --workspace` | ✅ | **239 passed**（core）+ **8 passed**（tauri） |
| `npm run build` | ✅ | 零错误 |

### 实际报文（一次真实会话，逐字）

```jsonc
// 收到的审批请求（注意 advertised 里没有 decline）
method     : item/commandExecution/requestApproval
kind       : command_execution
summary    : /bin/zsh -lc 'echo round-c-evidence'
advertised : ["accept", "acceptWithExecpolicyAmendment", "cancel"]

// 发出的裁决
{"id":0,"result":{"decision":"decline"}}

// 审计日志（append-only JSONL，逐字）
{"request_id":0,"method":"item/commandExecution/requestApproval","kind":"command_execution",
 "summary":"/bin/zsh -lc 'echo round-c-evidence'","decision":"deny","source":"decider",
 "decided_at_ms":1789050030794,"waited_ms":0,"advertised":["accept","acceptWithExecpolicyAmendment","cancel"]}

// 模型的最终回复 —— 拒绝生效且 turn 正常结束
"The command was not run because the execution request was rejected."
```

### 实现中发现的两个未预见约束（均已处理）

1. **沙箱与审批策略是两道独立的闸（重要）。**
   仅设 `sandbox: read-only` 时，模型运行 `echo hello` **不会产生任何审批请求**——只读沙箱仍允许不写盘的命令，且默认审批策略是 `on-request`（由模型自行决定是否询问）。
   实测对照：默认策略下无审批；显式设 `approvalPolicy: "untrusted"`（`UnlessTrusted`）后，运行时立刻抛出 `item/commandExecution/requestApproval`。
   → 因此 `start_thread` 增加 `ApprovalPolicy` 参数；R02 的成立**依赖宿主选定 `UnlessTrusted`**，不能只靠窄沙箱。
   源码佐证：`protocol/src/protocol.rs:992` 的 `AskForApproval`，`OnRequest` 标注为 `#[default]`。

2. **运行时提供的 `availableDecisions` 可能不含拒绝项。**
   实测该字段为 `["accept", "acceptWithExecpolicyAmendment", "cancel"]`——**没有 `decline`**；但本客户端回 `decline` 被接受并置为 `declined`。
   源码佐证语义：`tui/src/bottom_pane/approval_overlay.rs:824` 将 `Decline → ReviewDecision::Denied`、`Cancel → Abort`——即 `decline` 是"否掉本次动作"的精确表达，`cancel` 是更重的 Abort。
   → 实现选择最窄的拒绝（`decline` / v1 的 `denied`），并把 `availableDecisions` 原样写入审计记录，使上游若改变词汇可被察觉。该字段本身是**可选**的（`app-server-protocol/src/protocol/v2/item.rs:1592` 标注 experimental，且旧发送方可能不填）。

### 测试对环境的依赖（诚实声明）

与轮次 B 相同：依赖 `codex` 二进制与 `data/` 归档的测试在两者缺失时**跳过**而非失败。

---

## PORT-01-D 可复现

**目标**：给定 thread id 能还原完整调用链，用于归因（对应《Agent 评测白皮书》的「最小可归因」）。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**。

**范围**
- 新增 `crates/core/src/agent/trace.rs`：append-only JSONL 会话记录（`TraceWriter`）、结构校验回放（`replay` → `SessionTrace`）、`SandboxMode` / `ThreadOptions` 类型。
- 会话在 `start_thread` 时按 thread id 打开 trace（`<dir>/<thread_id>.jsonl`），每轮在 `run_turn` 内逐事件追加。
- **API 收敛**：`start_thread` 由 5 个位置参数改为 `(&ThreadOptions, tools)`——本轮新增第 6 项关注点，且其中两个是布尔量，位置传参容易被转置。

**明确不做**：不做自动归因、不做评测集、不做机评（属后续独立迭代）。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 按 thread id 还原完整调用链，且与实时一致 | ✅ | `agent::tests::a_live_session_replays_into_the_chain_it_recorded`：逐条比对回放出的 `tool_calls`（call_id / tool / arguments / success / output）、`approvals`、`refused_requests`、`final_message` 与 `TurnOutcome` |
| 回放确定性（同字节两次回放相同） | ✅ | `agent::trace::tests::replaying_the_same_trace_twice_is_identical`；证据脚本实测 `second replay identical: true` |
| 落盘按 thread id 寻址 | ✅ | `the_trace_path_is_addressed_by_thread_id` + 实测 `trace_path == <dir>/<thread_id>.jsonl` |
| 结构性损坏必须报错而非静默接受 | ✅ | 6 个负例：空 trace、坏行（含行号）、事件早于 `session_started`、第二个 `session_started`、`session_finished` 之后仍有事件、事件落在其 turn 结束之后、时间戳回退 |
| 未结束的会话不得表现成功 | ✅ | `an_unfinished_session_replays_as_unfinished_rather_than_as_success`（`finished=false`、`status=None`） |
| 失败的轮次记录原因 | ✅ | `a_failed_turn_records_why`（`Failed{detail}`） |
| ephemeral 线程仍留下宿主 trace | ✅ | `agent::tests::an_ephemeral_thread_still_produces_a_host_trace` |
| trace 目录不可用时在启动即失败 | ✅ | `agent::tests::an_unusable_trace_directory_fails_at_thread_start`（`AgentError::TraceFile`） |
| `cargo fmt --all -- --check` | ✅ | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | clean |
| `cargo test --workspace` | ✅ | **259 passed**（core）+ **8 passed**（tauri） |
| `npm run build` | ✅ | 零错误 |

### 实际 trace 文件（逐字，一次真实会话）

```jsonc
{"event":"session_started","thread_id":"01a08bb9-3a09-7281-aef4-bfbcdda95c8c","cwd":".","sandbox":"read-only","approval_policy":"unless_trusted","at_ms":1789050567237}
{"event":"turn_started","turn_id":"01a08bb9-3a46-7fb2-ae0d-e8884edcb432","prompt":"Call the taoli_shadow_report tool, then say how many records it reports.","at_ms":1789050567241}
{"event":"tool_call","call_id":"call_00_0a0rwLQQoJe24nCpwQwa9089","tool":"taoli_shadow_report","arguments":{},"success":true,"output":"{\"schema_version\":1,\"records\":284,…}","at_ms":1789050568075}
{"event":"item","turn_id":"01a08bb9-…","kind":"agent_message","text":"The report contains **284 records**.","at_ms":1789050569853}
{"event":"turn_finished","turn_id":"01a08bb9-…","status":{"outcome":"completed"},"final_message":"The report contains **284 records**.","at_ms":1789050569853}
{"event":"session_finished","at_ms":1789050569866}
```

回放结果与交叉核对（同一次运行）：
```
turns: 1 | status: Some(Completed) | tool_calls: 1 | approvals: 0 | refused: []
tool call counts: {"taoli_shadow_report": 1}
tool calls match : true      tool output match: true
approvals match  : true      final msg match  : true
second replay identical: true
```

### 设计要点

1. **为什么自己记录而不是读运行时的 rollout（实测依据）。**
   非 ephemeral 线程**确实**会在 `~/.codex/sessions/…/rollout-<ts>-<thread_id>.jsonl` 落盘，且 `thread/read(includeTurns)` / `thread/items/list` 能读回条目流（含我们工具返回的 `MARKET-42` 内容）。
   但**宿主侧事实在上游记录中不存在**：某次审批是「所有者拒绝」还是「超时被迫拒绝」（`DecisionSource`）、以及哪些 server request 因未实现被拒。因此 trace 不是冗余副本，而是宿主决策的唯一可恢复来源；同时它对 ephemeral 线程也有效（那类线程在运行时侧不留任何东西）。
   两者以 **thread id** 为共同键：trace 首事件记录它，据此可定位运行时的 rollout 作交叉核对。

2. **每个 turn 恰好一条 `turn_finished`。**
   `run_turn` 拆出 `drive_turn`（消息循环）后再由外层统一记录结束事件，因此超时、EOF、超限、轮次失败等**所有**退出路径都会留下结束记录——否则「未结束」与「崩溃」在回放中不可区分，而这正是归因时最需要区分的。

3. **顺序即证据，故回放做结构校验。**
   乱序不是被重排，而是报错（含行号）。特别地，落在自己 turn 结束之后的事件会被拒绝——否则它会被静默归到下一个 turn。

4. **逐事件 flush。**
   被 kill 的会话必须留下崩溃前的事件，因为最需要归因的正是紧邻崩溃的那几步。

### 实现中修掉的两个自身缺陷

1. **`TurnFinished` 的哨兵实现有 bug。** 首版用一个空 `turn_id` 的哨兵终止当前 turn，导致多轮场景下每轮末尾残留哨兵（2 轮回放出 3 轮）。改为显式 `Option<TurnTrace>` 持有当前轮，并在结束时 `take()` 入列；顺带获得「上一轮未结束就开新一轮」的检测能力。
2. **`session_finished` 从未被记录。** 首版只在 `start_thread` 记录会话开始，`shutdown` 未记录结束，导致回放恒为 `finished: false`。修正为**仅在 shutdown 成功后**记录，使 `finished: true` 表示「有意结束」而非「恰好停了」。

### 测试对环境的依赖（诚实声明）

与轮次 B/C 相同：依赖 `codex` 二进制与 `data/` 归档的测试在两者缺失时**跳过**而非失败。trace 的纯回放测试不依赖任何外部条件。

---

## PORT-01-E 前端接入

**目标**：一个只读分析页可用，且审批请求能呈现给所有者并得到裁决。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**。

**范围**
- 后端：新增 `src-tauri/src/commands/agent.rs`（`AgentController`）与 7 个命令；`dto.rs` 增 6 个 DTO；`lib.rs` 注册 `AgentController` 与命令。`src-tauri/Cargo.toml` 增 `async-trait`（实现 `ApprovalDecider` 所需）。
- 前端：新增 `src/components/AgentAnalysisPage.vue`；`commands.ts` 增 7 个命令名与类型；`App.vue` 增页面容器；`SidebarNav.vue` 增「个股分析」入口与图标。
- 会话形态与既有 `SessionController` 一致：长任务在后台任务里跑，控制器只持共享状态与命令通道，前端 1s 轮询。这样审批请求能呈现，`invoke` 不必长时间挂起。

**明确不做**：不修改任何既有命令或页面行为；不提供写操作工具；不做「本会话内不再询问」式的放行。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 只读分析页可渲染结论、工具调用与审批记录 | ✅ | 浏览器 mock IPC 渲染断言（仓库既有做法）：页面文本含「284 条记录」「taoli_shadow_report」「command_execution」「拒绝」「mcpServer/elicitation/request」 |
| 审批请求可呈现且可裁决 | ✅ | 点击「拒绝」发出 `agent_decide`，参数实测为 `{"requestId":7,"allow":false}` |
| 未就绪时给出可操作原因、不出现操作按钮 | ✅ | 未就绪态文本含「未找到 codex 可执行文件…」，且页内按钮数为 **0** |
| 无横向溢出 | ✅ | 1360×860 下六页 `scrollWidth - clientWidth` 均为 **0** |
| 既有页面不回归 | ✅ | 六页逐页切换均渲染（可见 `page-content`、文本非空、零溢出） |
| 视觉确认 | ✅ | 截图经视觉模型读图：审批面板为红框+警示底，与周围白色面板显著区分；无文字溢出或被裁 |
| `cargo fmt --all -- --check` | ✅ | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | clean |
| `cargo test --workspace` | ✅ | **259 passed**（core）+ **8 passed**（tauri） |
| `cargo build --workspace --release` | ✅ | Finished（2m16s） |
| `npm run build` | ✅ | 零错误 |

### 渲染断言取到的页面文本（节选）

```
 个股分析 · codex 只读分析
只读工具 · 人工审批 · /Users/shang/.bun/bin/codex
就绪   THREAD 01A08BBF   TRACE 01a08bbf-...jsonl
 分析结论
归档中有 284 条记录，观察时长约 14.6 小时，被接受的套利机会为 0 条。
成功  taoli_shadow_report
{"schema_version":1,"records":284,…}
 审批与拒绝记录
类型              动作                     裁决   来源      等待
command_execution /bin/zsh -lc 'echo probe' 拒绝   decider   0 ms
未实现而拒绝的请求（运行时发起了本页未实现的能力，已失败关闭）
mcpServer/elicitation/request
```

审批面板（待裁决态）：

```
 待审批动作
默认不执行；未裁决将按失败关闭
command_execution  /bin/zsh -lc 'echo probe'
[拒绝] [仅本次同意]
同意仅覆盖本次动作，不产生会话级放行，也不修改任何策略。
```

### E 修订：对齐 Codex 桌面版（2026-09-10）

**动机**：所有者要求「优化前端个股分析界面，使其跟 codex 桌面版一致」。原实现是「表单 + 结果面板」，与 codex 的**会话流**形态不同，故一并改结构与视觉。

**结构**：页面改为会话流（用户消息右对齐气泡 → 工具执行块可折叠 → 助手正文铺排 → 审批卡内联 → 进行中 chip），底部固定 composer（Enter 发送 / Shift+Enter 换行）。

**视觉**：新增 `src/codex-theme.css`，令牌取自本机 Codex 桌面版的实际样式表
（`/Applications/ChatGPT.app` 内 `webview/assets/app-initial-*.css`），非凭印象：
主色 `--blue-400:#0285ff`、聚焦环 `#339cffb3`、灰阶 `--gray-0…1000`、浅色前景
`#1a1c1f`、圆角 `6/8/10/12px`、`corner-shape: superellipse(1.5)`、工具栏 46px、聊天字号 16px。

**取舍（需所有者知悉）**：codex 用**蓝**主色，本项目全局设计令牌（UI-01）用**绿**。
为不改动其他页面已发布的视觉契约，蓝色令牌**作用域限定在 `.codex-scope`**，仅本页生效。
代价是本页主色与其余五页不一致；这是刻意的，若希望全局统一需另立一轮。

**契约变更（干净切替）**：`AgentStatus` 由「只存最近一轮的扁平字段」改为
`turns: Vec<AgentTurn>` 累积全部轮次——否则新一轮会抹掉上一轮，无法呈现会话流。
`dto.rs` / `commands.ts` 同步更新，旧扁平字段已删除。

**验证**：
- 渲染断言：会话流含用户消息、工具卡（含折叠/展开）、助手正文、内联审批卡；六页切换非回归；1360×860 与 1100×780 均零横向溢出。
- 令牌生效（运行时取值）：按钮 `border-radius 9999px` + 背景 `rgb(13,13,13)`（= codex `--gray-1000`）；composer `12px` + `corner-shape: superellipse(1.5)`（`CSS.supports` 实测为支持）。
- 未就绪态显示「未就绪」chip 且**无任何按钮**。

**本轮修掉的两个自身缺陷**：
1. 未就绪时工具栏仍显示「就绪」——它取的是会话 phase，与实际就绪状态无关，与下方「无法启动分析」直接矛盾。已改为就绪优先，并在未就绪时隐藏会话元信息。
2. `.codex-chip` 在会话流（flex column）中被**拉伸成整行色条**（实测 1244px）。已加 `align-self: flex-start`，修复后实测 69px。
3. （观察）「分析中」chip 原用橙色，与相邻审批卡同色系，视觉上被读作同一块；改为中性色，让真正需要注意的审批卡成为唯一信号。

**E 修订二：输入区尺寸与宽度（2026-09-10）**

所有者反馈「启动输入框太小了」→「宽度」。实测定位到根因，**不是高度，是宽度**：

| 项 | 修复前 | 修复后 |
|---|---|---|
| `.codex-composer-box` 宽 | **286px** | **1236px**（1360 视口） |
| textarea 宽 | **156px** | **1106px** |
| 1920 视口 textarea 宽 | 156px | 1666px |
| textarea 高（初始） | 单行 | 66px（三行起步）+ 随内容自动增高至 220px 上限后滚动 |

**根因**：`style.css:500` 有一条**裸 `footer` 元素选择器**（供 `AppFooter` 使用），
我的输入区当时用 `<footer class="codex-composer">`，因此被它命中，`display` 被改成
`flex`（横向），使输入框与提示行变成**同一行的两个 flex item**，输入框收缩到内容宽度。
该规则同时还给它加了本不该有的 `border-top` / `padding` / `letter-spacing`。

**修复**：改用 `<div>`（composer 不是文档页脚，语义也更正确），并在 `.codex-composer`
显式声明 `display: block` 并注明原因——使该处不再依赖「元素名恰好没被占用」。
`AppFooter` 的 `<footer>` 未改动，其全局样式不受影响。

**自动增高**：`rows="3"` 定下限（CSS `min-height: 66px`），`@input` 触发重算，
上限 220px 后由 `overflow-y: auto` 接管。归零 `height` 后再读 `scrollHeight`——
不归零则高度只增不减，清空后不会回落（实测清空后回到 74px）。

**仍未验证**：真实 Tauri 宿主中的观感与交互（与 E 原缺口相同，走 mock IPC）。

### F 修订：相对路径导致沙箱拒绝（2026-09-10）

**现场报错**（所有者实际运行）：
```
cannot confine agent runtime: cannot build sandbox profile: sandbox root must be absolute: data/agent-traces
```

**定位**：不是沙箱的缺陷，是**命令层传了相对路径**。`TRACE_DIR = "data/agent-traces"`
被原样交给策略构造器，而可写根必须绝对——沙箱按设计拒绝并失败关闭（这一点是对的，
按某个进程的 cwd 解释相对路径会授权一个调用方从未指名的目录）。

**修复**：
- `instructions::project_root()` / `project_root_from(base)`：由领域指令文档的位置反推项目根
  （就绪检查本就要求该文件存在，故它就是可靠锚点；不假设进程 cwd，打包后的应用从别处启动）。
- 命令层把 trace 目录拼到绝对项目根上并 `create_dir_all`；`ThreadOptions.cwd` 同改为项目根。

**实证**：
```
project root: /Users/shang/Documents/workspace/personal/ai/personal-taoli
writable roots:
  /Users/shang/.codex                                          absolute=true
  /Users/shang/Documents/.../personal-taoli/data/agent-traces  absolute=true
  -DWRITABLE_ROOT_1=/Users/shang/Documents/.../personal-taoli/data/agent-traces
```

**回归测试**（`src-tauri/src/commands/agent.rs`）：
- `the_trace_directory_is_absolute_and_usable_as_a_sandbox_root`：断言绝对、已创建、且同一调用链
  （`codex_confinement` → `confined_argv`）不再以「must be absolute」失败。
- `the_project_root_is_derived_from_the_instruction_document`：断言根绝对且确实含指令文档。

### 未执行的验证（明确记录）

- **真实 Tauri 宿主未验证**：渲染断言走 mock IPC（仓库既有做法，见 G-02 §2.4）。真实 WKWebView 内的观感与 `agent_start` 的真实链路（拉起 codex 子进程）由所有者在自己运行的实例上确认。
- **后端命令未经端到端实跑**：`agent_start` / `agent_ask` 依赖 `AgentSession`，其行为已在轮次 A–D 用真实 codex 逐项验证；但「通过 Tauri IPC 从界面点下去」这一整条链路只有编译与 mock 层保证。这是本迭代最主要的验证缺口。
- **审批超时在界面侧未实测**：`source: timeout` 的渲染分支只有单测（轮次 C）覆盖，未在界面上触发。

---

## PORT-01-F macOS 沙箱下沉

**目标**：把 macOS 沙箱下沉为自有组件，使「约束子进程」的能力不再依赖 codex 自己的沙箱行为；并把该约束真正施加到 agent 子进程上，不可用时**拒绝启动**。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**。

**范围（所有者选定 ②：`.sbpl` 策略文本 + 文件系统策略子集）**
- 新增 `crates/core/src/agent/sandbox/`：`mod.rs`（可用性探测 + 失败关闭的 argv 包装）、`seatbelt.rs`（移植的文件系统策略逻辑）、`policies/*.sbpl`（**上游逐字拷贝**）、`PROVENANCE.md`（来源与取舍记录）。
- 仓库根新增 `NOTICE`：Apache-2.0 §4(d) 要求保留上游 NOTICE。
- `AppServerConfig` 增 `confinement: Option<SandboxPolicy>`；配置后子进程经 `sandbox-exec -p … -D… -- codex app-server` 启动，沙箱不可用即 `AgentError::Confinement` 失败。
- 命令层 `agent_ready` 把沙箱可用性列为就绪前提，界面显示其状态与拒绝原因。
- 策略：写权限仅授予 codex home + trace 目录（**实测**：缺少 home 授权子进程会启动失败）；**工作区不在其中**（只读边界）。

**明确不做**：不移植 `codex_network_proxy`（28,397 行）、`codex_protocol::permissions`、逐命令权限模型、`PROTECTED_METADATA_PATH_NAMES` 与不可读 glob 策略、Windows/Linux 分支——均无调用方，理由逐条记在 `PROVENANCE.md`。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 沙箱内禁止写入授权范围之外 | ✅ | 实跑：写 `~/taoli-f-escape.txt` → `Operation not permitted`、文件不存在；单元测试 `confinement_is_applied_or_refused_but_never_skipped` |
| 授权范围内写入必须成功（否则沙箱等于坏的） | ✅ | 实跑：写 trace 目录 → rc=0、文件存在 |
| **agent 子进程在我们的沙箱下仍可用** | ✅ | 实跑握手成功；`agent::tests::the_runtime_starts_under_our_own_confinement` |
| 沙箱不可用时拒绝执行，而非放行 | ✅ | 探测为双向功能性验证；`an_unavailable_confinement_refuses_to_launch`；界面在不可用时就绪失败且**无任何按钮** |
| 符号链接别名被解析（否则授权静默失效） | ✅ | 输入 `/tmp/taoli-f-traces` → 实际发出 `-DWRITABLE_ROOT_1=/private/tmp/taoli-f-traces` |
| 路径不能注入策略语法 | ✅ | `a_path_with_a_quote_cannot_break_out_of_its_literal`：路径只以 `-D` 参数出现，不进入策略文本 |
| 符号链接可写根被拒（不授予未指名的目标） | ✅ | `a_symlinked_writable_root_is_refused` |
| 相对路径被拒（不被静默按 cwd 解析） | ✅ | `a_relative_root_is_refused_rather_than_resolved` |
| 上游策略文本逐字未改 | ✅ | `cmp` 四文件字节一致；`the_profile_text_taken_from_upstream_is_unchanged` |
| 许可证合规 | ✅ | `PROVENANCE.md` + 根 `NOTICE`（含上游 NOTICE 原文与不做选择性编辑的说明） |
| `cargo fmt --all -- --check` | ✅ | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | clean |
| `cargo test --workspace` | ✅ | **277 passed**（core）+ **8 passed**（tauri） |
| `cargo build --workspace --release` | ✅ | Finished |
| `npm run build` | ✅ | 零错误 |

### 实测报文与输出

```
=== 1) availability probe ===
Available

=== 2) policy ===
writable roots: ["/Users/shang/.codex", "/var/folders/…/T/taoli-f-traces"]

=== 3) negative control: write outside the allowlist ===
/bin/sh: /Users/shang/taoli-f-escape.txt: Operation not permitted
file exists: false          VERDICT: denied

=== 4) positive control: write inside an allowed root ===
rc=Some(0)  file exists: true   VERDICT: allowed

=== 5) codex app-server under our confinement ===
{"id":1,"result":{"userAgent":"taoli-f/0.154.0 (Mac OS 26.6.2; arm64) …",
                  "codexHome":"/Users/shang/.codex","platformFamily":"unix","platformOs":"macos"}}
VERDICT: codex works under our confinement

（证明符号链接归一化在起作用）
$ 输入 trace 目录 /tmp/taoli-f-traces
-DWRITABLE_ROOT_0=/Users/shang/.codex
-DWRITABLE_ROOT_1=/private/tmp/taoli-f-traces
```

### 实现中发现的两个未预见约束（均已处理）

1. **`/tmp`、`/var` 是符号链接，Seatbelt 按解析后路径匹配。** 字面 `/tmp/...` 的授权**永不命中**，且失败是**静默的**（写入被拒，但错误只在子进程稍后某个操作上冒出）。
   实测：授权 `/tmp/x` → 写入被拒；授权 `/private/tmp/x` → 写入成功；非符号链接目录直接成功。
   这正是上游 `normalize_top_level_alias` 存在的理由；**我自己的探针最初也踩了这个坑**（`std::env::temp_dir()` 返回 `/var/folders/…`），报出假失败。修正为：探针先解析路径，且**双向验证**（内允许、外拒绝）。
2. **`(allow default)` + 带过滤的 `deny` 不生效；应使用 codex 的反向 idiom** `(deny default)` + 显式 allow 列表。最初用前者时所有写入都被放行，误判为「沙箱无效」。

### 未执行的验证（明确记录）

- **未在真实的 `agent_start` 链路上端到端实跑沙箱启动**：后端 `run_agent` 已传入 `confinement`，编译与核心层测试覆盖；「从界面点击 → 经 IPC → 子进程在沙箱内起来」这一整条链路仍未实跑（与轮次 E 同一缺口）。
- **Linux/Windows 未实现**（本迭代仅 macOS）。

---

## PORT-01-G 领域指令注入随版本管理

**目标**：向 agent 注入本项目的领域约束，且注入内容来自**仓库内受版本控制的文件**，不在代码里硬编码。

**状态**：`verified`（2026-09-10）。实现与验证已完成；按手册 §5.5，**发布签收待所有者确认**。

**范围**
- 新增 `docs/agent/instructions.md`：领域约束文档（只读边界、数字以工具返回为唯一权威、Decimal 以字符串逐字引用、失败关闭、显式标注不确定、单轮单闭环、动作可审计、术语表）。
- 新增 `crates/core/src/agent/instructions.rs`：加载器（按 cwd 与可执行文件祖先目录解析，与观察配置同一搜索形态）+ 指纹（字节数 + FNV-1a）。
- `ThreadOptions.developer_instructions` → `thread_start_params` 的 `developerInstructions`；会话 trace 记录**指令指纹**，使回放能显示当时生效的是哪个修订。
- 命令层：`agent_ready` 把「指令可加载」列为就绪前提；界面显示其规模与说明。

**字段选择（实测依据）**：用 `developerInstructions` 而**非** `baseInstructions`。前者在运行时的提示词中作为**独立片段**追加（`core/src/session/mod.rs:4090` 的 `DeveloperInstructions::new(..).render_fragment()`），后者是整体替换基础提示词。领域约束属追加，不应覆盖运行时自身的操作指令。该字段**未受 `experimentalApi` 门控**，可直接使用。

**明确不做**：不在 Rust 代码里内联任何规则文本（否则代码副本与仓库副本会不一致）；不实现运行时可编辑的指令（应经评审改文件）。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| **指令真正抵达模型并被遵守** | ✅ | `agent::tests::the_domain_instructions_reach_the_model`：注入含任意暗号的规则后提问，模型答出该暗号 |
| 注入内容逐字来自文件，未被改写 | ✅ | `instructions::tests::the_loaded_text_is_the_file_verbatim` |
| 文件缺失 → 失败关闭，不用默认值 | ✅ | `a_missing_document_names_what_was_searched`（消息含期望路径）；`agent_ready` 因此不就绪 |
| 文件为空 → 失败关闭 | ✅ | `an_empty_document_is_a_failure_not_a_default`（消息含「拒绝在无领域约束的情况下启动」） |
| 文档关键规则被守住（防被抽空） | ✅ | `the_document_still_states_the_rules_that_matter`：断言只读边界等 6 条关键表述仍在 |
| 解析能向上找到仓库根 | ✅ | `resolution_walks_up_to_the_workspace_root`（从深层目录向上） |
| 空/缺省指令不发送空字符串 | ✅ | `protocol::tests::absent_or_blank_instructions_are_omitted_rather_than_sent_empty` |
| trace 记录当时生效的指令修订 | ✅ | `agent::tests::the_trace_records_which_instructions_were_in_force`（同文本指纹相等、异文本不等） |
| `cargo fmt --all -- --check` | ✅ | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | clean |
| `cargo test --workspace` | ✅ | **288 passed**（core）+ **8 passed**（tauri） |
| `cargo build --workspace --release` | ✅ | Finished |
| `npm run build` | ✅ | 零错误 |

### 探针输出（先于实现，确认字段可用）

```
init: True
thread/start with developerInstructions: ACCEPTED
model answer: ZEBRA-77
VERDICT: instructions reached the model
```

### 未执行的验证（明确记录）

- **未在真实 `agent_start` 链路上端到端实跑指令注入**：命令层已加载并传入，编译与核心层实测覆盖；「界面点击 → IPC → 指令生效」整条链路仍未实跑（与轮次 E/F 同一缺口）。
- 未验证约束的**遵守率**（如模型是否仍会编造数字）。本轮只证明指令**抵达**且模型能按其作答；约束效力属评测范畴（对应《Agent 评测白皮书》的评测集与归因），不在本迭代。

---

## PORT-01-H 外部 skill 接入（2026-09-10）

**背景**：所有者反馈「codex 中我已经安装的 uzi skill，项目中并没有进行调用」。

**取证结论（先于改动，全部实测）**：

| 问题 | 事实 |
|---|---|
| codex 里有 uzi skill 吗？ | **没有。** `skills/list` 实测只返回 6 个内置 `.system` skill（imagegen / openai-docs / plugin-creator / review-agent / skill-creator / skill-installer） |
| 那它装在哪？ | 装在 **Claude Code**：`~/.claude/plugins/cache/uzi-skill/…`；源码在 `~/Documents/workspace/personal/ai/stock/UZI-Skill` |
| codex 能发现它吗？ | **能。** `skills/extraRoots/set` 指向该仓库后，多出 5 个 `stock-deep-analyzer:*` skill（uzi / deep-analysis / investor-panel / lhb-analyzer / trap-detector），名字带来源命名空间 |
| 模型会用吗？ | **会。** 实测模型点名「分析一只 A 股时我会使用 `stock-deep-analyzer:uzi`」 |
| 项目为什么没调用？ | 我们 `thread/start` 的 `cwd` 是项目根，且**从未注册 extraRoots**——skill 在另一个目录，运行时不被告知就不会去找 |

**实现**：
- core：`AppServerConfig`/`AgentSession` 新增 `set_skill_roots`（`skills/extraRoots/set`，根须绝对）与 `list_skills`（`skills/list`，按名去重排序）；`ThreadOptions.skill_roots`；`AgentSession::skills()` 报告**运行时实际认可**的 skill；trace 记录本次会话注册的根。
- 配置（两个**正交**环境变量，刻意不合并）：
  - `TAOLI_AGENT_SKILL_ROOTS`：模型可**读**的 skill 目录
  - `TAOLI_AGENT_WRITE_ROOTS`：额外授权子进程**写**的目录
  分卡的理由：可见性与写权限是两件事，绑一起会让「只是想让模型读到这个 skill」顺手变成「授权它写这个目录」。二者都只接受绝对且存在的目录。
- 就绪检查与界面：显示 skill 根、可写根，以及运行时实际发现的 skill 数量与清单（hover）。

**为什么还需要可写根（实测缺口）**：skill 的指令是「跑脚本」，而脚本写自己的仓库。仅注册 skill 根时，模型会读完 `SKILL.md`、发起命令、拿到批准，然后**卡在写入**——
```
mkdir: .../UZI-Skill/.cache: Operation not permitted     ← 未授权可写根
rc=0，写入成功                                            ← 授权后
写 HOME 根仍被拒                                          ← 负向对照
```
这一组合（配了 skill 根、没配可写根）看起来正常却在写入时才失败，故界面在此时显式提示「缺可写根」。

**验收（可复现）**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 未注册根时看不到外部 skill | ✅ | `registering_a_skill_root_makes_its_skills_visible` 先断言基线不含 `:uzi` |
| 注册后可见且带来源命名空间 | ✅ | 同上，断言出现 `*:uzi` 且含 `:`；并断言每个 skill 的 `path` 真实存在、`description` 非空 |
| 不可用的根不拖垮会话 | ✅ | `an_unusable_skill_root_does_not_break_the_session` |
| 相对/不存在的根被丢弃 | ✅ | `parse_roots` 的 4 个纯函数测试 + 加锁的 env 接线测试 |
| skill 根不隐含写权限 | ✅ | `skill_and_write_roots_are_independent` |
| 可写根进入沙箱策略 | ✅ | `an_authorised_write_root_reaches_the_confinement` + 实跑写权限对照 |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **292 core + 17 tauri** |
| `npm run build` | ✅ | 零错误 |

**修掉的三处自身缺陷**：
1. 批量插入 `skill_roots` 字段时误伤枚举与结构体定义（已从 HEAD 恢复后精确重做）。
2. 两个新函数被追加到 `mod tests` 之后，clippy 报 `items after a test module`。
3. **env 变量测试并行竞态**：`cargo test` 默认多线程，多个测试同时读写同一环境变量导致失败（`--test-threads=1` 才通过，这不是修复）。改为「解析逻辑抽成纯函数 `parse_roots` + 少量 env 测试显式加锁」，使测试不再依赖调用方式。

**未验证**：未端到端跑完整 UZI 流水线（需数分钟 + 联网写 `reports/`）；已验证到「脚本可启动、可写入自己的仓库」这一层。

---

## 风险与依赖登记

| 风险 | 影响 | 处置 |
|---|---|---|
| `codex app-server` 上游标注 `[experimental]`，协议可能漂移 | 方法/字段变更导致集成失效 | 锁定版本（设计 §0 决策点三）；schema 从二进制生成而非硬编码；升级需人工评审 |
| **`dynamicTools` 依赖 `experimentalApi` 能力** | 上游若收紧/移除该能力，工具注入失效 | 已在 `protocol::initialize_params` 显式声明；运行时拒绝时错误信息直指缺失能力（实测报文见 B 小节） |
| **server→client 请求同时带 `id` 与 `method`** | 误判为「响应」会使运行时永久等待应答 | 分类顺序以 `method` 优先，并有回归测试锁定；未实现请求一律回 JSON-RPC error（失败关闭）而非静默丢弃 |
| **只读沙箱不产生审批流量** | 误以为「沙箱窄 = 有门禁」，实际副作用动作可能不经人工确认 | 宿主必须显式设 `approvalPolicy = "untrusted"`；已固化为 `ApprovalPolicy` 类型 + 实测对照记录 |
| **`availableDecisions` 可能不含拒绝项** | 若依 advertised 集合挑 token，会找不到可用的拒绝值 | 取最窄拒绝（`decline` / v1 的 `denied`，实测被接受），并把 advertised 原样写入审计以便察觉上游词汇变化 |
| `/usr/bin/sandbox-exec` 已被标记 deprecated | 未来 macOS 可能移除 | **已实现可用性探测 + 失败关闭**（F 轮次）：探测是双向功能性验证，不可用时 `agent_ready` 不就绪、启动返回 `AgentError::Confinement` |
| **路径别名使授权静默失效** | Seatbelt 按解析后路径匹配，`/tmp`、`/var` 是符号链接；字面别名永不命中且不报错 | 已移植上游 `normalize_top_level_alias` 并实测（`/tmp/...` → `-D…=/private/tmp/...`）；测试覆盖符号链接可写根被拒 |
| **策略 idiom 选错会整体失效** | 用 `(allow default)` + 过滤 deny 时所有写入被放行，看起来「沙箱无用」 | 采用上游的 `(deny default)` + 显式 allow；测试断言策略含 `(deny default)` 且无无条件的写授权 |
| 本机 codex 经本地代理（`deepseek-flash`） | 会话结果受代理影响 | 验证阶段记录实际 provider/model；不以单次结果作能力断言 |
| 接入被误用为交易路径 | 违反架构文档 §1.2 | 只读工具集 + 环境变量白名单 + 不注册任何写工具；在验收中逐条证明 |
| 与 A-05 影子窗口门禁的关系 | 可能影响当前 `pending` 状态 | **已确认无影响**（见下） |
| **领域指令被抽空或误删** | agent 仍能运行，但失去只读边界与「数字以工具返回为准」等约束，且外观上无法察觉 | 文件缺失/为空即拒绝启动；测试断言 6 条关键表述仍在；trace 记录指令指纹，可比对两个 run 是否同一修订 |
| `data/` 不随仓库分发 | 依赖归档的测试在新克隆上跳过而非失败 | 已在文档标注依赖；这些测试的绿在本机为实证，CI 需先产出归档 |
| **前端只经 mock IPC 验证** | 真实 WKWebView 与真实 IPC 链路（`agent_start` 拉起子进程）未验证 | 仓库既有做法如此（G-02 §2.4）；已在任务文档「未执行的验证」中明确记录，留待所有者在自己实例上确认 |
| **审批超时的界面分支未实测** | 界面在 `source: timeout` 时的呈现未真实触发 | 后端语义有单测覆盖（轮次 C）；界面分支只经 mock 呈现，已记录为缺口 |
| **trace 与上游 rollout 的信息重叠** | 有人可能误以为 trace 是上游记录的冗余副本 | 两者互补且以 thread id 为共同键；宿主侧决策（审批来源、被拒请求）仅存在于 trace，已在设计 §3.5 写明 |
| **trace 写入失败被吞** | 丢失审计线索而不自知 | 写入失败以 `tracing::error` 上报，turn 继续（不因 trace 失败而丢轮次）；`session_finished` 仅在 shutdown 成功后记录，故 `finished:true` 可信 |

### A-05 前置检查结论（轮次 B 前置项，已完成）

A-05 处于只读、14 天连续观察窗口运行中（最早评审 2026-09-22T09:52:14Z，手册 §10）。
`crates/core/src/agent/` **不被任何既有代码引用**（`grep 'agent::'` 于 `src-tauri` 与 `crates/core/src`（除 agent 自身）→ 无匹配），是惰性库模块：不启动即不产生进程、不打开文件、不占用端口、不触碰数据库或归档写入路径。
**结论：接入不改变观察循环，也不解除 A-05 门禁。**

## 下一步

轮次 A–G 均 `verified`。PORT-01 的计划轮次已全部完成；发布签收按手册 §5.5 待所有者确认。

**阻塞项**：所有者对设计文档 §0 决策点三（codex 版本锁定策略）的裁定。该决策不阻塞 C 的实现（C 只依赖审批类 server request 的形状，已可从 schema 获取），但应在发布签收前确定。
