# PORT-01 codex 接入：设计文档

文档状态：**待所有者评审**（2026-09-10）。对应需求：`docs/PORT-01-codex接入-需求.md`。

**本设计的第一性结论**：`codex` 的全量源码移植在工程量上不成立（实测 **1,303,294 行非测试 Rust**，为本项目 Rust 总量的 **64 倍**），且其中绝大部分与本项目的需求无关（TUI 28 万行、Windows 沙箱 2.4 万行等）。**可移植性真正成立的是两条窄路径**：① 进程级集成 `codex app-server`（零行 codex 源码进入本项目）；② 移植 macOS 沙箱子系统（约 1,850–2,300 行，自洽可验证）。两者不是互斥选项。方案选择属所有者决策，见 §0。

---

## 0. 决策记录（所有者前置选择）

本迭代涉及「移植」的含义选择，方向由所有者裁定，不是 AI 自行决定：

| 决策点 | 选项与代价 | 所有者选择 |
|---|---|---|
| 接入方式 | ① **进程集成 `app-server`**（零行 codex 源码进仓；需自研 JSON-RPC 客户端 + 工具桥 + 审批桥；codex 版本升级由上游维护）<br>② **移植 macOS 沙箱子系统**<br>③ **全量 vendor codex 源码**（1,303,294 行进仓，需承接 Bazel→Cargo 迁移与全部上游变更；本项目 Rust 总量将增至 65 倍）<br>④ **① + ②**（先集成拿到 agent 能力，再把 macOS 沙箱下沉为自有组件以摆脱对 codex 沙箱行为的依赖） | **①**（2026-09-10）→ **④**（2026-09-10，F 轮次授权） |
| F 的移植范围 | ① 按字面全量移植（`seatbelt.rs` 1,072 + `.sbpl` 344 + manager + `absolute-path` 767；须同时剥离 `codex_network_proxy` 28,397 行的耦合）<br>② **`.sbpl` 策略文本 + 文件系统策略子集**（约 500–700 行）<br>③ 仅复用 `.sbpl` + 自写最小启动器（约 150–250 行） | **②**（2026-09-10） |
| 运行边界 | ① **只读分析**（符合架构文档 §1.2；能力受限但零风险）<br>② 只读分析 + 生成文档草案（写入限定在 `docs/` 且经审批） | **①**（2026-09-10） |
| codex 版本策略 | ① 锁定已装的 **v0.154.0**（版本可预期，升级需人工评审）<br>② 跟随 latest（协议漂移风险） | **待补**（不阻塞轮次 A：A 不依赖具体版本，仅依赖 `initialize` 形状） |

**代价已在选项中明示**：选项③将使本仓库承担上游 1.3M 行的合并负担，与手册 §2.5「单轮单闭环」及 §2.6「任何产生外部副作用的能力必须先有幂等键…」的收敛原则直接冲突，AI 不建议。

**F 范围决策的依据（实测）**：F 文档原文写「约 1,850–2,300 行、自洽可移植」，实测该前提不成立——`seatbelt.rs` 依赖 `codex_network_proxy`（28,397 行）与 `codex_protocol::permissions`，且其中约 230 行是**逐命令网络代理策略生成器**；而本项目的需求是约束**一个固定子进程**，不需要逐命令语义。故取 ②：保留经实战检验的策略文本与文件系统策略逻辑，丢弃没有调用方的部分。

---

## 1. 精读取证（codex @ main，快照 2026-09-10）

基线：`https://github.com/openai/codex`，codeload tarball `refs/heads/main`，解包于 `/tmp/codex-port/codex-main`。Apache-2.0，`edition = "2024"`，workspace `version = "0.0.0"`（发版时由 `rust-release-prepare` 替换），`codex-rs/rust-toolchain.toml` 声明的 channel 与 `edition` 见该文件。本机 `rustc 1.97.1`。

### 1.1 规模测绘

| 对象 | 文件数 | 行数 |
|---|---:|---:|
| `codex-rs/**/*.rs`（**不含 `tests/` 目录，仍含 `*_tests.rs`**） | 4,018 | **1,303,294** |
| `codex-rs/**/*.rs`（全部，含 `tests/`） | 4,018 | 1,706,715 |
| 对照：本项目 `crates/core` | 46 | 18,836 |
| 对照：本项目 `src-tauri` | — | 1,447 |
| 对照：本项目前端 `src/**` | — | 4,983 |

