//! codex 接入的命令层（PORT-01 轮次 E）：只读分析会话。
//!
//! 与 [`super::session`] 的形态一致：长任务在后台任务里跑，控制器只持有共享
//! 状态与一个命令通道，前端轮询状态。这样审批请求能在界面上呈现，而 `invoke`
//! 不必长时间挂起。
//!
//! 三条边界由本层保证：
//! - **不注册宿主工具**：agent 的能力来自运行时的内置工具与已配置的 skill，
//!   两者产生的副作用动作都要过审批。本层不额外注册任何工具（原
//!   `taoli_shadow_report` 汇报的是套利观察归档，与个股分析无关，已移除）。
//!   也不引用 `order`/`execution`/`account`。
//! - **人工门禁**：审批一律经界面裁决，默认拒绝；超时由 `AgentSession` 侧按
//!   失败关闭处理。
//! - **不可用时失败关闭**：运行时缺失、trace 目录不可写、沙箱不可用、领域指令
//!   缺失都在开跑之前判明，不在中途降级。

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, oneshot};

use personal_taoli_core::agent::{
    AgentSession, TurnContext, TurnLimits,
    app_server::AppServerConfig,
    approval::{ApprovalDecider, ApprovalRequest, Decision},
    discover_program, instructions,
    sandbox::{self, SandboxPolicy},
    tools::ToolRegistry,
    trace::ThreadOptions,
};

use crate::error::{ApiResponse, ErrorCode};

use super::{
    dto::{
        AgentApprovalDecision, AgentApprovalRequest, AgentReady, AgentSkill, AgentStatus,
        AgentToolCall, AgentTurn,
    },
    support::{api_fail, api_map},
};

/// 单轮上限：总时长、工具调用数、审批等待时长。
const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_TOOL_CALLS: usize = 16;

/// trace 落盘目录，**相对于项目根**（`/data/` 已在 `.gitignore` 中）。
///
/// 必须拼到绝对的项目根上再用：沙箱要的是绝对可写根，相对路径会被
/// `sandbox::SandboxPolicy` 直接拒绝（这是刻意的——按某个进程的 cwd 去解释
/// 相对路径，会授权一个调用方从未指名的目录）。
const TRACE_DIR_REL: &str = "data/agent-traces";

/// 额外 skill 目录，取自环境变量（`:` 分隔）。
///
/// 形如 `/path/to/UZI-Skill`。之所以用环境变量而非硬编码：skill 仓库不在本项目
/// 内，其位置因机器而异，硬编码会在别人机器上静默失效——而「skill 没被加载」这
/// 件事本身是无声的，模型只是慢一点、笨一点，不会报错。
///
/// 只接受**绝对路径且实际存在**的目录：运行时拒绝相对根，不存在的根则会让注册
/// 变成一次看起来成功、实际什么都没加载的调用。
const SKILL_ROOTS_ENV: &str = "TAOLI_AGENT_SKILL_ROOTS";
const PATH_SEPARATOR: char = ':';

/// 子进程的额外可写根，取自环境变量（`:` 分隔）。
///
/// 与 `TAOLI_AGENT_SKILL_ROOTS` **正交且刻意分开**：可见性（能读到 skill）与写权限
/// 是两件事，绑在同一个变量上会让「我只是想让它读到这个 skill」顺手变成「我授权
/// 它写这个目录」。
///
/// 为什么需要它：skill 的指令通常是「跑某个脚本」，而脚本会写自己的仓库——本机的
/// UZI-Skill 要写 `.cache/` 与 `reports/`。缺少这项授权时，模型会读完 SKILL.md、
/// 发起命令、拿到批准，然后在**写入时**被沙箱拒绝，表现为一个难以归因的失败。
const WRITE_ROOTS_ENV: &str = "TAOLI_AGENT_WRITE_ROOTS";

fn skill_roots() -> Vec<PathBuf> {
    extra_roots(SKILL_ROOTS_ENV)
}

