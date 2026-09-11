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
| PORT-01-I | 移除作用域错误的宿主工具 | H | **`verified`**（见下） |
| PORT-01-J | 审批动作对齐 Codex 桌面版（四档） | C | **`verified`**（见下） |
| PORT-01-K | 禁止长期放行（carve-out + 不提供第四档） | J | **`verified`**（见下） |
| PORT-01-L | 操作档位（请求批准 / 帮我批准 / 完全访问权限） | K | **`verified`**（见下） |
| PORT-01-M | 轮次进度与工具/skill 的流式展示 | L | **`verified`**（见下） |
| PORT-01-N | 轮次超时改为静默判据 + 超时中断 | M | **`verified`**（见下） |
| PORT-01-O | 分析结果进对话 + Markdown 渲染 | N | **`verified`**（见下） |
| PORT-01-P | 撤掉静态分析结果页，结果即真实输出 | O | **`verified`**（见下） |
| PORT-01-Q | 结构化报告（outputSchema + 校验 + 渲染） | P | **`verified`**（见下） |

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

## PORT-01-I 移除作用域错误的宿主工具（2026-09-10）

**背景**：所有者判定「现有工具只能回放套利观察归档，这个是不对的。去除这个工具」。

**依据**：`taoli_shadow_report` 汇报的是**跨交易所套利观察归档**（记录数、观察时长、
被接受的套利机会、拒绝原因分布）。这与个股分析无关——分析一家上市公司与套利簿记
没有任何关系。把它作为唯一的宿主工具，等于给这个页面配了一个答非所问的能力；而且
它在真实归档上会失败（见下）。

**范围**：
- 删除 `crates/core/src/agent/shadow_tool.rs`（336 行）与 `mod.rs` 的模块声明。
- 命令层不再注册任何宿主工具（保留 `ToolRegistry` 与注册点，给将来作用域正确的工具）。
- 就绪检查**去掉归档门控**——归档只是那个已被移除的工具的输入，留着会让缺归档时无故拒绝启动。
- `AgentReady` 去掉 `archive_path`；前端去掉「归档」检查项；提示词与文案改为不提宿主工具。
- 领域指令同步两处**已不成立的事实**：原文写「宿主提供的工具是只读的（观察归档统计）」，
  现在宿主不注册工具；并新增一条约束——不得把套利观察归档当作个股分析依据。
- 顺带清掉因移除而变成死代码的 `trace::outcome_text` 及其测试。

**保留**：`archive::ShadowReport` / `replay_archive` / `replay_observations` / `DetailPanel`
的归档回放是**既有的 A-05 功能**（控制台「回放」按钮），与 agent 工具无关，未改动。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 工具及其全部引用消失 | ✅ | `grep` 检索 `taoli_shadow_report` / `ShadowReportTool` / `shadow_tool`，源码中仅剩两处**刻意保留的移除记录**注释 |
| 归档不再是启动前提 | ✅ | `readiness_does_not_gate_on_the_archive`：缺归档时就绪为真，且原因不含「归档」 |
| 宿主不注册工具 | ✅ | `the_host_registers_no_tools`：注册表为空、不上广告 |
| 工具机制本身仍有测试 | ✅ | 改用仅存在于测试的 `taoli_test_echo`：调用往返、未注册即拒（失败关闭）、trace 归因，三条均通过 |
| 既有归档回放不受影响 | ✅ | `commands::archive::tests::replay_reports_missing_archive_before_opening_gap_journal` 通过 |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **286 core + 19 tauri** |
| `npm run build` | ✅ | 零错误 |
| 界面不再出现归档 | ✅ | 渲染断言：无「归档」检查项、无「只读工具」文案，`skill 1` 正常，零横向溢出 |

测试数由 292 降到 286：删掉 6 个与已移除工具直接相关的测试（其机制部分已由 echo 工具覆盖）。

**附带说明**：该工具在真实归档上会失败，因为归档含 4 处**跨 run 时间回退**
（同一 pid 的多轮 writer，后一轮的首个事件早于前一轮末尾，最大 12.7 秒），而
`replay_archive` 要求全局时间单调。这是归档侧的独立问题，不在本次移除范围内，
未做改动。

---

## PORT-01-J 审批动作对齐 Codex 桌面版（2026-09-10）

**背景**：所有者指令——「执行门禁只能：仅本次同意，这个需要跟 codex app 中的执行门禁一样」。

即：取消我在轮次 C 按 R08 施加的「单动作」限制。**这是所有者的决定，原 R08 已被取代**
（改为 R08 新表述：四个动作 + 越权守卫 + 超时仍拒绝）。

**取证（先于改动）**：从本机 Codex 桌面版 `app.asar` 的语言包读到审批卡文案——

```
approvalRequestCard.allowOnce         : 允许一次
approvalRequestCard.allowConversation : 允许此对话
approvalRequestCard.alwaysAllow       : 始终允许
approvalRequestCard.deny              : 拒绝
approvalRequestCard.approvalOptions   : 审批选项
```

对应线缆词汇（`--experimental` schema + 活体实测）：

| 动作 | v2 命令/文件变更 | v1 旧方法 | 权限 |
|---|---|---|---|
| 允许一次 | `accept` | `approved` | `{permissions: <请求的>}`（scope 默认 `turn`） |
| 允许此对话 | `acceptForSession` | `approved_for_session` | 同上 + `scope: "session"` |
| 始终允许 | `{acceptWithExecpolicyAmendment:{execpolicy_amendment:[…]}}` | `{approved_execpolicy_amendment:{…}}` | 协议无此词，不提供 |
| 拒绝 | `decline` | **`{denied:{rejection:…}}`** | `{permissions:{}}` |

