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
    access::{AccessLevel, AccessSetting},
    app_server::AppServerConfig,
    approval::{self, ApprovalDecider, ApprovalRequest, Decision},
    discover_program, instructions, report,
    sandbox::{self, SandboxPolicy},
    tools::ToolRegistry,
    trace::ThreadOptions,
};

use crate::error::{ApiResponse, ErrorCode};

use super::{
    agent_live::LiveTracker,
    dto::{
        AgentAccessLevel, AgentApprovalDecision, AgentApprovalRequest, AgentLive, AgentReady,
        AgentSkill, AgentStatus, AgentToolCall, AgentTurn,
    },
    support::{api_fail, api_map},
};

/// 单轮上限：**静默**时长、硬顶、工具调用数、审批等待时长。
///
/// 静默时长而非总时长：真实的深度分析 skill 会持续汇报进度而跑很久（实测 UZI
/// skill 的 stage1 抓取早已超过五分钟仍在正常推进），用总时长做闸会把正在正常
/// 工作的轮次砍掉。判据是「还在动」，不是「跑了多久」。
///
/// 二者可用环境变量覆盖，因为「多久算合理」取决于所有者用的 skill：
/// `TAOLI_AGENT_TURN_IDLE_SECS`、`TAOLI_AGENT_TURN_MAX_SECS`。
const DEFAULT_TURN_IDLE_SECS: u64 = 600;
const DEFAULT_TURN_MAX_SECS: u64 = 7_200;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_TOOL_CALLS: usize = 32;

/// 读取一个秒数上限，非法值一律退回默认并记录，不静默取 0。
fn seconds_from_env(variable: &str, fallback: u64) -> Duration {
    let Ok(raw) = std::env::var(variable) else {
        return Duration::from_secs(fallback);
    };
    match raw.trim().parse::<u64>() {
        Ok(0) => {
            tracing::warn!(target: "agent", variable, "ignoring a zero limit; using the default");
            Duration::from_secs(fallback)
        }
        Ok(seconds) => Duration::from_secs(seconds),
        Err(error) => {
            tracing::warn!(
                target: "agent",
                variable,
                value = %raw,
                %error,
                "unparseable limit; using the default"
            );
            Duration::from_secs(fallback)
        }
    }
}

fn turn_limits() -> TurnLimits {
    TurnLimits {
        idle_timeout: seconds_from_env("TAOLI_AGENT_TURN_IDLE_SECS", DEFAULT_TURN_IDLE_SECS),
        max_duration: seconds_from_env("TAOLI_AGENT_TURN_MAX_SECS", DEFAULT_TURN_MAX_SECS),
        max_tool_calls: MAX_TOOL_CALLS,
        approval_timeout: APPROVAL_TIMEOUT,
    }
}

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
                    options: request
                        .options
                        .iter()
                        .map(|decision| decision_token(*decision).to_string())
                        .collect(),
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
    /// The level this session was started with, so a change can be refused
    /// rather than silently ignored: the runtime takes these settings at
    /// `thread/start`, and a sandbox cannot be retrofitted onto a live process.
    level: AccessLevel,
}

/// 应用级句柄：同一时刻至多一个分析会话。
pub struct AgentController {
    inner: Mutex<Option<RunningAgent>>,
    /// The level a new session will use.
    ///
    /// Session-scoped, like the app's per-composer dropdown, and independent of
    /// `inner` so it survives a stopped session. Defaults to the asking level,
    /// the only one that keeps the original read-only boundary.
    access: Mutex<AccessSetting>,
}

impl Default for AgentController {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            access: Mutex::new(AccessSetting::default()),
        }
    }
}

/// 施加于 codex 子进程的沙箱策略。
///
/// 只授予运行时可证必需的可写位置：它自己的 home（会话与状态存储——实测缺少
/// 该授权会启动失败）与 trace 目录。**工作区不在其中**：本接入的边界是只读。
/// 当前生效的档位：有控制器就用它，否则用默认（`ask`）。
///
/// `readiness` 拿不到 `AppHandle`，故接受一个可选控制器；命令层传真实状态。
fn access_level(controller: Option<&AgentController>) -> AccessLevel {
    controller.map_or_else(AccessLevel::default, |controller| {
        lock(&controller.access).level
    })
}