**倍数关系**：codex-rs 非测试代码是本项目 Rust 总量（20,283 行）的 **64 倍**。

Top 15 crate（`tests/` 目录外，含 `*_tests.rs`）：

| crate | 行数 | crate | 行数 |
|---|---:|---|---:|
| `tui` | 280,818 | `cli` | 27,737 |
| `core` | 221,595 | `config` | 26,677 |
| `app-server` | 56,513 | `windows-sandbox-rs` | 24,050 |
| `core-plugins` | 45,560 | `state` | 23,989 |
| `exec-server` | 40,211 | `rmcp-client` | 23,885 |
| `app-server-protocol` | 34,314 | `codex-mcp` | 23,007 |
| `thread-store` | 33,319 | `rollout` | 16,038 |
| `protocol` | 29,589 | `sandboxing` | 9,343 |

**Bazel 不是障碍**：根 `BUILD.bazel` 仅 26 行，各 crate 的 `BUILD.bazel` 是 6–18 行的 `codex_rust_crate` 宏包装（如 `rollout/BUILD.bazel` 6 行）。Cargo workspace 是自洽的，无需 Bazel 即可 `cargo build`。

### 1.2 逐模块可移植性判定

| 模块 | 规模（行） | 判定 | 耦合点 / 证据 |
|---|---:|---|---|
| **`protocol`** | 29,589（prod 18,772；68 个 `.rs`） | **不可移植** | `Submission`/`Op`/`Event`/`EventMsg` **是纯进程内类型**：仅 `derive(Debug)`，**无 serde**（`protocol/src/protocol.rs:190-205`、`:596-772`、`:1344-1351`、`:1358-1580`）。无法用于跨进程。且硬依赖 **13 个 workspace 内 crate 共 47,414 行**，其中 `network-proxy` 28,397 + `http-client` 9,033 为大头。移植 protocol 实际要拖入 ~77,000 行 |
| **`core`** | 222,921（500 文件；非测试 113,325） | **不可移植** | `submission_loop` 在 `core/src/session/handlers.rs:538`；会话主循环与 config/tools/guardian/context_manager 深度交织。抽离「最小 agent loop」需连带 `config`(26,677)+`tools`(7,035)+`protocol`(29,589)+`sandboxing`(9,343)+`state`/`rollout`(40,027) 等，实测下限远大于本项目整体规模 |
| **`sandboxing`（macOS 部分）** | **约 1,850–2,300** | **✅ 可移植** | macOS 侧 = `sandboxing/src/seatbelt.rs`（1,075 行）+ 4 个 `.sbpl`（344 行）+ `manager.rs` 分派（约 40 行）+ `utils/absolute-path`（767 行）。依赖仅 `regex-lite`/`serde`/`url`(可选)/`tracing`(可选)。**本仓唯一小而自洽的可移植子系统** |
| `sandboxing`（Linux/Windows 部分） | 9,343 − macOS 部分 | 不移植 | Linux 依赖外部 `bwrap`（系统或在 `codex-resources/` 捆绑并经 SHA-256 校验，校验失败退出码 8）；Windows 24,050 行 + 微软 mxc git 依赖 |
| **`rmcp-client` / `codex-mcp`** | 23,885 / 23,007 | **无需移植** | `rmcp` 是 **crates.io 依赖 `=3.2.0`**（`codex-rs/Cargo.toml:430`，仓库内无 `[patch]`）。本项目若要 MCP 能力，**直接 `cargo add rmcp` 即可**，不必碰 codex 代码 |
| `rollout` / `history` / `state` / `thread-store` | 16,038 / 2,741 / 23,989 / 33,319 | 不移植 | 落盘格式为 JSONL，路径 `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<thread_id>.jsonl`，12 个 wire 变体；`RolloutLine`/`ResumedHistory`/`InitialHistory` 在 `history/src/lib.rs:262/271/278`。**格式可读即可集成，无需移植代码** |
| `tui` | 280,818 | 不移植 | 本迭代明确不做（需求 R07） |
| `v8` / `code-mode-runtime` | — | 不移植 | 仅 `v8-poc`、`code-mode-runtime` 直接依赖 `v8`；与需求无关 |

### 1.3 关键发现：`app-server` 是被低估的完整能力面（**本设计的技术核心**）