**实现**：
- `Decision` 由 `Allow/Deny` 扩为 `AllowOnce/AllowForSession/AllowAlways/Deny`。
- 新增 `available_decisions(kind, params)`：可用集合由**请求本身**算出，而非照抄运行时的
  `availableDecisions`（该字段可选、实测不含 `decline`）。未附 `proposedExecpolicyAmendment`
  时不提供「始终允许」——记不住东西的「始终允许」是假的。
- 命令层：`agent_decide(request_id, decision)` 收 token；新增 `ensure_offered` 越权守卫
  （不得选用未提供的选项，例如对未附规则的请求要求永久授权）。
- 界面：审批卡按 `options` 渲染四个动作，中文文案取自 codex app 同一组词；按钮 title
  说明各自的作用范围。
- 超时与无裁决通道仍是 **拒绝**（`DenyAll` 未变）：问不到人时不得假定同意。

**顺带修掉一个既有错误**：legacy（`execCommandApproval` / `applyPatchApproval`）的拒绝我原先
发的是字符串 `"decision":"denied"`，而 schema 要求 `{"denied":{"rejection":…}}`（`rejection`
为必填）。字符串形式会在协议层被拒 —— 即**操作员点「拒绝」反而失败**。已修正，并加防回归测试。

**实测（活体运行时，用本模块的 `response_payload` 构造回复）**：

```
allow_once        -> {"decision":"accept"}                        命令 completed
allow_for_session -> {"decision":"acceptForSession"}              命令 completed
allow_always      -> {"decision":{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["echo","verdict"]}}}   命令 completed
deny              -> {"decision":"decline"}                       命令 declined
```

**「始终允许」是持久的——实测确认并已清理副作用**：它会把 `prefix_rule` 写入
`~/.codex/rules/default.rules`。探针期间写入的 3 条（`echo amend-probe` / `echo verdict-probe`
/ `echo verdict`）**已从你的配置中删除**（备份 `/tmp/default.rules.bak`，当前 56 条为你原有规则）。
因果也已闭环验证：规则存在时该命令不再询问、直接执行；删除规则后重新询问且拒绝生效
（命令状态 `declined`）。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 四个动作可点且文案与 codex app 一致 | ✅ | 渲染断言：按钮 = `["允许一次","允许此对话","始终允许","拒绝"]` |
| 每个动作映射正确且被运行时接受 | ✅ | 上表四条活体实测（completed / declined） |
| 未附规则时不出现也不接受「始终允许」 | ✅ | 渲染断言（3 个按钮）；`a_verdict_outside_the_offered_set_is_refused` |
| token 往返与互异 | ✅ | `every_decision_round_trips_through_its_token`（4 个 token 互异） |
| 未知 token 被拒而非默认 | ✅ | `an_unknown_token_is_refused_rather_than_defaulted` |
| legacy 拒绝带 `rejection` 且不是裸字符串 | ✅ | `the_legacy_refusal_carries_its_rejection_field`、`the_legacy_refusal_is_never_the_bare_string` |
| 已裁决记录显示中文标签 | ✅ | 渲染断言含「始终允许 · command_execution」 |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **292 core + 22 tauri** |
| `npm run build` | ✅ | 零错误 |

**审计格式变化**：`ApprovalRecord.decision` 的序列化值由 `allow`/`deny` 变为
`allow_once`/`allow_for_session`/`allow_always`/`deny`。审计为本地 append-only 记录，
无兼容承诺；旧记录仍可读（枚举新增变体不影响既有值得解析）。

---

## PORT-01-K 禁止长期放行（2026-09-10）

**背景**：所有者指令——「（长期放行落盘的问题）需要做」。

**这暴露了 J 轮次里一个真实的矛盾**，必须先讲清楚，因为它决定了取舍：

| 诉求 | 来源 | 与另一条的关系 |
|---|---|---|
| 审批四档与 codex app 一致（含**始终允许**） | J 轮次（所有者） | — |
| 不让「始终允许」在本机**长期生效** | K 轮次（所有者） | **与上一条在第四项上互斥**：第四项的全部含义就是写下一条长期规则 |

不能既保留一个承诺永久的按钮、又让它不生效 —— 那是我在 C 轮次就明确拒绝做的假话。因此 K 的实现是：
**保留四档机制，但在本页禁用第四档并说明原因**。

**测量（先于取舍）**：给沙箱子进程加上「规则目录不可写」后，`acceptWithExecpolicyAmendment` 的真实行为——

```
approval -> {"decision":{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["echo","taoli-repeat-…"]}}}
approvals raised : 3          ← 第二条同样的命令仍被门禁拦住（还额外触发了一次）
command items    : ["declined/\"\"", "completed/\"\"", "declined/\"\""]
rule persisted   : false
VERDICT: allow_always left no effect
```

即：**被接受、然后什么都不做**。用它当「允许一次」是误导；用它当按钮则承诺无法兑现。

**实现**：
- 沙箱新增 `deny_write_subpaths`（`(deny file-write* (subpath …))`），**排在所有 allow 之后**——Seatbelt 后者胜出，deny 若排在前面会被后续 allow 悄悄重新打开。
- `codex_child_policy` 恒定 carve out `<codex_home>/rules`（`RULES_SUBDIR` 单点命名）。这样「接受然后无效果」不会再发生：规则根本写不进去。
- `SandboxPolicy::blocks_rule_persistence()` 把「沙箱是否禁止写规则」变成可询问的事实；命令层据此选择 `approval::Persistence::{Allowed, Blocked}`，`available_decisions` 在 `Blocked` 时不提供 `AllowAlways`。
- 界面按 options 渲染；第四项缺席时给出说明，而不是让使用者对着 codex app 少一个按钮发愣。