/// 档位清单，供界面渲染下拉项。
fn access_levels() -> Vec<AgentAccessLevel> {
    AccessLevel::ALL
        .into_iter()
        .map(|level| AgentAccessLevel {
            token: level.token().to_string(),
            label: level.label().to_string(),
            description: level.description().to_string(),
            consults_client: level.consults_this_client(),
            confined: level != AccessLevel::FullAccess,
        })
        .collect()
}

/// 按档位派生沙箱子进程的策略。
///
/// 档位决定两件事，二者必须一致，否则标签就成了假话：交给运行时的沙箱/审批设置，
/// 以及**本项目自己**的 Seatbelt 约束。`完全访问权限` 返回 `None`——不施加任何约束。
fn confinement_for(
    level: AccessLevel,
    codex_home: &Path,
    trace_dir: &Path,
    workspace: &Path,
) -> Option<SandboxPolicy> {
    // `完全访问权限` yields no policy: applying one would make the level's own
    // description ("可不受限制地…") untrue.
    let mut policy = level.confinement(codex_home, Some(trace_dir), Some(workspace))?;
    // 额外可写根：skill 的脚本会写自己的仓库（.cache / reports）。显式授权，
    // 由 TAOLI_AGENT_WRITE_ROOTS 提供，不由 skill 根自动推导；只叠加在有约束的档位上。
    policy.writable_roots.extend(write_roots());
    Some(policy)
}

/// 就绪检查：运行时、trace 目录、沙箱、领域指令四项都必须可用。
///
/// 归档**不再**是就绪前提：需要归档的是已移除的那个套利工具，留下这道门只会让
/// 缺归档时无故拒绝启动。`config_path` 仍保留在签名里，供后续需要配置的能力使用。
fn readiness(config_path: Option<String>) -> AgentReady {
    let _ = config_path;
    readiness_with(None)
}

fn readiness_with(controller: Option<&AgentController>) -> AgentReady {
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
        access_level: access_level(controller).token().to_string(),
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
        access_level: AccessLevel::default().token().to_string(),
        live: None,
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
/// 把裁决转成与前端约定的稳定 token。
///
/// 用字符串而非 Rust 枚举名：界面按它对上按钮，改这里等于改前后端契约。
fn decision_token(decision: Decision) -> &'static str {
    match decision {
        Decision::AllowOnce => "allow_once",
        Decision::AllowForSession => "allow_for_session",
        Decision::AllowAlways => "allow_always",
        Decision::Deny => "deny",
    }
}

/// 校验裁决确实在本请求提供的选项里。
///
/// 这是防越权的那一道：界面之外的调用方（脚本、将来的自动化）不得对一个未附规则的
/// 请求要求「始终允许」，也不得对一个文件变更要求协议里不存在的永久授权。选项集合
/// 由 `approval::available_decisions` 依请求类型与运行时的提议算出，这里只做成员检查。
fn ensure_offered(options: &[String], decision: Decision) -> Result<(), anyhow::Error> {
    let token = decision_token(decision);
    if options.iter().any(|option| option == token) {
        return Ok(());
    }
    anyhow::bail!("裁决 `{token}` 不在本请求提供的选项中：{options:?}")
}

/// 解析界面传来的裁决 token。未知取值失败，不猜、也不降级为放行。
fn parse_decision(token: &str) -> Result<Decision, anyhow::Error> {
    match token {
        "allow_once" => Ok(Decision::AllowOnce),
        "allow_for_session" => Ok(Decision::AllowForSession),
        "allow_always" => Ok(Decision::AllowAlways),
        "deny" => Ok(Decision::Deny),
        other => anyhow::bail!("unknown approval decision `{other}`"),
    }
}