精读发现两条进程级接口，能力差异是**决定性的**：

| 接口 | 事件/方法面 | 审批 | 自定义工具 | 上下文控制 | 结论 |
|---|---|---|---|---|---|
| **`codex exec --json`** | 8 类顶层事件 + 9 类 item | ❌ **主动拒绝** | ❌ | 仅 prompt | 只适合「一次性问答」，**不满足 PORT01-R01/R02** |
| **`codex app-server`** | **99 个 client request 变体 / 193 个定义**（非 experimental） | ✅ 可自定义 | ✅ **可注入** | ✅ 可注入 | **满足全部需求** |

**证据（本机实测，非文档推测）**：

1. `codex exec --json` 实测输出（`codex-cli 0.154.0`，本机）：
   ```jsonl
   {"type":"thread.started","thread_id":"01a08b8e-5df7-7322-a2f2-342096f85623"}
   {"type":"turn.started"}
   {"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"pong"}}
   {"type":"turn.completed","usage":{"input_tokens":8105,"cached_input_tokens":7040,"output_tokens":3}}
   ```
   同时精读确认：`exec` 对**全部** server request 直接回错误（`exec/src/lib.rs:1976-2117`），涵盖命令审批、文件变更审批、权限审批、`request_user_input` 与**动态工具调用**。故 `exec` 无法承载 R01/R02。

2. `codex app-server generate-json-schema` 实测产出 **37 个 schema 文件（4.2 MB）**，其中 `ClientRequest.json` = **99 个 `oneOf` 变体 + 193 个定义**。方法面覆盖（部分）：
   `Thread/start`、`Thread/resume`、`Thread/fork`、`Thread/compact/start`、`Turn/start`、`Turn/steer`、`Turn/interrupt`、`Fs/readFile`、`Fs/writeFile`、`Fs/readDirectory`、`Command/exec`、`McpServer/tool/call`、`McpServer/resource/read`、`PermissionProfile/list`、`Skills/list`、`Hooks/list`、`Config/read`、`Config/value/write`、`Review/start`、`Model/list`、`Account/read`。

3. 传输方式（`codex app-server --help` 实测）：`--listen stdio://`（默认）、`unix://`、`unix://PATH`、`ws://IP:PORT`、`off`。

4. **`dynamicTools` 受 `experimentalApi` 能力门控（实测，非文档推测）。**
   不带该能力时，运行时明确拒绝：
   ```json
   {"error":{"code":-32600,"message":"thread/start.dynamicTools requires experimentalApi capability"},"id":2}
   ```
   在 `initialize` 声明 `capabilities: {"experimentalApi": true}` 后 `thread/start` 成功。
   源码佐证：`app-server-protocol/src/protocol/v2/thread.rs:138` 的 `#[experimental("thread/start.dynamicTools")]`。
   → 该字段**不在**默认 schema 输出中（默认 99 个 `ClientRequest` 变体），只在 `--experimental` 输出中（**159** 个变体）。本设计 §1.3 前文的「99」是默认口径，实现按 experimental 口径对接。

5. **自定义审批**：`CommandExecutionApprovalDecision` 等类型（`app-server-protocol/src/protocol/v2/item.rs:64-83`）——即 PORT01-R02 的人机门禁可直接落在协议层。

6. **动态工具注入**：`thread/start` 的 `dynamicTools` 字段 + `item/tool/call` 服务端请求（`app-server-protocol/src/protocol/v2/thread.rs:138-142`、`v2/item.rs:1642-1657`）——即 PORT01-R01 的「调用本项目自有工具」无需移植任何 codex 代码。

7. **上下文控制**：`base_instructions` / `developer_instructions` / `additional_context`——即 PORT01-R05 的领域指令注入。
   **实测选择 `developerInstructions`**：它在提示词中作为独立片段追加（`core/src/session/mod.rs:4090` 的 `DeveloperInstructions::new(..).render_fragment()`），而 `baseInstructions` 是整体替换基础提示词。领域约束属追加，不应覆盖运行时自身操作指令。二者均**未受 `experimentalApi` 门控**（实测：`thread/start` 接受该字段，且模型按注入规则作答）。

> **注意**：方法数存在口径差异。实测非 experimental schema 为 **99 个 `ClientRequest` 变体**；另一路取证报告给出 164（含 experimental 字段）。本设计**以实测的 99 为准**，并在实现阶段用 `--experimental` 复核。