**绑定防漂移**：`the_child_policy_reports_that_rule_persistence_is_blocked` 断言子进程策略确实禁止写规则，
`the_permanent_grant_is_not_offered_when_persistence_is_blocked` 断言该状态下不提供第四档。若有人移除
carve-out，前者失败，而不是让一个「始终允许」按钮悄悄回来继续被沙箱忽略。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 子进程无法写运行时的规则目录 | ✅ | 实测：`allow_always` 下 `default.rules` 字节数不变、marker 不在其中；`the_codex_child_policy_denies_writes_to_the_rules_directory` |
| carve-out 排在所有 allow 之后 | ✅ | `a_carve_out_is_emitted_after_every_allowance`（含 platform defaults 之后） |
| 无法兑现的第四档不被提供 | ✅ | `the_permanent_grant_is_not_offered_when_persistence_is_blocked`；渲染断言按钮 = `["允许一次","允许此对话","拒绝"]` |
| 越权守卫拦截绕过界面的调用 | ✅ | `a_verdict_outside_the_offered_set_is_refused` |
| 界面说明第四项缺席原因 | ✅ | 渲染断言含「本页没有「始终允许」：运行时的规则目录被设为不可写」 |
| 机制未被删死（宿主若允许持久化仍给四档） | ✅ | 渲染断言：options 含 `allow_always` 时按钮为四个 |
| 相对 carve-out 被拒 | ✅ | `a_relative_carve_out_is_refused` |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **297 core + 22 tauri** |
| `npm run build` | ✅ | 零错误 |

**实测确认无副作用**：测量期间 `~/.codex/rules/default.rules` 由探针快照并在结束时比对，两次测量均未发生变化；
此前 J 轮次探针写入的 3 条已删除。你现有的规则条数未受影响。

**仍未验证**：未在真实 Tauri 宿主里跑（沿用 mock IPC）；未测 carve-out 是否影响 codex 的其他功能
（它只在写规则目录时会失败，读取不受限）。

---

## PORT-01-L 操作档位（请求批准 / 帮我批准 / 完全访问权限）（2026-09-10）

**背景**：所有者问「codex app 的请求批准、帮我批准、可完全访问权限是怎么设计的，在本项目中实现」。

**取证（先于实现）**：

* **app 的权限下拉**（`composer.permissionsDropdown.*`）：`请求批准` / `帮我批准` / `完全访问权限` / `自定义 (config.toml)` / `由组织管理`，标题「应如何批准 Codex 操作？」。
* **Rust 预设**（`utils/approval-presets/src/lib.rs:28`）把每档定义为 `(AskForApproval, PermissionProfile)` 的一对，而非单一旋钮。
* **`ApprovalsReviewer`** = `user` | `auto_review` | `guardian_subagent`（legacy）。`auto_review` 由**运行时的审查子代理**按风险判定——即「帮我批准」会绕过本页的审批卡。

因此档位 = 三件事的合取：`sandbox` + `approvalPolicy` + `approvalsReviewer`，加上**本项目自己的 Seatbelt 形状**。

**实现**：
- 新增 `crates/core/src/agent/access.rs`：`AccessLevel` 与 `ApprovalsReviewer`，档位本身即单位（不暴露三个独立旋钮，否则可拼出 app 不提供、本项目也未推理过的组合）。
- `thread_start_params` 增 `approvalsReviewer`；`ThreadOptions` 增该字段并记入 trace（向后兼容：新字段 `#[serde(default)]`）。
- `codex_child_policy` → `AccessLevel::confinement(codex_home, trace, workspace)`：`Ask` 维持原只读边界；`AutoApprove` 追加**工作区**可写；`FullAccess` 返回 **None**（施加任何约束都会让该档位的说明变成假话）。
- 顺带修掉 clippy 的参数过多：`AppServer::start_thread` 改为接收 `&ThreadOptions`，不再传 7 个位置参数（其中 3 个是字符串）。
- 命令层：`agent_access_levels` / `agent_set_access_level`；会话运行中改档**拒绝**并说明原因（沙箱无法追加到已在运行的进程）。
- 界面：工具栏下拉（与 app 同构，会话运行中禁用）；当档位会绕过审批或不施加沙箱时，在输入区上方显式说明。

**实测（活体运行时，每档各起一个真实线程）**

| 档位 | 运行时 sandbox / policy / reviewer | 我们的沙箱可写根 | 本页裁决器被调用 | 越出询问档白名单写入 |
|---|---|---|---|---|
| 请求批准 | `read-only` / `untrusted` / `user` | home + trace | **1 次** | **失败**（`Operation not permitted`） |
| 帮我批准 | `workspace-write` / `on-request` / **`auto_review`** | home + trace + 工作区 | **0 次** | 失败（$HOME 不在工作区，codex 自行拒绝） |
| 完全访问权限 | `danger-full-access` / `never` / `user` | **无** | **0 次** | **成功** |

三点值得单独记住：
1. 只有 `Ask` 会把请求交给本页；`AutoApprove` 由运行时自审，**本页的审批卡不会出现**——界面已显式说明，否则会让人以为每个动作仍由自己把关。
2. `FullAccess` 下本项目**不施加任何约束**，agent 可写任意路径并联网。
3. `AutoApprove` 仍然被**codex 自己的** `workspace-write` 沙箱约束（写 $HOME 被拒），所以该档并非无限制。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 三档标签与说明与 app 一致 | ✅ | 渲染断言下拉 = `["请求批准","帮我批准","完全访问权限"]`；`the_labels_are_the_ones_the_app_uses` |
| 每档映射为 app 的三元组 | ✅ | `each_level_maps_to_the_runtimes_own_settings` + 上表活体实测 |
| 只有询问档会咨询本页 | ✅ | `only_the_asking_level_consults_this_client` + 实测「裁决器被调用 1 / 0 / 0 次」 |
| 沙箱随档位放宽，完全访问无沙箱 | ✅ | `confinement_widens_with_the_level_and_vanishes_at_full_access` + 实测写入三态 |
| 界面说明绕过审批与无沙箱 | ✅ | 渲染断言：`ask` 无警告；`auto_approve`/`full_access` 各显示对应说明 |
| 会话运行中改档被拒 | ✅ | `a_running_session_accepts_only_its_own_level`；界面在会话运行中禁用下拉 |
| 未知 token 被拒而非回退 | ✅ | `an_unknown_level_token_is_refused` + `an_unknown_level_is_refused` |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **305 core + 27 tauri** |
| `npm run build` | ✅ | 零错误 |