/// 解析并校验一个「绝对且存在」的根列表（纯函数，便于测试）。
fn parse_roots(raw: &str, variable: &str) -> Vec<PathBuf> {
    raw.split(PATH_SEPARATOR)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .filter(|path| {
            if !path.is_absolute() {
                tracing::warn!(
                    target: "agent",
                    variable,
                    path = %path.display(),
                    "ignoring a relative root: the runtime and the sandbox both require absolute paths"
                );
                return false;
            }
            if !path.is_dir() {
                tracing::warn!(
                    target: "agent",
                    variable,
                    path = %path.display(),
                    "ignoring a root that is not a directory"
                );
                return false;
            }
            true
        })
        .collect()
}

/// 读取环境变量并解析。环境是进程级状态，故测试只覆盖其上的纯函数
/// [`parse_roots`]，外加少量加锁的端到端断言。
fn extra_roots(variable: &str) -> Vec<PathBuf> {
    match std::env::var(variable) {
        Ok(raw) => parse_roots(&raw, variable),
        Err(_) => Vec::new(),
    }
}

fn write_roots() -> Vec<PathBuf> {
    extra_roots(WRITE_ROOTS_ENV)
}

/// 项目根：由领域指令文档的位置确定（就绪检查已要求该文件存在）。
fn project_root() -> Result<PathBuf, anyhow::Error> {
    instructions::project_root().ok_or_else(|| {
        anyhow::anyhow!(
            "找不到项目根（未定位到 {}）",
            personal_taoli_core::agent::instructions::DEFAULT_PATH
        )
    })
}

/// 解析并创建 trace 目录，返回**绝对路径**。
fn trace_dir() -> Result<PathBuf, anyhow::Error> {
    let dir = project_root()?.join(TRACE_DIR_REL);
    std::fs::create_dir_all(&dir)
        .map_err(|error| anyhow::anyhow!("无法创建 trace 目录 {}：{error}", dir.display()))?;
    Ok(dir)
}

/// 默认分析提示词。
///
/// 写死一句话而非让用户从零输入，是因为本页用途固定为「让 agent 分析一只个股」；
/// 用户仍可改写。提示里不再点名任何宿主工具——本层不注册工具。
const DEFAULT_PROMPT: &str = "分析 600519.SH。先说明你打算用哪个 skill、按什么步骤做，再给出结论与依据；数字只能来自你实际读取到的数据，不要编造。";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 后台任务可执行的命令。
enum AgentCommand {
    Ask { prompt: String },
    Stop { done: oneshot::Sender<()> },
}

/// 界面上待裁决的审批，连同其回填通道。
struct PendingApproval {
    request: AgentApprovalRequest,
    reply: oneshot::Sender<Decision>,
    /// 已裁决则由 `agent_decide` 置位，避免重复裁决同一请求。
    decided: bool,
}

/// 把审批请求送到界面并等待裁决。
///
/// 超时**不在这里**处理：`AgentSession::run_turn` 用 `timeout_at` 包住
/// `decide`，因此即使本实现永远不返回也会被判为拒绝。这里只负责把请求呈现给
/// 所有者，并把裁决回填。
struct UiApprovalDecider {
    slot: Arc<Mutex<Option<PendingApproval>>>,
}

impl UiApprovalDecider {
    fn new(slot: Arc<Mutex<Option<PendingApproval>>>) -> Self {
        Self { slot }
    }
}

#[async_trait::async_trait]
impl ApprovalDecider for UiApprovalDecider {
    async fn decide(&self, request: &ApprovalRequest) -> Decision {
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            // 作用域收紧：锁不得跨 await 持有。
            let mut slot = lock(&self.slot);
            if slot.is_some() {
                // 同一时刻只应有一个待裁决请求；出现第二个说明状态机出错，按
                // 失败关闭拒绝，而不是覆盖前一个。
                return Decision::Deny;
            }
            *slot = Some(PendingApproval {
                request: AgentApprovalRequest {
                    request_id: request.id,
                    kind: request.kind.as_str().to_string(),
                    summary: request.summary.clone(),
                    advertised: request.advertised.clone(),
                },
                reply: reply_tx,
                decided: false,
            });
        }