fn approval_dto(
    record: &personal_taoli_core::agent::approval::ApprovalRecord,
) -> AgentApprovalDecision {
    use personal_taoli_core::agent::approval::DecisionSource;
    AgentApprovalDecision {
        request_id: record.request_id,
        kind: record.kind.as_str().to_string(),
        summary: record.summary.clone(),
        decision: decision_token(record.decision).to_string(),
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
            None => {
                let mut status =
                    stopped_status(discover_program().map(|p| p.to_string_lossy().into_owned()));
                status.access_level = lock(&self.access).level.token().to_string();
                status
            }
        }
    }

    /// 启动运行时并建立线程。已启动时幂等返回当前状态。
    fn start(&self, config_path: Option<String>) -> Result<AgentStatus, anyhow::Error> {
        let ready = readiness(config_path.clone());
        if !ready.ready {
            anyhow::bail!(ready.reason.unwrap_or_else(|| "agent 未就绪".to_string()));
        }
        let program = ready.program.clone();
        let level = lock(&self.access).level;

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
            access_level: level.token().to_string(),
            live: None,
            error: None,
        }));

        let task_status = Arc::clone(&status);
        let task_slot = Arc::clone(&slot);
        tauri::async_runtime::spawn(async move {
            let outcome = run_agent(
                program,
                level,
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
            level,
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

    /// 设置新会话使用的档位。
    ///
    /// 会话运行中且档位不同则拒绝：运行时的沙箱与审批设置是 `thread/start` 时确定
    /// 的，无法给一个已在运行的进程追加沙箱。停掉会话即可切换（与 app 的「适用于
    /// 新聊天」一致）。
    fn set_access_level(&self, level: AccessLevel) -> Result<AgentStatus, anyhow::Error> {
        {
            let guard = lock(&self.inner);
            if let Some(agent) = guard.as_ref()
                && agent.level != level
            {
                anyhow::bail!(
                    "会话正在以「{}」运行，无法切换到「{}」：沙箱与审批设置在线程启动时确定，请先停止会话",
                    agent.level.label(),
                    level.label()
                );
            }
        }
        lock(&self.access).level = level;
        Ok(self.snapshot())
    }

    /// 所有者裁决待审批动作。无待裁决项时失败（不猜、不默认放行）。
    ///
    /// `decision` 是 [`decision_token`] 的取值之一。仅接受该请求**实际提供**
    /// 的选项：界面之外的调用方不得凭空升级授权范围（例如对一个未附规则的
    /// 请求要求「始终允许」）。
    fn decide(&self, request_id: i64, decision: Decision) -> Result<AgentStatus, anyhow::Error> {
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
            ensure_offered(&pending.request.options, decision)?;
            pending.decided = true;
            decision
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
    level: AccessLevel,
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
    // `完全访问权限` produces no confinement at all; see `confinement_for`.
    let confinement = confinement_for(level, &codex_home, &trace, &root);
    // A permanent approval is only meaningful if the runtime can write its rule
    // store. Every confined level denies that directory, so under them the fourth
    // approval action is withheld rather than shown and ignored. The unconfined
    // level has no such block — and no approvals either, its policy being `never`.
    let persistence = match &confinement {
        Some(policy) if policy.blocks_rule_persistence() => approval::Persistence::Blocked,
        _ => approval::Persistence::Allowed,
    };
    // Re-read rather than carrying the readiness result: the document is read
    // once per session, and a failure here must stop the launch.
    let developer_instructions =
        instructions::load_default().map_err(|error| anyhow::anyhow!("{error}"))?;
    // Same treatment as the instructions: a missing or malformed contract stops the
    // launch rather than silently running a session that cannot produce a report.
    let report_schema: serde_json::Value =
        report::load_default().map_err(|error| anyhow::anyhow!("{error}"))?;

    let config = AppServerConfig {
        program: PathBuf::from(program),
        request_timeout: Duration::from_secs(60),
        confinement,
        ..AppServerConfig::default()
    };
    let mut session = AgentSession::connect(config).await?;

    // 本层不注册宿主工具：能力来自运行时内置工具与已配置的 skill，副作用动作
    // 一律过审批。注册表留在这里，是给将来一个作用域正确的工具留位置。
    let tools = Arc::new(ToolRegistry::new());
    let options = ThreadOptions {
        cwd: root.to_string_lossy().into_owned(),
        ephemeral: false,
        // The level is the single source for all three: they are one decision in
        // the app too, and splitting them would allow combinations the app never
        // offers and this project has not reasoned about.
        sandbox: level.sandbox(),
        approval_policy: level.approval_policy(),
        approvals_reviewer: level.reviewer(),
        developer_instructions: Some(developer_instructions),
        skill_roots: skill_roots(),
        trace_dir: Some(trace),
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
                let (seq, tracker) = {
                    let mut guard = lock(&status);
                    guard.phase = "running";
                    guard.error = None;
                    guard.pending_approval = None;
                    let seq = guard.turns.len() as u64;
                    // The live view is created here, before the turn's first
                    // event, so an early notification cannot arrive with nowhere
                    // to land.
                    guard.live = Some(AgentLive {
                        turn_seq: seq,
                        stage: "thinking".to_string(),
                        reasoning: String::new(),
                        message: String::new(),
                        items: Vec::new(),
                        tokens: None,
                    });
                    let tracker = Arc::new(LiveTracker::new(Arc::clone(&status), seq));
                    guard.turns.push(AgentTurn {
                        seq,
                        prompt: prompt.clone(),
                        tool_calls: Vec::new(),
                        approvals: Vec::new(),
                        refused_requests: Vec::new(),
                        messages: Vec::new(),
                        report: None,
                        report_error: None,
                        error: None,
                        started_at_ms: now_ms(),
                        finished_at_ms: None,
                    });
                    (seq, tracker)
                };

                let context = TurnContext {
                    tools: Arc::clone(&tools),
                    approvals: Arc::new(UiApprovalDecider::new(Arc::clone(&slot))),
                    audit: None,
                    persistence,
                    // Every analysis turn asks for the report schema, so the
                    // interface can render a structured result when the model
                    // honours it — and falls back to the prose when it does not.
                    output_schema: Some(report_schema.clone()),
                    // Streams the turn into the shared status while it runs.
                    progress: Some(tracker.sink()),
                    limits: turn_limits(),
                };
                let result = session.run_turn(&prompt, context).await;

                let mut guard = lock(&status);
                let (tool_calls, approvals, refused, messages, report, report_error, error) =
                    match result {
                        Ok(outcome) => (
                            outcome.tool_calls.iter().map(tool_call_dto).collect(),
                            outcome.approvals.iter().map(approval_dto).collect(),
                            outcome.refused_requests.clone(),
                            outcome.messages.clone(),
                            outcome.report.clone(),
                            outcome.report_error.clone(),
                            None,
                        ),
                        Err(error) => (
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            None,
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
                    turn.messages = messages;
                    turn.report = report;
                    turn.report_error = report_error;
                    turn.error = error;
                    turn.finished_at_ms = Some(now_ms());
                }
                // The record is now authoritative; keeping the live copy would
                // render the same work twice.
                guard.live = None;
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
pub fn agent_ready(app: AppHandle) -> ApiResponse<AgentReady> {
    let state = app.state::<AgentController>();
    ApiResponse::ok(readiness_with(Some(&state)))
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

/// 裁决一次审批。
///
/// `decision` ∈ `allow_once` / `allow_for_session` / `allow_always` / `deny`，
/// 与 Codex 桌面版审批卡的四个动作一一对应。
#[tauri::command]
pub fn agent_decide(app: AppHandle, request_id: i64, decision: String) -> ApiResponse<AgentStatus> {
    let state = app.state::<AgentController>();
    match parse_decision(&decision) {
        Ok(decision) => api_map(
            state.decide(request_id, decision),
            ErrorCode::InvalidRequest,
            false,
        ),
        Err(error) => api_fail(ErrorCode::InvalidRequest, error, false),
    }
}

/// 可选档位清单（标签与说明取自 Codex 桌面版）。
#[tauri::command]
pub fn agent_access_levels() -> ApiResponse<Vec<AgentAccessLevel>> {
    ApiResponse::ok(access_levels())
}

/// 设置新会话使用的档位。会话运行中改档会被拒绝，需先停止会话。
#[tauri::command]
pub fn agent_set_access_level(app: AppHandle, level: String) -> ApiResponse<AgentStatus> {
    let Some(level) = AccessLevel::parse(&level) else {
        return api_fail(
            ErrorCode::InvalidRequest,
            format!("unknown access level `{level}`"),
            false,
        );
    };
    let state = app.state::<AgentController>();
    api_map(
        state.set_access_level(level),
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
        let policy = confinement_for(AccessLevel::Ask, &codex_home, &dir, Path::new("/tmp"))
            .expect("the asking level is confined");
        for root in &policy.writable_roots {
            assert!(
                root.is_absolute(),
                "every writable root must be absolute, got {}",
                root.display()
            );
        }

        // Building the argv is what rejected the relative path before.
        let policy = confinement_for(AccessLevel::Ask, &codex_home, &dir, Path::new("/tmp"))
            .expect("the asking level is confined");
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

/// Serialises every test that touches process-global environment variables.
///
/// **One** lock for the whole crate, not one per test module: two modules each
/// holding their own mutex provide no mutual exclusion at all, which is exactly
/// how a real flake appeared here. Tests that *read* env-derived defaults must
/// take it too, not only the ones that write.
#[cfg(test)]
mod test_env {
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn locked<F: FnOnce()>(body: F) {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        body();
        drop(guard);
    }
}

#[cfg(test)]
mod root_env_tests {
    use super::test_env::locked;
    use super::*;

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
            let policy = confinement_for(AccessLevel::Ask, &codex_home, &trace, Path::new("/tmp"))
                .expect("the asking level is confined");

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

#[cfg(test)]
mod decision_token_tests {
    use super::*;

    /// Every verdict the runtime understands round-trips through the token the
    /// interface sends. A miss here would silently send the wrong grant.
    #[test]
    fn every_decision_round_trips_through_its_token() {
        for decision in [
            Decision::AllowOnce,
            Decision::AllowForSession,
            Decision::AllowAlways,
            Decision::Deny,
        ] {
            let token = decision_token(decision);
            assert_eq!(
                parse_decision(token).expect("known token"),
                decision,
                "{token}"
            );
        }
        // And the tokens are distinct: two verdicts sharing one token would
        // collapse 允许一次 and 始终允许 into the same grant.
        let mut tokens: Vec<&str> = [
            Decision::AllowOnce,
            Decision::AllowForSession,
            Decision::AllowAlways,
            Decision::Deny,
        ]
        .iter()
        .map(|decision| decision_token(*decision))
        .collect();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), 4);
    }

    #[test]
    fn an_unknown_token_is_refused_rather_than_defaulted() {
        for bad in ["", "allow", "ALLOW_ONCE", "accept", "yes"] {
            assert!(parse_decision(bad).is_err(), "`{bad}` must not be accepted");
        }
    }

    /// The escalation guard: a caller cannot pick a verdict the request did not
    /// offer — in particular it cannot ask for a permanent grant when the
    /// runtime proposed no rule to remember.
    #[test]
    fn a_verdict_outside_the_offered_set_is_refused() {
        let without_rule = vec![
            "allow_once".to_string(),
            "allow_for_session".to_string(),
            "deny".to_string(),
        ];
        assert!(ensure_offered(&without_rule, Decision::AllowOnce).is_ok());
        assert!(ensure_offered(&without_rule, Decision::AllowForSession).is_ok());
        assert!(ensure_offered(&without_rule, Decision::Deny).is_ok());

        let error = ensure_offered(&without_rule, Decision::AllowAlways)
            .expect_err("no rule was proposed, so 始终允许 is not available");
        assert!(error.to_string().contains("allow_always"), "{error}");

        // And with a rule proposed, it is.
        let with_rule = vec![
            "allow_once".to_string(),
            "allow_for_session".to_string(),
            "allow_always".to_string(),
            "deny".to_string(),
        ];
        assert!(ensure_offered(&with_rule, Decision::AllowAlways).is_ok());
    }
}

#[cfg(test)]
mod access_level_tests {
    use super::*;

    /// The interface renders these, so their labels and descriptions must be the
    /// app's own — that is the whole point of the parity.
    #[test]
    fn the_levels_offered_match_the_apps_dropdown() {
        let levels = access_levels();
        let tokens: Vec<&str> = levels.iter().map(|level| level.token.as_str()).collect();
        assert_eq!(tokens, vec!["ask", "auto_approve", "full_access"]);

        let labels: Vec<&str> = levels.iter().map(|level| level.label.as_str()).collect();
        assert_eq!(labels, vec!["请求批准", "帮我批准", "完全访问权限"]);
        for level in &levels {
            assert!(!level.description.is_empty(), "{level:?}");
        }
    }

    /// The two facts the interface must not get wrong: which levels still ask
    /// this client, and which still apply a sandbox.
    #[test]
    fn the_levels_report_whether_they_still_ask_and_still_confine() {
        let levels = access_levels();
        let by_token = |token: &str| {
            levels
                .iter()
                .find(|level| level.token == token)
                .unwrap_or_else(|| panic!("{token} missing"))
        };

        // Only the asking level puts requests to this client.
        assert!(by_token("ask").consults_client);
        assert!(
            !by_token("auto_approve").consults_client,
            "the runtime reviews"
        );
        assert!(!by_token("full_access").consults_client, "it never asks");

        // Only full access drops the sandbox.
        assert!(by_token("ask").confined);
        assert!(by_token("auto_approve").confined);
        assert!(!by_token("full_access").confined);
    }

    #[test]
    fn a_default_controller_reports_the_asking_level() {
        let controller = AgentController::default();
        assert_eq!(lock(&controller.access).level, AccessLevel::Ask);
    }

    /// Setting a level is allowed while idle, and the snapshot reflects it —
    /// this is what the dropdown shows before a session exists.
    #[test]
    fn a_level_set_while_idle_is_reported() {
        let controller = AgentController::default();
        let status = controller
            .set_access_level(AccessLevel::AutoApprove)
            .expect("idle controller accepts a change");
        assert_eq!(status.access_level, "auto_approve");
        assert_eq!(lock(&controller.access).level, AccessLevel::AutoApprove);
    }

    /// An unknown token must not silently fall back to a level: a typo would
    /// otherwise change the sandbox.
    #[test]
    fn an_unknown_level_token_is_refused() {
        assert!(AccessLevel::parse("sandboxed").is_none());
        assert!(AccessLevel::parse("Ask").is_none());
        assert!(AccessLevel::parse("").is_none());
    }
}

#[cfg(test)]
mod turn_limit_tests {
    use super::test_env::locked;
    use super::*;

    /// Regression: the limit that killed a real analysis must be gone.
    ///
    /// A UZI stage-1 run was cut off after 300s while it was still reporting
    /// progress. The offending limit was a **total** turn budget; what exists
    /// now is an idle limit (silence, not duration) plus a much larger ceiling.
    #[test]
    fn no_total_turn_budget_remains() {
        // Locked because it reads env-derived defaults: unlocked, it can observe
        // another test's temporary override and fail intermittently.
        locked(|| {
            let limits = turn_limits();
            // The old total was 300s; the ceiling must be far above it, or the same
            // failure returns for any skill that legitimately runs long.
            assert!(
                limits.max_duration >= Duration::from_secs(3_600),
                "the ceiling must not be a wall-clock cap on real work: {:?}",
                limits.max_duration
            );
            // And silence, not duration, is what marks a turn stuck.
            assert!(
                limits.idle_timeout >= Duration::from_secs(120),
                "too eager to call a slow skill stuck: {:?}",
                limits.idle_timeout
            );
            assert!(
                limits.idle_timeout < limits.max_duration,
                "the idle limit is the primary control and must fire first"
            );
        });
    }

    #[test]
    fn the_limits_are_overridable_because_they_depend_on_the_skill_in_use() {
        locked(|| {
            // SAFETY: guarded by ENV_LOCK.
            unsafe {
                std::env::set_var("TAOLI_AGENT_TURN_IDLE_SECS", "42");
                std::env::set_var("TAOLI_AGENT_TURN_MAX_SECS", "4242");
            }
            let limits = turn_limits();
            assert_eq!(limits.idle_timeout, Duration::from_secs(42));
            assert_eq!(limits.max_duration, Duration::from_secs(4_242));
            unsafe {
                std::env::remove_var("TAOLI_AGENT_TURN_IDLE_SECS");
                std::env::remove_var("TAOLI_AGENT_TURN_MAX_SECS");
            }
        });
    }

    /// A bad value must fall back, not silently become zero — a zero limit would
    /// end every turn instantly.
    #[test]
    fn an_unusable_limit_falls_back_instead_of_becoming_zero() {
        for bad in ["0", "-1", "soon", ""] {
            assert_eq!(
                seconds_from_env("TAOLI_AGENT_TURN_IDLE_SECS_UNSET", 600),
                Duration::from_secs(600),
                "an absent variable must use the default"
            );
            locked(|| {
                // SAFETY: guarded by ENV_LOCK.
                unsafe { std::env::set_var("TAOLI_AGENT_TURN_IDLE_SECS_PROBE", bad) };
                let resolved = seconds_from_env("TAOLI_AGENT_TURN_IDLE_SECS_PROBE", 600);
                unsafe { std::env::remove_var("TAOLI_AGENT_TURN_IDLE_SECS_PROBE") };
                assert_eq!(
                    resolved,
                    Duration::from_secs(600),
                    "`{bad}` must fall back rather than disable the limit"
                );
            });
        }
    }
}