**实现中修掉的一个真实缺陷**：TS 接口把后端的 `consults_client` 写成 `consultsClient`。字段名不符会让读取恒为 `undefined`，而 `undefined` 在布尔位置是 falsy —— 于是「是否绕过审批」的判断**静默走错分支**（「请求批准」也误报「不会出现审批卡」）。已对齐为 serde 原名并渲染复核。

**仍未验证**：未在真实 Tauri 宿主里跑（沿用 mock IPC）；`auto_review` 的判定质量（是否会错误放行风险动作）未评估——那是运行时子代理的行为，属评测范畴。

---

## PORT-01-M 轮次进度与工具/skill 的流式展示（2026-09-10）

**背景**：所有者指令——「分析中要显示当前调用的进度和执行的工具以及使用 skill 等信息。要流式显示」。

**取证（先于实现）**：一轮里运行时到底发什么。实测一轮 `sleep` 循环命令，收到 10 余种通知、
字段形状如下（截断展示）：

```jsonc
{"method":"item/started","params":{"item":{"type":"commandExecution","id":"call_…",
 "command":"/bin/zsh -lc 'for i in 1 2 3; do …'","cwd":"/tmp","status":"inProgress"},
 "threadId":"…","turnId":"…"}}
{"method":"item/commandExecution/outputDelta","params":{"delta":"line-2\n","itemId":"call_…"}}
{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"call_…",
 "command":"…","aggregatedOutput":"line-1\nline-2\nline-3\n","exitCode":0,"durationMs":3895,"status":"completed"}}}
{"method":"item/reasoning/summaryTextDelta","params":{"delta":"…","itemId":"…"}}
{"method":"item/agentMessage/delta","params":{"delta":"…","itemId":"…"}}
{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"total":{…},"last":{…},"modelContextWindow":121600}}}
{"method":"thread/status/changed", …}
{"method":"serverRequest/resolved", …}
```

**关于「使用 skill」——必须说清楚的一点**：协议里**没有**「skill 被调用」这一信号（我逐个查了
item 类型与通知方法，没有 skill 专用类型）。skill 的体现就是普普通通的工具与 shell 调用
（读它的 `SKILL.md`、跑它的脚本）。因此界面**并排**呈现「可用 skill 清单」与「调用流」，
`stock-deep-analyzer:uzi` 会作为一次工具调用出现在流里，而不是凭空生成一个协议不支持的归属。

**实现**：
- 新增 `crates/core/src/agent/progress.rs`：把 80 余种通知收敛为一小组 `TurnProgress` 事件
  （`Started` / `ReasoningDelta` / `MessageDelta` / `ItemStarted` / `ItemOutput` /
  `ItemFinished` / `Tokens` / `ApprovalRequested` / `ApprovalResolved` / `Stage`）。
  **增量按 chunk 转发**而非累积文本：这一层因此无状态，消费者按 `item_id` 自行累加。
- `mod.rs` 把相关通知翻译后转发给 `ProgressSink`；sink 在 `TurnContext` 里，`None` 即静默运行（测试用）。
- 新增 `src-tauri/src/commands/agent_live.rs`：把事件折叠进共享状态的 `live` 块。两条边界：
  **流式文本与单步输出都只保留尾部**（每轮可产出兆字节，而这个状态每次轮询都要序列化）；
  **轮次结束后清空** `live`（届时权威数据在 `turns` 里，留着会重复渲染同一批工作）。
- 界面在会话流顶部新增实时区：阶段 chip、token 用量、推理/正文流式文本、逐步卡片
  （类型标签 + 标题 + 状态 + 退出码/耗时），运行中的 chip 有脉冲动画（并尊重
  `prefers-reduced-motion`）。

**实测（真实会话，括号内为事件抵达时刻）**

```
[   162 ms] ItemStarted(userMessage)
[   987 ms] ItemStarted(Reasoning reasoning)
[  1745 ms] ItemStarted(Command /bin/zsh -lc 'for i in 1 2 3; do echo line-$i; sleep 1; done')
[  1745 ms] ApprovalRequested(/bin/zsh -lc 'for i in 1 2 3; …')
[  1745 ms] ApprovalResolved(0)
[  2784 ms] ItemOutput("line-2")
[  3792 ms] ItemOutput("line-3")
[  4829 ms] Tokens(total=7732)
[  5699 ms] ItemStarted(Message agentMessage)
[  6321 ms] Tokens(total=7941)
[  6321 ms] Stage(Done)
```

事件在 **6.3 秒里陆续抵达**，而非结束时一次吐出——这就是「流式」的判据。命令输出在命令
**执行期间**到达（1745ms 启动，2784/3792ms 收到两行），token 也在中途更新。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 进度实时可见（非结束时才有） | ✅ | 上表时间戳；`cargo run --example stream_check` 实测 |
| 展示执行的工具/命令与其状态 | ✅ | 渲染断言：命令卡「进行中… · line-1」、另一卡「退出码 0 · 12 ms」；单测覆盖 running→output→exit |
| 展示 skill 信息 | ✅ | 工具栏 skill 计数与清单（hover）；skill 调用以工具卡出现在流中（`stock-deep-analyzer:uzi`） |
| 流式文本与输出 | ✅ | `ReasoningDelta`/`MessageDelta`/`ItemOutput` 三条分支 + 渲染断言（推理文本、命令增量） |
| token 用量 | ✅ | 渲染断言「7,732 / 121,600 TOKENS」 |
| 文本有界（不随轮次无界增长） | ✅ | `long_output_keeps_a_bounded_tail`（500 行后仍 ≤ 上限且保留末行） |
| 上一轮的迟到事件不改写本轮 | ✅ | `events_for_another_turn_are_ignored`（turn_seq 守卫） |
| 轮次结束后清空实时块 | ✅ | 代码路径 + `a_missing_live_block_is_not_an_error` |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **307 core + 35 tauri** |
| `cargo build --release` / `npm run build` | ✅ | Finished / 零错误 |

