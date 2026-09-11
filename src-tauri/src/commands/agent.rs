//! codex 接入的命令层（PORT-01 轮次 E）：只读分析会话。
//!
//! 与 [`super::session`] 的形态一致：长任务在后台任务里跑，控制器只持有共享
//! 状态与一个命令通道，前端轮询状态。这样审批请求能在界面上呈现，而 `invoke`
//! 不必长时间挂起。
//!
//! 三条边界由本层保证：
//! - **只读**：只注册 `taoli_shadow_report`（读归档）。不提供任何写操作工具，
//!   也不引用 `order`/`execution`/`account`。
//! - **人工门禁**：审批一律经界面裁决，默认拒绝；超时由 `AgentSession` 侧按
//!   失败关闭处理。
//! - **不可用时失败关闭**：运行时缺失、归档缺失、trace 目录不可写都在开跑之
//!   前判明，不在中途降级。

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
    shadow_tool::ShadowReportTool,
    tools::ToolRegistry,
    trace::ThreadOptions,
};

use crate::error::{ApiResponse, ErrorCode};

use super::{
    dto::{
        AgentApprovalDecision, AgentApprovalRequest, AgentReady, AgentStatus, AgentToolCall,
        AgentTurn,
    },
    support::{api_fail, api_map, load_config},
};

/// 归档新鲜度容差：宿主策略，不是模型入参。
///
/// 24 小时是「分析连续影子窗口」这一用途的取值——窗口本身是历史数据，过紧的
/// 阈值只会让工具恒失败。真正防的是把很久以前的窗口当成现状：界面上同时展示
/// `observation_age_ms`，且超过该阈值时工具**拒绝返回**。
const ARCHIVE_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

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
/// 写死一句话而非让用户从零输入，是因为本页的用途固定为「读只读工具并复述结
/// 论」；用户仍可改写。
const DEFAULT_PROMPT: &str = "调用 taoli_shadow_report 工具，然后说明归档中有多少条记录、观察时长是多少、有多少条被接受的套利机会。不要编造工具未返回的数字。";

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

/// 只读工具集：归档统计。绝不注册写操作。
fn tool_registry(archive: &Path) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShadowReportTool::new(
        archive.to_path_buf(),
        ARCHIVE_MAX_AGE_MS,
    )));
    Arc::new(registry)
}

/// 施加于 codex 子进程的沙箱策略。
///
/// 只授予运行时可证必需的可写位置：它自己的 home（会话与状态存储——实测缺少
/// 该授权会启动失败）与 trace 目录。**工作区不在其中**：本接入的边界是只读。
fn codex_confinement(codex_home: &Path, trace_dir: &Path) -> SandboxPolicy {
    sandbox::codex_child_policy(codex_home, Some(trace_dir))
}

/// 归档路径：优先观察配置，其次默认搜索。
fn archive_path(config_path: Option<String>) -> Result<PathBuf, anyhow::Error> {
    let (_, config) = load_config(config_path)?;
    Ok(config.archive.path.clone())
}

/// 就绪检查：运行时、归档、trace 目录、沙箱四项都必须可用。
///
/// 结果同时给出各项的实际取值，便于界面显示「在哪跑、读哪、写哪、是否受限」。
fn readiness(config_path: Option<String>) -> AgentReady {
    let program = discover_program().map(|path| path.to_string_lossy().into_owned());
    let archive = archive_path(config_path).ok();
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
    } else if archive.as_ref().is_none_or(|path| !path.is_file()) {
        Some("未找到观察归档。请先在控制台跑一次观测，或检查配置中的 archive.path。".to_string())
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
        archive_path: archive.map(|path| path.to_string_lossy().into_owned()),
        trace_dir,
        sandbox: confinement.to_string(),
        instructions: domain_instructions
            .as_ref()
            .ok()
            .map(|text| format!("{} 字节", text.len())),
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
        let archive = ready
            .archive_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("未找到归档路径"))?;
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
            error: None,
        }));

        let task_status = Arc::clone(&status);
        let task_slot = Arc::clone(&slot);
        tauri::async_runtime::spawn(async move {
            let outcome = run_agent(
                program,
                PathBuf::from(archive),
                commands_rx,
                Arc::clone(&task_status),
                task_slot,
            )
            .await;
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
    archive: PathBuf,
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

    let tools = tool_registry(&archive);
    let options = ThreadOptions {
        cwd: root.to_string_lossy().into_owned(),
        ephemeral: false,
        developer_instructions: Some(developer_instructions),
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