        // 等待界面裁决。通道被丢弃（如停止会话）同样视为拒绝。
        let decision = reply_rx.await.unwrap_or(Decision::Deny);
        lock(&self.slot).take();
        decision
    }
}

struct RunningAgent {
    commands: mpsc::Sender<AgentCommand>,
    slot: Arc<Mutex<Option<PendingApproval>>>,
    status: Arc<Mutex<AgentStatus>>,
}

/// 应用级句柄：同一时刻至多一个分析会话。
pub struct AgentController {
    inner: Mutex<Option<RunningAgent>>,
}

impl Default for AgentController {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

/// 施加于 codex 子进程的沙箱策略。
///
/// 只授予运行时可证必需的可写位置：它自己的 home（会话与状态存储——实测缺少
/// 该授权会启动失败）与 trace 目录。**工作区不在其中**：本接入的边界是只读。
fn codex_confinement(codex_home: &Path, trace_dir: &Path) -> SandboxPolicy {
    let mut policy = sandbox::codex_child_policy(codex_home, Some(trace_dir));
    // 额外可写根：skill 的脚本会写自己的仓库（.cache / reports）。这是显式授权，
    // 由 TAOLI_AGENT_WRITE_ROOTS 提供，不由 skill 根自动推导。
    policy.writable_roots.extend(write_roots());
    policy
}

/// 就绪检查：运行时、trace 目录、沙箱、领域指令四项都必须可用。
///
/// 归档**不再**是就绪前提：需要归档的是已移除的那个套利工具，留下这道门只会让
/// 缺归档时无故拒绝启动。`config_path` 仍保留在签名里，供后续需要配置的能力使用。
fn readiness(config_path: Option<String>) -> AgentReady {
    let _ = config_path;
    let program = discover_program().map(|path| path.to_string_lossy().into_owned());
    // 绝对路径：既用于创建，也用于沙箱可写根。
    let traces = trace_dir().ok();
    let trace_writable = traces.is_some();
    let trace_dir = traces
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());

    // The agent runtime is confined with this project's own macOS Seatbelt
    // profile rather than relying on the runtime's self-imposed sandbox. That
    // confinement is a precondition, not a nicety: if it cannot be applied the
    // session must not start, because starting it would run the runtime with
    // less confinement than the interface claims.
    let confinement = sandbox::probe();

    // The domain constraints are loaded here rather than at turn time so that a
    // missing or empty document is reported as "not ready" instead of being
    // discovered mid-session. Running without them still looks like it works,
    // which is exactly why their absence must block the start.
    let domain_instructions = instructions::load_default();

    let reason = if program.is_none() {
        Some("未找到 codex 可执行文件。请先安装 codex 并确保它在 PATH 上。".to_string())
    } else if !trace_writable {
        Some(format!(
            "无法创建 trace 目录 {TRACE_DIR_REL}（相对项目根）。"
        ))
    } else if !confinement.is_available() {
        Some(format!(
            "沙箱不可用，拒绝在未受限的情况下启动：{confinement}"
        ))
    } else if let Err(error) = &domain_instructions {
        Some(error.to_string())
    } else {
        None
    };

    AgentReady {
        ready: reason.is_none(),
        program,
        trace_dir,
        sandbox: confinement.to_string(),
        instructions: domain_instructions
            .as_ref()
            .ok()
            .map(|text| format!("{} 字节", text.len())),
        skill_roots: skill_roots()
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        write_roots: write_roots()
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        reason,
    }
}

fn stopped_status(program: Option<String>) -> AgentStatus {
    AgentStatus {
        phase: "stopped",
        program,
        thread_id: None,
        trace_path: None,
        pending_approval: None,
        turns: Vec::new(),
        skills: Vec::new(),
        error: None,
    }
}

/// 把一次工具调用记录转成 DTO。
fn tool_call_dto(call: &personal_taoli_core::agent::ToolCallRecord) -> AgentToolCall {
    AgentToolCall {
        call_id: call.call_id.clone(),
        tool: call.tool.clone(),
        success: call.success,
        output: call.output.clone(),
    }
}