**死路径修正**：我定义了 `ApprovalRequested` 却从未发出（于是 live 阶段不会切到「等待审批」）。
实测比对时发现，已在调用裁决器**之前**补发。

**修掉一个我自己造成的间歇性失败（5 连跑已复现）**：`an_unavailable_confinement_refuses_to_launch`
先调一次 `probe()` 决定走哪个分支，而 `AgentSession::connect` 内部会**再探测一次**；并发下
`sandbox-exec` 探测偶发失败，两次结论可能不同，断言随之落空（实测出现 306/1）。
改为使用**必然构造失败**的策略（相对可写根），使该测试与探测结果解耦——无论探测走哪条路，
错误都必然是 `Confinement`。这不是把失败藏起来：改前 3 连跑必现，改后 5 连跑全绿。

**仍未验证**：真实 Tauri 宿主（沿用 mock IPC）；未测超长会话下前端每秒轮询的序列化开销
（现有上限已把单次载荷压到常数级，但未实测）。

---

## PORT-01-N 轮次超时改为「静默」判据并中断（2026-09-10）

**现场报错**（所有者实际运行）：`turn 01a08e46-… did not finish within 300s`，
出现在一次**正常推进**的 UZI 分析上（原文：stage1 网络预检与三波抓取已进行，财务、
估值、事件、龙虎榜已返回，基金持有人仍在继续）。

**根因**：我在命令层写死了 `TURN_TIMEOUT = 300s` 的**总时长**上限。判据选错了——
总时长衡量「跑了多久」，而标记卡死的应是「多久没动静」。深度分析 skill 持续汇报进度
却跑很久是正常形态，用总预算做闸会把正在正常工作的轮次砍掉。

取证：`TurnStartParams` **没有任何 timeout 字段**，运行时不设轮次时限，300 秒纯粹是我的。

**第二个缺陷（更隐蔽）**：超时后我只返回错误、**没有中断**运行时。实测：

```
turn/start while busy      -> OK: {"turn":{"id":"01a08e4b-…"}}   ← 返回的是**正在跑的那个**
turn/interrupt             -> OK: {}
turn/start after interrupt -> OK: {"turn":{"id":"01a08e4c-…"}}   ← 这才是新轮次
```

即：线程留在「进行中」时，下一次 `turn/start` 会被**静默忽略**并回报旧轮次。我们会
`parse` 出旧 turn id，误以为新轮次开始，实际在监听旧轮次的事件——而旧轮次的
`turn/completed` 会提前结束我们的新轮次。

**修复**：
- `TurnLimits` 的 `timeout`（总时长）拆为 **`idle_timeout`（静默）+ `max_duration`（硬顶）**。
  循环每处理完一条消息即刷新 `last_activity`；处理本身（派发工具、等待裁决）也算活动。
- 超时区分两种原因（`TurnTimeoutReason::{Idle, MaxDuration}`），因为二者指向不同的问题：
  静默指向卡死的进程或挂住的调用，硬顶指向预算。
- 超时**先 `turn/interrupt`** 再返回错误，使线程可复用。
- 默认值：静默 600s、硬顶 7200s、审批等待 300s、工具调用上限 32。前两者可用
  `TAOLI_AGENT_TURN_IDLE_SECS` / `TAOLI_AGENT_TURN_MAX_SECS` 覆盖——「多久算合理」
  取决于所有者用的 skill，不该由代码定死。非法值（0、负数、非数字）退回默认并记日志，
  **不会静默变成 0**（那会让每个轮次立即结束）。

**受控对照（同一命令，只改哪一种上限起作用）**

| 上限配置 | 同一条每秒输出一行、持续约 10 秒的命令 | 结果 |
|---|---|---|
| 总预算 4s + 静默 600s（旧形态） | 每 1s 有输出，从不静默 | **MaxDuration 超时，被中断** |
| 静默 3s + 硬顶 600s（新形态） | 同上 | **正常完成**（总时长 5s+ > 静默 3s） |

这条对照即修复的实质：同一条仍在正常工作的轮次，旧判据杀掉它，新判据让它跑完。
测试 `the_same_working_turn_survives_idleness_but_not_a_total_budget` 固化之。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 持续汇报的轮次不被误杀 | ✅ | 上述对照 (b)；`a_silent_turn_is_interrupted_and_says_why` 的第二段 |
| 静默的轮次被中断且说明原因 | ✅ | 对照与 `a_silent_turn_…`：断言 `TurnTimeoutReason::Idle`，消息含 `interrupted` |
| 硬顶仍能终止「一直在报但永不结束」的轮次 | ✅ | `the_ceiling_ends_a_turn_that_is_still_working`（静默 60s、硬顶 6s，只有硬顶能触发） |
| 超时后线程可复用 | ✅ | `a_silent_turn_…` 超时后再跑一轮并成功 |
| 不存在 300s 量级的总预算 | ✅ | `no_total_turn_budget_remains`（硬顶 ≥ 3600s 且 > 静默） |
| 上限可覆盖、非法值退回默认 | ✅ | `the_limits_are_overridable…`、`an_unusable_limit_falls_back_instead_of_becoming_zero` |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **310 core + 38 tauri** |
| `cargo build --release` / `npm run build` | ✅ | Finished / 零错误 |

**顺带修掉自己写的两处**：
1. 循环里第一处 `last_activity` 刷新完全冗余（编译器 `unused_assignments` 抓出：总会被
   处理后的那一次覆盖），已删。