### 1.4 macOS 沙箱可用性（本机实测）

| 项 | 结果 |
|---|---|
| `/usr/bin/sandbox-exec` | **存在**（102,368 B，`root:wheel`） |
| 本机系统 | macOS **26.6.2**（build 25G83），arm64 |
| 基本可用性 | `sandbox-exec -p '(version 1)(allow default)' /usr/bin/true` → **exit 0** |
| 上游风险信号 | 该二进制内含字符串 **"The %s profile is deprecated and may not be secure"** |
| 嵌套限制 | 在已受沙箱的进程中再套 Seatbelt → `sandbox-exec: sandbox_apply: Operation not permitted`；codex 对此仅跳过相关测试，且 `get_platform_sandbox()` **不探测可用性**（`sandboxing/src/manager.rs:67-81`） |
| macOS 15.x | **未实测，不作断言** |

**对本设计的影响**：macOS 沙箱目前可用，但上游已标记 deprecated，且 codex 自身不做可用性探测。若选方案②/④，**必须自建可用性探测并按失败关闭处理**（需求 PORT01-R03 的失败场景行）。

---

## 2. 方案对比

| 维度 | ① app-server 集成 | ② 移植 macOS 沙箱 | ③ 全量 vendor |
|---|---|---|---|
| 进仓 codex 代码 | **0 行** | ~1,850–2,300 行 | 1,303,294 行 |
| 得到的 agent 能力 | **完整**（工具注入/审批/上下文） | 无（只是沙箱） | 完整 |
| 满足 R01/R02 | ✅ | ❌ | ✅ |
| 满足 R03/R04 | 由宿主实现 | 部分 | 由宿主实现 |
| 上游变更负担 | 协议级（可选版本锁定） | 沙箱策略维护 | **全量合并 1.3M 行** |
| 与 §2.5 单轮单闭环 | 兼容 | 兼容 | **冲突** |
| 可独立验证 | ✅（协议 + 实测事件） | ✅（沙箱拒绝写入可测） | 极难 |
| 主要风险 | 协议漂移、`app-server` 标注 `[experimental]` | sandbox-exec 已 deprecated | 长期不可维护 |

**推荐：①**（若需摆脱对 codex 沙箱行为的依赖，再叠加 ②，即方案 ④）。推荐理由是可验证的事实而非偏好：`app-server` 暴露的 `dynamicTools` + 自定义审批 + 上下文注入**恰好逐条对应** PORT01-R01/R02/R05，而这三条恰是「移植源码」才能达成的能力——即**不必移植也能达成**。

**不推荐 ③ 的理由**：1.3M 行 > 本项目 64 倍；与手册 §2.5 收敛原则冲突；且其中 28 万行 TUI、2.4 万行 Windows 沙箱对本项目零价值。

---

## 3. 推荐方案的接口设计（若选 ①）

### 3.1 模块边界（新增，不修改既有）

```
crates/core/src/agent/          ← 新增子模块（只读边界内）
  mod.rs                        ← 对外：AgentSession / TurnContext / TurnOutcome
  app_server.rs                 ← codex app-server 子进程 + JSON-RPC 客户端（stdio）
  protocol.rs                   ← JSON-RPC 信封、能力协商、thread/turn 参数
  tools.rs                      ← 工具机制：spec/注册表/唯一路由点
  approval.rs                   ← 审批归类、裁决、审计日志
  trace.rs                      ← 会话 trace：append-only 落盘 + 结构校验回放
  instructions.rs               ← 领域指令加载（内容来自仓库文件，缺失/为空即失败关闭）
  access.rs                     ← 操作档位：sandbox + approvalPolicy + reviewer + 自有沙箱形状
  progress.rs                   ← 把 80 余种通知收敛为一小组 TurnProgress 事件
  sandbox/                      ← macOS Seatbelt 约束（策略文本上游逐字 + 文件系统策略子集）
    mod.rs                      ← 可用性探测（上游没有）+ 失败关闭的 argv 包装
    seatbelt.rs                 ← 移植：可写根归一化、访问策略、-D 参数装配
    policies/*.sbpl             ← 上游逐字拷贝（Apache-2.0，见 PROVENANCE.md）
src-tauri/src/commands/agent.rs ← 已实现：AgentController + 7 个命令（复用 ApiResponse<T>）
src/components/AgentAnalysisPage.vue ← 已实现：对齐 Codex 桌面版的会话流页面
src/codex-theme.css            ← Codex 桌面版令牌（取自其 webview 实际样式表，作用域 .codex-scope）
docs/agent/instructions.md      ← 已实现：领域约束文档（受版本控制，注入 developerInstructions）
```