/// 把一次审批记录转成 DTO。
fn approval_dto(
    record: &personal_taoli_core::agent::approval::ApprovalRecord,
) -> AgentApprovalDecision {
    use personal_taoli_core::agent::approval::DecisionSource;
    AgentApprovalDecision {
        request_id: record.request_id,
        kind: record.kind.as_str().to_string(),
        summary: record.summary.clone(),
        decision: match record.decision {
            Decision::Allow => "allow".to_string(),
            Decision::Deny => "deny".to_string(),
        },
        source: match record.source {
            DecisionSource::Decider => "decider",
            DecisionSource::Timeout => "timeout",
            DecisionSource::Unsupported => "unsupported",
        }
        .to_string(),
        waited_ms: record.waited_ms,
    }
}

impl AgentController {
    fn snapshot(&self) -> AgentStatus {
        let guard = lock(&self.inner);
        match guard.as_ref() {
            Some(agent) => {
                let mut status = lock(&agent.status).clone();
                // 待裁决请求是共享槽位的真相，不重复存进状态，避免两份副本
                // 不一致。
                status.pending_approval = lock(&agent.slot)
                    .as_ref()
                    .map(|pending| pending.request.clone());
                if status.pending_approval.is_some() && status.phase == "running" {
                    status.phase = "awaiting_approval";
                }
                status
            }
            None => stopped_status(discover_program().map(|p| p.to_string_lossy().into_owned())),
        }
    }

    /// 启动运行时并建立线程。已启动时幂等返回当前状态。
    fn start(&self, config_path: Option<String>) -> Result<AgentStatus, anyhow::Error> {
        let ready = readiness(config_path.clone());
        if !ready.ready {
            anyhow::bail!(ready.reason.unwrap_or_else(|| "agent 未就绪".to_string()));
        }
        let program = ready.program.clone();

        {
            let guard = lock(&self.inner);
            if let Some(agent) = guard.as_ref() {
                // Already running: report the existing session unchanged rather
                // than starting a second one.
                return Ok(lock(&agent.status).clone());
            }
        }

        let (commands_tx, commands_rx) = mpsc::channel::<AgentCommand>(4);
        let slot: Arc<Mutex<Option<PendingApproval>>> = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(AgentStatus {
            phase: "starting",
            program: program.clone(),
            thread_id: None,
            trace_path: None,
            pending_approval: None,
            turns: Vec::new(),
            skills: Vec::new(),
            error: None,
        }));

        let task_status = Arc::clone(&status);
        let task_slot = Arc::clone(&slot);
        tauri::async_runtime::spawn(async move {
            let outcome =
                run_agent(program, commands_rx, Arc::clone(&task_status), task_slot).await;
            let mut guard = lock(&task_status);
            if let Err(error) = outcome {
                guard.phase = "error";
                guard.error = Some(format!("{error:#}"));
            } else if guard.phase != "error" {
                guard.phase = "stopped";
            }
        });

        *lock(&self.inner) = Some(RunningAgent {
            commands: commands_tx,
            slot,
            status: Arc::clone(&status),
        });