2. 三个超时测试最初用 `DenyAll`，命令**根本没跑**，于是既无静默也无持续输出，全部失败。
   改用「允许一次」的裁决器后才有东西可测——这是测试设计错误，不是实现问题。

**未验证**：未在真实 Tauri 宿主里跑；未实测「模型会不会把长命令缩短」（实测中 `seq 1 84`
的命令只跑到 36 就被模型收尾了，因此 183s 那次不足以证明越过 300s 的情形——改用受控对照
来证明机制，而非依赖一次碰运气的长跑）。

---

## PORT-01-O 分析结果进入对话并按 Markdown 渲染（2026-09-10）

**背景**：所有者指令——「直接将分析结果展示在对话框中。同时对话框中支持 markdown 文档格式。」

**两个真实缺陷，都是先取证后动手**：

1. **一轮里只保留最后一条 agent 消息。** `TurnOutcome.final_message` 每收到一条
   `agentMessage` 就被覆盖。而分析类 skill 会**边跑边汇报**（所有者贴给我的那段
   「当前已完成 stage1 的网络预检…」就是中间消息），最后才给结论——只留最后一条会把
   中间结果全部丢掉，而那正是读者要在对话里看到的内容。
   → 新增 `TurnOutcome.messages: Vec<String>`（全部、有序），DTO 的
   `AgentTurn.final_message` **删除**（`messages` 是其超集，留着就是重复字段）。
2. **结论可能只落在文件里。** skill 把报告写进 `reports/*.md` 后只回一句「已完成」，
   对话里就什么都没有。

**实现**：
- 核心保留全部消息；命令层透传 `turn.messages`。
- 前端新增 `src/markdown.ts` + `MarkdownBlock.vue`，`turns[].messages` 与实时
  `live.message` 都按 Markdown 渲染（否则流式时末尾会露出源码）。
- 领域指令新增第 2 节「结论必须写在回复里」：**不得**只写文件再回「已完成」；结论较长时
  回复至少含核心结论、关键数据（含单位与口径）、数据来源；过程性汇报可另发，但最终必须有
  一条给出结论的消息；中途受阻也要说明卡在哪、已有什么、还缺什么。原 1–5 节顺延为 2–6。

**净化（输入不可信，且这是能触达 Rust 后端的 webview）**：
- `html: false` —— 原始 HTML 被转义而非透传，从源头消除 `<script>`、`<img onerror>`、
  `<iframe>` 这一类注入，无需再对已生成的 HTML 做二次净化。
- markdown-it 自带的链接校验拒绝 `javascript:`/`vbscript:`/`file:` 与多数 `data:`。
- **链接不导航**：未安装外部打开插件，在 webview 内跳转会把应用自身页面替换成远端文档。
  URL 保留在 `title` 里可查看，点击被抑制。
- **`linkify` 关闭**（领域决策，非安全考虑）：实测它把股票代码 `002600.SZ` 自动变成
  `http://002600.SZ`，而本页满屏都是代码。显式 Markdown 链接仍正常。

**实测（渲染断言，含对抗输入）**

| 断言 | 结果 |
|---|---|
| 本轮两条消息都渲染 | ✅ `.markdown-body` × 2（修复前只会显示最后一条） |
| Markdown 结构 | ✅ `h2` × 1、表格 2 行 3 列表头、有序列表 2 项、代码块 1、引用 2 |
| `<script>window.__XSS__=1</script>` | ✅ **未执行**（`__XSS__ = 0`、`scripts = 0`） |
| `<img src=x onerror="...">` | ✅ 未解析（`imgs = 0`） |
| `[恶意链接](javascript:...)` | ✅ 被拒、未成锚点 |
| 正常链接 | ✅ 渲染为 `https://example.com/a` |
| `002600.SZ` 不被误链接 | ✅ 唯一锚点即上面那条 |
| 零横向溢出 / 零页面错误 | ✅ |

**验收**：`cargo fmt/clippy(-D warnings)/test` clean / 0 / **310 core + 38 tauri**；
`npm run build` 零错误。

**未验证**：真实 Tauri 宿主（沿用 mock IPC）；超长 Markdown（当前流式文本上限 4,000 字符，
未测更大文档的渲染耗时）。

**附带说明（必须记录）**：上一轮提交 `e502733` 我用了 `git add -A`，把所有者当时**未提交**
的 WIP 一并提交并推送（`StockAnalysisReport.vue` 1089 行、`stockAnalysis.ts` 195 行，在父提交中
并不存在；`AgentAnalysisPage.vue` 里的双 tab 改动同批）。内容未丢失，但被记在该提交信息下。
**本轮回改为只添加明确路径**，未触碰所有者的在制文件（`src/App.vue`、
`src/components/StockAnalysisReport.vue`、`src/data/stockAnalysis.ts`）。

---

## PORT-01-P 撤掉静态「分析结果」页，结果即 agent 真实输出（2026-09-10）

**背景**：所有者指令——「1 移除分析结果或者隐藏 2 改为真实输出」。

**背景事实**：该页原先是一个双 tab 结构（`分析结果` / `Codex 会话`），其中「分析结果」读的是
`src/data/stockAnalysis.ts` 里的**静态快照**（`mode` 字段自称「Stage 1 部分快照 · Stage 2 未完成」，
多个字段为 `null` 占位）。它与 agent 的真实产出没有任何连接——展示的是写死的数据。

**切替**（所有者选中「移除」，故不留隐藏分支）：
- `AgentAnalysisPage.vue` 去掉 tab 栏、`viewMode` 状态、`StockAnalysisReport` 引入与用法，
  以及那层 `v-else` 包装——页面现在**直接**就是会话流。