**边界不变式**：
- `crates/core/src/agent/` **不得** `use` 任何 `order`/`execution`/`account` 的写路径；只读扫描/对账/仿真查询结果。
- 子进程环境变量**白名单**构造（`env_clear()` 后按需注入），确保不含交易密钥。
- 既有 `crates/core` 公开 API、迁移文件、前端 `commands.ts` 契约**逐字不变**（不改既有命令）。

### 3.2 已实现的方法子集（轮次 A/B/C，方法名与载荷形状均已实测）

| 方法 | 方向 | 用途 | 需求映射 |
|---|---|---|---|
| `initialize` | client → server | 握手；**必须声明 `capabilities.experimentalApi = true`**，否则 `dynamicTools` 被拒 | R01 |
| `thread/start` | client → server | 建会话：`cwd` / `ephemeral` / `sandbox` / **`approvalPolicy`** / **`dynamicTools`** | R01/R02/R05 |
| `turn/start` | client → server | 发起一轮：`{threadId, input:[{type:"text",text}]}` | R01 |
| `item/tool/call` | **server → client** | 宿主执行自有工具并回结果 | R01 |
| `item/commandExecution/requestApproval` | **server → client** | 命令执行审批 → 宿主裁决 | R02 |
| `item/fileChange/requestApproval` | **server → client** | 文件变更审批 → 宿主裁决 | R02 |
| `item/permissions/requestApproval` | **server → client** | 权限扩张审批 → 宿主裁决 | R02 |
| `execCommandApproval` / `applyPatchApproval` | **server → client** | 上一代审批方法（词汇不同） | R02 |
| 其余 server request | server → client | **不实现，一律回 JSON-RPC error（失败关闭）**，并在 `TurnOutcome::refused_requests` 中记录 | — |
| 通知流 | server → client | `turn/completed` / `turn/failed` / `item/completed`（取 `agentMessage.text`）等 | R04 |

**沙箱与审批策略是两道独立的闸（实测结论）**：仅设 `sandbox: read-only` 时，不写盘的命令（如 `echo`）**不产生审批**，且默认 `approvalPolicy = on-request` 由模型自行决定是否询问。要让每个副作用动作都过人的门，必须由宿主显式设 `approvalPolicy = "untrusted"`。这不是可选项，是 R02 成立的前提。

**实测报文形状**（`crates/core` 原件，非文档推断）：

```jsonc
// server → client（请求）
{"id":0,"method":"item/tool/call","params":{
  "arguments":{},"callId":"call_00_Umb9cR7aB7h58tLwxcPS4804","namespace":null,
  "threadId":"01a08ba4-...","tool":"taoli_shadow_report","turnId":"01a08ba4-..."}}

// client → server（应答；DynamicToolCallResponse）
{"id":0,"result":{"contentItems":[{"type":"inputText","text":"{...}"}],"success":true}}
```

分帧为**换行分隔 JSON**（`app-server-transport/src/transport/stdio.rs`：stdin 逐行读、stdout 每条追加 `\n`）。

### 3.3 工具集（轮次 B 实现，轮次 I 清空）

**当前宿主不注册任何自定义工具。** agent 的能力来自运行时的内置工具与已配置的
skill，两者产生的副作用动作都要过审批。

原本注册的 `taoli_shadow_report` 汇报**跨交易所套利观察归档**，与个股分析无关，
已移除（见任务文档 PORT-01-I）。`tools.rs` 的机制与唯一路由点保留：将来一个作用域
正确的工具（如行情快照、财报读取）注册到同一处即可，无需改动会话循环。

**未注册工具的处置**：在 `ToolRegistry::dispatch` 这唯一路由点拒绝，回 `success:false` 并把可用工具名一并告知模型；**无任何回退路径**（不会静默改用 codex 内置工具）。