        Ok(self.snapshot())
    }

    /// 发起一轮分析。前一状态为 `starting` 时拒绝，避免在未就绪时排队。
    fn ask(&self, prompt: String) -> Result<AgentStatus, anyhow::Error> {
        let guard = lock(&self.inner);
        let agent = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("分析会话尚未启动"))?;
        let current = lock(&agent.status).phase;
        if current == "running" || current == "awaiting_approval" {
            anyhow::bail!("上一轮分析尚未结束");
        }
        if current == "starting" {
            anyhow::bail!("运行时尚未就绪");
        }
        agent
            .commands
            .try_send(AgentCommand::Ask { prompt })
            .map_err(|_| anyhow::anyhow!("分析会话已停止"))?;
        drop(guard);
        Ok(self.snapshot())
    }

    /// 所有者裁决待审批动作。无待裁决项时失败（不猜、不默认放行）。
    fn decide(&self, request_id: i64, allow: bool) -> Result<AgentStatus, anyhow::Error> {
        let guard = lock(&self.inner);
        let agent = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("分析会话尚未启动"))?;
        let decision = {
            let mut slot = lock(&agent.slot);
            let pending = slot
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("当前没有待裁决的审批"))?;
            if pending.request.request_id != request_id {
                anyhow::bail!(
                    "审批 {request_id} 与当前待裁决项 {} 不匹配",
                    pending.request.request_id
                );
            }
            if pending.decided {
                anyhow::bail!("审批 {request_id} 已裁决");
            }
            pending.decided = true;
            if allow {
                Decision::Allow
            } else {
                Decision::Deny
            }
        };
        // 取出的是发送端；发送失败表示等待方已消失（如停止会话），此时裁决自
        // 然失效，按失败上报而非静默吞掉。
        let pending = lock(&agent.slot)
            .take()
            .ok_or_else(|| anyhow::anyhow!("待裁决项已被移除"))?;
        pending
            .reply
            .send(decision)
            .map_err(|_| anyhow::anyhow!("审批等待方已消失，裁决未生效"))?;
        drop(guard);
        Ok(self.snapshot())
    }

    /// 停止会合并等待运行时退出。
    async fn stop(&self) -> AgentStatus {
        let Some(agent) = lock(&self.inner).take() else {
            return stopped_status(discover_program().map(|p| p.to_string_lossy().into_owned()));
        };
        let (done_tx, done_rx) = oneshot::channel();
        if agent
            .commands
            .send(AgentCommand::Stop { done: done_tx })
            .await
            .is_ok()
        {
            // 运行时退出有界：超时则放弃等待，进程组清扫由内核侧保证。
            let _ = tokio::time::timeout(Duration::from_secs(20), done_rx).await;
        }
        stopped_status(discover_program().map(|p| p.to_string_lossy().into_owned()))
    }
}