- 删除 `src/components/StockAnalysisReport.vue`（1089 行）与 `src/data/stockAnalysis.ts`（195 行）：
  已无消费者，留下即死代码。二者在 `e502733` 的 git 历史中，可恢复。
- `codex-theme.css` 删除随之失效的 `.analysis-view-tabs` 规则（38 行）。

**「真实输出」的落地**：结果就是 agent 在会话里写的消息——`turns[].messages`（PORT-01-O 已改为保留
**全部**消息而非仅最后一条）逐条按 Markdown 渲染，领域指令第 2 节要求结论必须写在回复里。
因此「分析结果」不再是一个独立页面，而是对话本身。

**验收**

| 验收项 | 结果 | 证据 |
|---|---|---|
| 页面无 tab、直接显示会话 | ✅ | 渲染断言 `tabsPresent: false`、`threadVisible: true`；视觉确认「无 tab 控件」 |
| 静态快照数据已被移除 | ✅ | 产物 bundle 中不再含 `分析结果`/`Codex 会话`/`analysis-view-tabs`/`领益智造` |
| 真实输出按 Markdown 完整渲染 | ✅ | `markdownBlocks: 2`、`h1` 命中、`h2 × 4`、表格 3 行、有序列表、代码块、引用 |
| **长报告不被截断**（结论必须可见） | ✅ | 报告末尾标记 `END-OF-REPORT-MARKER` 存在（`lastLineVisible: true`） |
| 零横向溢出 / 零页面错误 | ✅ | `overflow: 0`、无 `pageerror` |
| `cargo fmt/clippy(-D warnings)/test` | ✅ | clean / 0 / **310 core + 38 tauri** |
| `cargo build --release` / `npm run build` | ✅ | Finished / 零错误 |

**修掉一个我自己造成的并发测试缺陷（重要）**：`turn_limit_tests::no_total_turn_budget_remains`
在 `--workspace` 全量并发下**间歇失败**（单跑必过，已实测复现）。两个原因叠加：
1. 该测试读的是**环境变量派生的默认值**，却没有加锁；
2. 我在 `root_env_tests` 与 `turn_limit_tests` 里**各定义了一把锁**——两把锁对同一组进程级
   环境变量提供不了任何互斥，等于没锁。

修法：把锁提到 crate 级的 `mod test_env`，两个模块共用一把；并给这个读默认值的测试补上锁。
**实测 5/5 全量并发通过**（修复前 4 次里复现 1 次）。这不是把失败藏起来，是找到并消除了竞态。

**未验证**：真实 Tauri 宿主（沿用 mock IPC）。

**可能的下一步（未做，等所有者定）**：原先那份报告页是有版式的（KPI 卡片、分区、表格）。现在
结果是 agent 的自由 Markdown，版式由它自己决定。若要恢复**结构化**呈现，协议里有 `outputSchema`
（`TurnStartParams` 的字段），可让 agent 按给定 schema 输出后再由前端渲染固定版式——这是一件新事，
不在本次「移除 + 用真实输出」的范围内。

---

## PORT-01-Q 结构化报告（outputSchema + 校验 + 渲染）（2026-09-10）

**背景**：所有者指令——「需要做」，指恢复报告页的**结构化版式**（KPI 卡片、分区、表格）。

**取证：`outputSchema` 是建议性的，不是强制的（实测三次，结论一致）**

| 条件 | 结果 |
|---|---|
| 只发 `outputSchema`（提示词说「按给定结构输出」） | 模型**拒绝**：*「当前消息中未提供『给定结构』」*，去项目里找结构了 |
| 加 `additionalContext`（`kind: application`）说明要求 | 产出 JSON，但字段名是**它自己编的**（`stock_code`/`company_name`） |
| 再加同样的上下文与更强措辞 | 又一套自创字段（`业务结构_2026H1`/`板块`/`收入_亿元`） |

协议原文只说 `outputSchema` 是 *"JSON Schema used to constrain the final assistant message"*，
但实测在本运行时/模型下它**不构成强制约束**。据此设计，而非假设它有效。

**实现**：
- `docs/agent/report-schema.json`（受版本控制）：完整报告契约。字段大量**可选**——领域指令
  禁止编造数据，取不到的字段应当**缺席**而非填占位值，界面据此显示「未提供」。所有 object 均
  `additionalProperties: false`（有测试遍历断言，开放对象会让契约悄悄变成建议）。
- `crates/core/src/agent/report.rs`：加载（与领域指令同一套向上解析）+ `context_fragment`
  （**从 schema 生成**要求措辞，字段名不可能与 schema 漂移）+ `validate`（浅校验：三个必需字段
  存在且非 null；深校验会重造运行时的 schema 引擎）。
- `turn_start_params` 增 `outputSchema` 与 `additionalContext`（按来源标识分片、`kind: application`）。
- 会话在**最终**消息上：解析 JSON → 校验 → 通过才置 `report`；任一步失败记 `report_error` 并保留正文。
- 命令层把 `report` / `report_error` 透传给界面；schema 在会话启动时加载，缺失/非法即**拒绝启动**
  （与领域指令同一处理）。
- 前端 `ReportView.vue`：按 schema 渲染六个分区（行情 / 财务 / 技术面 / 估值 / 重大事件 /
  数据完整度），可选字段缺席即整块隐藏而非显示 0。有报告时**不再重复**显示承载它的那条 JSON 消息。

**验收（渲染断言）**

| 场景 | 结果 |
|---|---|
| 报告通过校验 | ✅ `reportPresent`、标题「领益智造 002600.SZ」、徽章 `[中性, 评分 54, Stage 1 部分快照]`、六个分区、24 个数据格、敏感性表 2 行、事件 2 条、缺失维度已列出、摘要按 Markdown 渲染（`<strong>`）、**`assistantBlocks: 1`**（JSON 消息未重复） |
| 校验失败（模型自创字段） | ✅ 无报告、显示正文 2 条 + chip「未按结构输出，已回退为正文」 |
| 未要求结构的普通轮次 | ✅ 无报告、chip「本轮无结构化报告」 |
| 零横向溢出 / 零页面错误 | ✅ 三种场景均 0 |
| `cargo fmt/clippy(-D warnings)/test` | ✅ clean / 0 / **323 core + 38 tauri** |
| `npm run build` | ✅ 零错误 |