**注册表一致性**：`start_thread` 广告 `tools.specs()`，`run_turn` 用传入的注册表分发。二者若不一致（广告了但分发不到），调用会按上一条**失败关闭**——这是刻意的：不一致必须是可观测的拒绝，而不是静默的替代。验收测试即以此构造验证该路径跨线缆生效。

**尚未提供**：下单、撤单、划转、改配置、写库类工具一律不存在（R03）。

### 3.3b 会话状态契约（含 E 修订）

`AgentStatus` 携带 **`turns: Vec<AgentTurn>`**（按时间升序，每轮含 prompt / 工具调用 / 审批 / 被拒请求 / 结论 / 错误 / 起止时间）。

**为何是累积而非「最近一轮」**：界面按 Codex 桌面版呈现为会话流，只保留最近一轮会导致新一轮抹掉上一轮。这是一次**干净切替**——旧的扁平字段已删除，无兼容层。

**视觉作用域**：Codex 令牌（蓝主色、superellipse 圆角、16px 聊天气泡字号）仅作用于 `.codex-scope`，不覆盖本项目 UI-01 的全局令牌。代价是本页主色与其余页面不同；统一需另立一轮。

### 3.3e 轮次上限（轮次 N）

**判据是静默，不是时长**。`TurnLimits` 给出 `idle_timeout`（多久没事件算卡死）与
`max_duration`（兜底硬顶）。理由来自一次真实失败：UZI 的 stage1 抓取持续汇报进度却远超
五分钟，而原先的 300 秒**总预算**把它砍在半途。总时长衡量「跑了多久」，卡死应看「多久
没动静」。

**处理本身不是空转**：每处理完一条消息即刷新 `last_activity`，因此派发工具、等待所有者
裁决所花的时间不计入静默——否则一个慢工具刚干完活，下一轮就会因截止时刻已过而被判卡死。

**超时先中断**：实测，线程留在「进行中」时下一次 `turn/start` 会被静默忽略并回报**旧**
轮次；不中断就继续用该线程，会监听错误的事件流。故超时路径先发 `turn/interrupt`（尽力而为，
失败也不覆盖超时这一原因），再返回带原因的 `TurnTimeout`。

### 3.3d 流式进度（轮次 M）

**分层**：`progress.rs` 只做**词汇归一**（80 余种通知 → 十来个事件，增量按 chunk 转发，本层无状态）；
会话把它转发给 `TurnContext.progress: Option<ProgressSink>`；命令层的 `agent_live::LiveTracker`
把事件折叠进共享状态的 `live` 块；界面轮询该块。

**为什么在命令层折叠而不是让界面自己折叠**：`live` 是锁保护下的共享状态，折叠必须在那把锁
之下完成；集中在一处才能避免「界面看到的」与「记录里的」出现两套说法。

**三条边界**：
- 流式文本与单步输出**各只保留尾部**（4,000 字符）——状态每次轮询都要序列化，长轮次可产出兆字节。
- `live` 带 `turn_seq`，**上一轮的迟到事件不改写本轮**。
- 轮次结束后**清空** `live`：届时权威数据在 `turns`，留着会重复渲染。

**锁的选择**：与命令层其余部分一致用 `std::sync::Mutex`（本项目自有代码 9 处、`parking_lot` 0 处，
且本模块与 `agent.rs` 共享同一把 `Arc<Mutex<AgentStatus>>`），并以 `PoisonError::into_inner`
容忍中毒——折叠进度时 panic 不应让界面连状态都读不到，那正是排查所需的线索。

### 3.3c 操作档位（轮次 L）

档位与 Codex 桌面版权限下拉同构，且**是单位而非三个旋钮**：一档同时决定运行时的 `sandbox`、`approvalPolicy`、`approvalsReviewer` 与本项目自己的 Seatbelt 形状。暴露三个独立旋钮会允许 app 不提供、本项目也未推理过的组合。

| 档位 | 运行时 | 自有沙箱 | 本页审批卡 |
|---|---|---|---|
| 请求批准 | `read-only` / `untrusted` / `user` | 运行时可写目录仅 home + trace + 显式写根 | 每个副作用动作都过 |
| 帮我批准 | `workspace-write` / `on-request` / `auto_review` | 追加工作区 | **不出现**——运行时自审 |
| 完全访问权限 | `danger-full-access` / `never` / `user` | **无** | **不出现**——不请求批准 |