/// 后台任务主体：拥有 `AgentSession`，串行处理命令。
async fn run_agent(
    program: Option<String>,
    mut commands: mpsc::Receiver<AgentCommand>,
    status: Arc<Mutex<AgentStatus>>,
    slot: Arc<Mutex<Option<PendingApproval>>>,
) -> Result<(), anyhow::Error> {
    let program = program.ok_or_else(|| anyhow::anyhow!("codex 不可用"))?;
    // Writes are granted to the runtime's own home (it stores session and
    // state there) and to the trace directory. The workspace is deliberately
    // absent: this integration's boundary is read-only.
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| anyhow::anyhow!("cannot determine the codex home directory"))?;
    // 绝对路径是硬要求：沙箱拒绝相对可写根，且子进程的 cwd 也应是一个真实目录。
    let root = project_root()?;
    let trace = trace_dir()?;
    let confinement = codex_confinement(&codex_home, &trace);
    // Re-read rather than carrying the readiness result: the document is read
    // once per session, and a failure here must stop the launch.
    let developer_instructions =
        instructions::load_default().map_err(|error| anyhow::anyhow!("{error}"))?;

    let config = AppServerConfig {
        program: PathBuf::from(program),
        request_timeout: Duration::from_secs(60),
        confinement: Some(confinement),
        ..AppServerConfig::default()
    };
    let mut session = AgentSession::connect(config).await?;

    // 本层不注册宿主工具：能力来自运行时内置工具与已配置的 skill，副作用动作
    // 一律过审批。注册表留在这里，是给将来一个作用域正确的工具留位置。
    let tools = Arc::new(ToolRegistry::new());
    let options = ThreadOptions {
        cwd: root.to_string_lossy().into_owned(),
        ephemeral: false,
        developer_instructions: Some(developer_instructions),
        skill_roots: skill_roots(),
        trace_dir: Some(trace),
        ..ThreadOptions::default()
    };
    let thread_id = session.start_thread(&options, Arc::clone(&tools)).await?;
    let trace_path = session
        .trace_path()
        .map(|path| path.to_string_lossy().into_owned());
    {
        let mut guard = lock(&status);
        guard.phase = "ready";
        guard.thread_id = Some(thread_id);
        guard.trace_path = trace_path;
        // 报告运行时**实际**认可的 skill，而非我们请求注册的：注册是否生效由
        // 它说了算。
        guard.skills = session
            .skills()
            .iter()
            .map(|skill| AgentSkill {
                name: skill.name.clone(),
                description: skill.description.clone(),
            })
            .collect();
    }

    while let Some(command) = commands.recv().await {
        match command {
            AgentCommand::Ask { prompt } => {
                // 新轮次在开始时即入列，界面据此立刻显示用户消息与「进行中」
                // 状态，而不是等到结束时才出现。
                let seq = {
                    let mut guard = lock(&status);
                    guard.phase = "running";
                    guard.error = None;
                    guard.pending_approval = None;
                    let seq = guard.turns.len() as u64;
                    guard.turns.push(AgentTurn {
                        seq,
                        prompt: prompt.clone(),
                        tool_calls: Vec::new(),
                        approvals: Vec::new(),
                        refused_requests: Vec::new(),
                        final_message: None,
                        error: None,
                        started_at_ms: now_ms(),
                        finished_at_ms: None,
                    });
                    seq
                };

                let context = TurnContext {
                    tools: Arc::clone(&tools),
                    approvals: Arc::new(UiApprovalDecider::new(Arc::clone(&slot))),
                    audit: None,
                    limits: TurnLimits {
                        timeout: TURN_TIMEOUT,
                        max_tool_calls: MAX_TOOL_CALLS,
                        approval_timeout: APPROVAL_TIMEOUT,
                    },
                };
                let result = session.run_turn(&prompt, context).await;

                let mut guard = lock(&status);
                let (tool_calls, approvals, refused, final_message, error) = match result {
                    Ok(outcome) => (
                        outcome.tool_calls.iter().map(tool_call_dto).collect(),
                        outcome.approvals.iter().map(approval_dto).collect(),
                        outcome.refused_requests.clone(),
                        outcome.final_message.clone(),
                        None,
                    ),
                    Err(error) => (
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        None,
                        // 轮次失败不终止会话：运行时仍在，下一轮可继续；原因
                        // 原样呈现给所有者。
                        Some(format!("{error:#}")),
                    ),
                };

                // 落回本轮记录。seq 由本任务独占递增，因此按 seq 定位比按下标
                // 更稳。
                if let Some(turn) = guard.turns.iter_mut().find(|turn| turn.seq == seq) {
                    turn.tool_calls = tool_calls;
                    turn.approvals = approvals;
                    turn.refused_requests = refused;
                    turn.final_message = final_message;
                    turn.error = error;
                    turn.finished_at_ms = Some(now_ms());
                }
                guard.phase = "ready";
            }
            AgentCommand::Stop { done } => {
                // 丢弃待裁决项：等待方收到通道关闭，按失败关闭拒绝。
                lock(&slot).take();
                let _ = session.shutdown().await;
                let _ = done.send(());
                return Ok(());
            }
        }
    }

    // 命令通道关闭（控制器被丢弃）：仍要收掉子进程。
    lock(&slot).take();
    let _ = session.shutdown().await;
    Ok(())
}

#[tauri::command]
pub fn agent_ready() -> ApiResponse<AgentReady> {
    ApiResponse::ok(readiness(None))
}

#[tauri::command]
pub fn agent_start(app: AppHandle) -> ApiResponse<AgentStatus> {
    let state = app.state::<AgentController>();
    api_map(state.start(None), ErrorCode::InvalidRequest, false)
}

#[tauri::command]
pub fn agent_ask(app: AppHandle, prompt: String) -> ApiResponse<AgentStatus> {
    let state = app.state::<AgentController>();
    if prompt.trim().is_empty() {
        return api_fail(ErrorCode::InvalidRequest, "分析指令不能为空", false);
    }
    api_map(state.ask(prompt), ErrorCode::InvalidRequest, false)
}

#[tauri::command]
pub fn agent_decide(app: AppHandle, request_id: i64, allow: bool) -> ApiResponse<AgentStatus> {
    let state = app.state::<AgentController>();
    api_map(
        state.decide(request_id, allow),
        ErrorCode::InvalidRequest,
        false,
    )
}

#[tauri::command]
pub fn agent_status(app: AppHandle) -> ApiResponse<AgentStatus> {
    ApiResponse::ok(app.state::<AgentController>().snapshot())
}