**端到端实测**：`a_report_turn_returns_structured_output` 用真实运行时 + 真实 schema 跑通，
断言 `report.ticker == "002600.SZ"` 且原始文本同时保留；`a_conversational_turn_yields_no_report`
断言未要求结构的轮次不会凭空产出报告。

**修掉两处自己写出的缺口**：
1. `report_error` 起初**从未被填充**——界面有分支却永不触发，即假实现。已补：校验失败时记录原因。
2. 只覆盖了「产出了 JSON 但不符合 schema」这一种失败；**模型返回纯文本（非 JSON）**这种更常见的
   失败不设 `report_error`，界面只会说「无结构化报告」而不说原因。已补第二条分支。

**未验证**：真实 Tauri 宿主（沿用 mock IPC）；模型**稳定遵循** schema 的比例（实测三次里两次自创
字段，故界面必须假设它可能失败——这正是校验存在的理由）。

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
| 接入被误用为交易路径 | 违反架构文档 §1.2 | 宿主不注册工具 + 环境变量白名单 + 沙箱不授予工作区写；在验收中逐条证明 |
| ~~「始终允许」写入持久规则~~ **已由 K 轮次消除** | — | 沙箱 carve out `<codex_home>/rules`，子进程无法写规则；该动作随之不再提供（实测：被接受但无效果）。剩余风险是**该目录被改回可写**——由 `the_child_policy_reports_that_rule_persistence_is_blocked` 挡住 |
| **审批审计格式变化** | 裁决值由 `allow`/`deny` 变为四值 | 已记入本文档；审计为本地 append-only，无兼容承诺 |
| 与 A-05 影子窗口门禁的关系 | 可能影响当前 `pending` 状态 | **已确认无影响**（见下） |
| **领域指令被抽空或误删** | agent 仍能运行，但失去只读边界与「数字以工具返回为准」等约束，且外观上无法察觉 | 文件缺失/为空即拒绝启动；测试断言 6 条关键表述仍在；trace 记录指令指纹，可比对两个 run 是否同一修订 |
| `data/` 不随仓库分发 | 依赖归档的测试在新克隆上跳过而非失败 | 已在文档标注依赖；这些测试的绿在本机为实证，CI 需先产出归档 |
| **前端只经 mock IPC 验证** | 真实 WKWebView 与真实 IPC 链路（`agent_start` 拉起子进程）未验证 | 仓库既有做法如此（G-02 §2.4）；已在任务文档「未执行的验证」中明确记录，留待所有者在自己实例上确认 |
| **审批超时的界面分支未实测** | 界面在 `source: timeout` 时的呈现未真实触发 | 后端语义有单测覆盖（轮次 C）；界面分支只经 mock 呈现，已记录为缺口 |
| **轮次被判超时后线程仍在使用** | 不中断时下一次 `turn/start` 被静默忽略并回报旧轮次，我们会监听错误的事件流 | 超时路径先 `turn/interrupt` 再报错，并有测试断言超时后可正常开始新轮次 |
| **总时长做闸会砍掉正常工作的长任务** | 深度分析 skill 持续汇报却跑很久是常态 | 判据改为「静默」，硬顶作为兜底；受控对照固化 |
| **每秒轮询的载荷随轮次增长** | 长轮次会产出兆字节文本，而状态每次轮询都要序列化 | 流式文本与单步输出均**只保留尾部**（4,000 字符上限，`long_output_keeps_a_bounded_tail` 盯住）；轮次结束即清空 live 块 |
| **协议无 skill 信号** | 无法诚实标注「本次用了哪个 skill」 | 界面并排呈现可用 skill 清单与调用流，不伪造归属；已写入本文档 |
| **档位「完全访问权限」移除沙箱与审批** | 该档下 agent 可写任意路径、联网且不需批准；若被误选，只读边界与 K 的规则守卫同时失效 | 该档需显式选择且**会话运行中不可切换**；界面在其下方明示「不施加沙箱」；默认档为 `请求批准`；实测三档写入行为逐档核过 |
| **trace 与上游 rollout 的信息重叠** | 有人可能误以为 trace 是上游记录的冗余副本 | 两者互补且以 thread id 为共同键；宿主侧决策（审批来源、被拒请求）仅存在于 trace，已在设计 §3.5 写明 |
| **trace 写入失败被吞** | 丢失审计线索而不自知 | 写入失败以 `tracing::error` 上报，turn 继续（不因 trace 失败而丢轮次）；`session_finished` 仅在 shutdown 成功后记录，故 `finished:true` 可信 |

### A-05 前置检查结论（轮次 B 前置项，已完成）

A-05 处于只读、14 天连续观察窗口运行中（最早评审 2026-09-22T09:52:14Z，手册 §10）。
`crates/core/src/agent/` **不被任何既有代码引用**（`grep 'agent::'` 于 `src-tauri` 与 `crates/core/src`（除 agent 自身）→ 无匹配），是惰性库模块：不启动即不产生进程、不打开文件、不占用端口、不触碰数据库或归档写入路径。
**结论：接入不改变观察循环，也不解除 A-05 门禁。**

## 下一步

轮次 A–G 均 `verified`。PORT-01 的计划轮次已全部完成；发布签收按手册 §5.5 待所有者确认。

**阻塞项**：所有者对设计文档 §0 决策点三（codex 版本锁定策略）的裁定。该决策不阻塞 C 的实现（C 只依赖审批类 server request 的形状，已可从 schema 获取），但应在发布签收前确定。