**为什么 `FullAccess` 不施加约束**：该档的说明是「可不受限制地…」。施加任何 profile 都会让它变成假话；诚实的实现就是不约束，并在界面明示。

**为什么 `Ask` 用 `untrusted` 而非运行时的默认 `on-request`**：`on-request` 由模型自行决定是否询问——实测会不询问就执行无副作用外观的命令。所有者要当那道闸，就不能把闸交给模型判断。

### 3.4 审批门禁与审计（轮次 C 已实现）

**裁决模型与 Codex 桌面版一致**（轮次 J，所有者指令）：`Decision::{AllowOnce, AllowForSession, AllowAlways, Deny}`，对应审批卡的 允许一次 / 允许此对话 / 始终允许 / 拒绝。

可用集合由 `available_decisions(kind, params)` 依**请求本身**算出，而不是照抄运行时的 `availableDecisions`（可选、实测不含 `decline`）：
- 「始终允许」仅在运行时给出具体规则提议（`proposedExecpolicyAmendment`）时提供——记不住东西的永久授权是假的；
- 文件变更与权限扩张在协议里没有永久授权词汇，故不提供；
- 「拒绝」恒在末位，确保操作员总能否决。

**「始终允许」在本项目内不可用**（轮次 K）：实测它会写入 `~/.codex/rules/default.rules`，此后同类命令在该机器上**所有** codex 会话都不再询问——超出本项目的沙箱边界。因此沙箱 carve out `<codex_home>/rules`（`deny_write_subpaths`，排在所有 allow 之后），而 `available_decisions` 在 `Persistence::Blocked` 时不再提供该动作。

**为什么不只是禁用按钮、而要在沙箱层阻断**：只禁按钮，运行时的持久写入仍然存在（例如它在别处自行写规则）；只在沙箱阻断而保留按钮，则会得到一个「被接受然后无效果」的假承诺（实测：下一条同样的命令仍被拦住）。两层都要，缺一不可。二者由 `blocks_rule_persistence()` 与 `SandboxPolicy` 绑定，并有测试盯住漂移。

**超时与未接线的默认仍是拒绝**（`DenyAll`）：问不到人时不得假定同意。

**超时的保证位置**：截止时间由**会话**施加（`tokio::time::timeout_at` 包住 `decider.decide`），不是在 decider 内部。因此「实现者忘了处理超时」不会让门变成常开——未按时返回裁决 ⇒ 记为 `DecisionSource::Timeout` 且 `Decision::Deny`。

**默认拒绝**：未接入所有者通道时使用 `DenyAll`。这是「未接线」不会意外变成「宽松」的原因。

**审计**：`ApprovalLog` 为 append-only JSONL（不可原地改写——可改写的审计不构成证据）。记录含 `request_id / method / kind / summary / decision / source / decided_at_ms / waited_ms / advertised`。`advertised` 保存上游当时提供的裁决项，使协议词汇变化可被事后察觉。读取时**遇到无法解析的行即失败**，不静默跳过，否则证据不可靠。

**代际词汇**：v2 `item/…` 用 `accept`/`decline`；v1 `execCommandApproval`/`applyPatchApproval` 用 `approved`/`denied`。拒绝取最窄表达（`decline` 对应 `ReviewDecision::Denied`，而非 `cancel` 对应的 `Abort`）。实测：`availableDecisions` 可能不含 `decline`，但回 `decline` 被接受。

**路径**：审计文件路径由调用方给定；本迭代建议置于 `.gitignore` 覆盖的 `data/` 下。

### 3.5 会话 trace 与回放（轮次 D 已实现）

**宿主记录而非上游副本**：非 ephemeral 线程的上游 rollout（`~/.codex/sessions/…/rollout-<ts>-<thread_id>.jsonl`）与 `thread/read` / `thread/items/list` 都存在（实测），但**宿主侧决策不在其中**——某次审批是「所有者拒绝」还是「超时被迫拒绝」，以及哪些 server request 因未实现被拒，只存在于宿主 trace。故两者互补，以 **thread id** 为共同键。

**记录粒度**：只记宿主事实——`session_started` / `turn_started` / `tool_call`（含入参与逐字出参）/ `approval`（复用审计记录，单一真相源）/ `refused_request` / `item`（仅 `agent_message`，不记 delta 噪声）/ `turn_finished` / `session_finished`。