#[tauri::command]
pub async fn agent_stop(app: AppHandle) -> ApiResponse<AgentStatus> {
    ApiResponse::ok(app.state::<AgentController>().stop().await)
}

/// 默认提示词，供界面首次填充。
#[tauri::command]
pub fn agent_default_prompt() -> ApiResponse<&'static str> {
    ApiResponse::ok(DEFAULT_PROMPT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `cannot build sandbox profile: sandbox root must be absolute`.
    ///
    /// The trace directory used to be assembled as the bare relative string
    /// `data/agent-traces` and handed straight to the profile builder, which
    /// refuses relative roots by design. The failure was correct; the caller was
    /// wrong. This asserts the caller now anchors it to the absolute project
    /// root, and that the confinement the command layer builds therefore
    /// succeeds.
    #[test]
    fn the_trace_directory_is_absolute_and_usable_as_a_sandbox_root() {
        let Ok(dir) = trace_dir() else {
            eprintln!("skipping: project root not resolvable from here");
            return;
        };
        assert!(
            dir.is_absolute(),
            "a sandbox writable root must be absolute, got {}",
            dir.display()
        );
        assert!(
            dir.is_dir(),
            "the directory must have been created: {}",
            dir.display()
        );
        assert!(
            dir.ends_with(TRACE_DIR_REL),
            "unexpected location: {}",
            dir.display()
        );

        // The exact call that failed: the policy the command layer builds must
        // be accepted, not rejected for being relative.
        let codex_home = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".codex");
        let policy = codex_confinement(&codex_home, &dir);
        for root in &policy.writable_roots {
            assert!(
                root.is_absolute(),
                "every writable root must be absolute, got {}",
                root.display()
            );
        }

        // Building the argv is what rejected the relative path before.
        let policy = codex_confinement(&codex_home, &dir);
        let command = vec!["/bin/echo".to_string(), "ok".to_string()];
        match personal_taoli_core::agent::sandbox::confined_argv(&command, &policy) {
            Ok(argv) => assert_eq!(argv[0], "/usr/bin/sandbox-exec"),
            Err(error) => {
                // Only an unavailable sandbox may fail here; it must never be
                // "root must be absolute".
                let rendered = error.to_string();
                assert!(
                    !rendered.contains("must be absolute"),
                    "relative-root rejection came back: {rendered}"
                );
                eprintln!("skipping the argv assertion: {rendered}");
            }
        }
    }

    /// The project root must not be assumed to be the process working
    /// directory: a bundled app launches from elsewhere.
    #[test]
    fn the_project_root_is_derived_from_the_instruction_document() {
        let Ok(root) = project_root() else {
            eprintln!("skipping: project root not resolvable from here");
            return;
        };
        assert!(root.is_absolute(), "{}", root.display());
        assert!(
            root.join(personal_taoli_core::agent::instructions::DEFAULT_PATH)
                .is_file(),
            "the root must contain the instruction document: {}",
            root.display()
        );
    }
}

#[cfg(test)]
mod root_env_tests {
    use super::*;
    use std::sync::Mutex;