**三条不变式**：
- 每个 turn **恰好一条** `turn_finished`（`run_turn` 拆出 `drive_turn` 后由外层统一记录，覆盖全部错误路径）。
- 回放做**结构校验**：乱序报错（含行号）而非重排；落在自己 turn 结束之后的事件被拒绝，否则会被静默归到下一轮。
- **逐事件 flush**：被 kill 的会话必须留下崩溃前的事件。

**路径**：`<trace_dir>/<thread_id>.jsonl`，即「按 thread id 回放」的字面实现。

### 3.5 进程监管（实现中发现，非设计预判）

实现 PORT-01-A 时发现两个约束，均由**本机实测**得到，且都会导致「孤儿 agent 进程」这一真实故障：

1. **`codex` 可能是 shim，真正的 runtime 是孙进程。** 本机 `~/.bun/bin/codex` 是一个 Node 脚本（`#!/usr/bin/env node`），它再 `spawn` 真实二进制
   `~/.bun/install/global/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`。
   实测进程树：`node(shim) → codex(native) → git`。
   → 只对直接子进程发信号（`kill_on_drop` 的默认语义）会**留下真正在跑的 agent runtime 并继续持有管道**。

2. **优雅关闭不会回收孙进程。** 关闭 stdin 后直接子进程正常退出，但 codex 在启动期派生的 `git`（拉取其插件市场
   `https://github.com/openai/plugins.git`）**不在其回收范围内**，仍留在原进程组里。

**处置（已实现）**：
- 子进程以 `process_group(0)` 启动，独占一个进程组（实测 pgid == 子进程 pid，与宿主组分离）。
- 进程组 id 在 spawn 时捕获并保存——因为 `Child::id()` 在直接子进程被 reap 后返回 `None`，而清扫必须在那之后进行。
- `shutdown()` 在优雅等待**之后无条件清扫进程组**，覆盖孙进程；`Drop` 亦做 best-effort 清扫。
- 可验证不变量：`shutdown_leaves_no_process_in_the_agent_group` 以 `ps -g <pgid>` 断言组内无残留（按组成员判定，
  不会因机器上存在无关 codex 而误判通过）。

> 说明：`kill` 工具在「部分目标已消失」时返回非零，属预期行为，不作为失败上报（其 stderr 已被静音），实际不变量由上述测试断言。

---

## 4. 不做什么

- 不移植 codex 源码（任何形式）。
- 不引入 codex 的 TUI / V8 code-mode / Windows / Linux 沙箱。
- 不修改 `crates/core` 的交易、账务、对账、订单路径。
- 不自动放行任何审批；不实现 agent 自进化循环。

---

## 5. 可复现验证命令（本设计的取证手段，供复核）

```bash
# 1) 规模与可移植性（不依赖网络缓存）
curl -sL https://codeload.github.com/openai/codex/tar.gz/refs/heads/main | tar -xz
find codex-main/codex-rs -name '*.rs' -not -path '*/tests/*' -print0 | xargs -0 wc -l | tail -1

# 2) 协议面（需本机装有 codex）
codex app-server generate-json-schema --out ./schema
jq '.oneOf | length, (.definitions | length)' ./schema/ClientRequest.json

# 3) exec 事件流（实测 4 行 JSONL）
codex exec --json --skip-git-repo-check -s read-only "Reply with one word"

# 4) macOS 沙箱可用性
ls -l /usr/bin/sandbox-exec
/usr/bin/sandbox-exec -p '(version 1)(allow default)' /usr/bin/true; echo "exit=$?"
strings /usr/bin/sandbox-exec | grep -i deprecated
```

---

## 6. 待补（后续轮次）

轮次 A–G 已 `verified`（见任务文档）。尚未落地的：

- codex 版本锁定策略（§0 决策点三，`待补`；不阻塞实现，发布签收前需定）。
- 会话全量落盘与按 thread id 回放——**属轮次 D**。
- 所有者交互界面的**真实宿主验证**（mock IPC 已覆盖渲染与交互，未在真实 WKWebView 中跑）——**属所有者侧确认**。
- 领域指令注入的**真实宿主端到端实跑**（核心层已实测，界面链路未跑）——**属所有者侧确认**。
- 沙箱策略与工作目录白名单细节——**本轮不涉及**；`/usr/bin/sandbox-exec` 当前可用，但其可用性探测与失败关闭属方案 ②/④ 范围。