    /// Env vars are process-global, so any test that touches them must not run
    /// concurrently with another that does. Serialised explicitly rather than
    /// relying on `--test-threads=1`, which a plain `cargo test` does not pass.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn locked<F: FnOnce()>(body: F) {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        body();
        drop(guard);
    }

    // ── 解析规则（纯函数，无环境依赖）────────────────────────────────

    #[test]
    fn blank_and_empty_inputs_yield_nothing() {
        assert!(parse_roots("", "V").is_empty());
        assert!(parse_roots("   ", "V").is_empty());
        assert!(parse_roots(":::", "V").is_empty());
    }

    #[test]
    fn a_relative_root_is_dropped() {
        // The runtime and the sandbox both reject relative roots, so handing
        // one over would turn a configuration mistake into a failed start.
        assert!(parse_roots("skills/uzi", "V").is_empty());
    }

    #[test]
    fn a_root_that_does_not_exist_is_dropped() {
        // Registering a nonexistent root looks like it worked and loads
        // nothing, which is exactly the silent failure this filter prevents.
        assert!(parse_roots("/nonexistent/uzi-skill", "V").is_empty());
    }

    #[test]
    fn existing_absolute_roots_are_kept_in_order_with_blanks_ignored() {
        let temp = std::env::temp_dir();
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let raw = format!(":{}::{}:", temp.display(), home.display());
        let roots = parse_roots(&raw, "V");
        assert_eq!(roots, vec![temp, home], "order preserved, blanks dropped");
    }

    // ── 环境接线（加锁）──────────────────────────────────────────────

    #[test]
    fn the_env_var_is_read_and_its_validation_applies() {
        locked(|| {
            // SAFETY: guarded by ENV_LOCK; no other test touches this var.
            unsafe { std::env::set_var(SKILL_ROOTS_ENV, "/nonexistent/skill-root") };
            assert!(
                skill_roots().is_empty(),
                "validation must apply to whatever the env supplies"
            );
            unsafe { std::env::remove_var(SKILL_ROOTS_ENV) };
            assert!(
                skill_roots().is_empty(),
                "unset means the runtime's own only"
            );
        });
    }

    /// The pairing this exists for: a skill repository becomes writable, which
    /// is what its scripts need, and the confinement carries it.
    #[test]
    fn an_authorised_write_root_reaches_the_confinement() {
        locked(|| {
            let dir = std::env::temp_dir();
            // SAFETY: guarded by ENV_LOCK.
            unsafe { std::env::set_var(WRITE_ROOTS_ENV, dir.to_string_lossy().into_owned()) };

            let codex_home =
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".codex");
            let trace = dir.join("agent-traces-roots-test");
            let policy = codex_confinement(&codex_home, &trace);

            assert!(
                policy.writable_roots.contains(&dir),
                "the authorised root must reach the policy: {:?}",
                policy.writable_roots
            );
            assert!(policy.writable_roots.contains(&codex_home));
            assert!(policy.writable_roots.contains(&trace));

            unsafe { std::env::remove_var(WRITE_ROOTS_ENV) };
        });
    }

    #[test]
    fn skill_and_write_roots_are_independent() {
        // Reading a skill and letting it write its checkout are separate
        // grants; setting one must not imply the other.
        locked(|| {
            let dir = std::env::temp_dir();
            // SAFETY: guarded by ENV_LOCK.
            unsafe {
                std::env::set_var(SKILL_ROOTS_ENV, dir.to_string_lossy().into_owned());
                std::env::remove_var(WRITE_ROOTS_ENV);
            }
            assert!(!skill_roots().is_empty());
            assert!(
                write_roots().is_empty(),
                "a skill root must not silently grant write access"
            );
            unsafe { std::env::remove_var(SKILL_ROOTS_ENV) };
        });
    }
}

#[cfg(test)]
mod readiness_gate_tests {
    use super::*;

    /// Regression: the archive must not gate readiness.
    ///
    /// It was a prerequisite only because the removed tool read it. Left in
    /// place, the gate would refuse to start a session over a file nothing
    /// reads — and the message would point the operator at the wrong thing.
    #[test]
    fn readiness_does_not_gate_on_the_archive() {
        if discover_program().is_none()
            || !personal_taoli_core::agent::sandbox::probe().is_available()
        {
            eprintln!("skipping: codex or the seatbelt sandbox is unavailable here");
            return;
        }
        let ready = readiness(None);
        assert!(
            ready.ready,
            "readiness must not depend on the archive; reason was {:?}",
            ready.reason
        );
        // And nothing in the reported state should mention it either.
        assert!(
            !ready.reason.unwrap_or_default().contains("归档"),
            "the archive must not appear in the readiness reason"
        );
    }

    /// The host ships no tools, deliberately: the removal was about scope, and
    /// the registry is still the single place a correctly-scoped tool would go.
    #[test]
    fn the_host_registers_no_tools() {
        let registry = ToolRegistry::new();
        assert!(
            registry.is_empty(),
            "expected no host tools, got {:?}",
            registry.names()
        );
        assert!(registry.specs().is_empty(), "nothing may be advertised");
    }
}
