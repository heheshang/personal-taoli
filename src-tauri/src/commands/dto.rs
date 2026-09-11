use rust_decimal::Decimal;
use serde::Serialize;

use personal_taoli_core::{
    account::AccountData,
    accounting::AccountingSmokeReport,
    control::ControlSmokeReport,
    execution::DoubleLegSmokeReport,
    local_book::{BookFeedStatus, BookState},
    order::OrderFactsSmokeReport,
    paper::PaperCoreSmokeReport,
    reconciliation::ReconciliationSmokeReport,
    scan::ScanReport,
};

#[derive(Debug, Clone, Serialize)]
pub struct DesktopStatus {
    pub mode: &'static str,
    pub version: &'static str,
    pub real_order_capability: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairSummary {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub quantity: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSummary {
    pub pairs: Vec<PairSummary>,
    pub orderbook_depth: u16,
    pub archive_path: String,
    pub binance_websocket_url: String,
    pub bybit_websocket_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedSummary {
    pub venue: String,
    pub symbol: String,
    pub state: BookState,
    pub generation: u64,
    pub reconnects: u64,
    pub applied_updates: u64,
    pub reason: Option<String>,
}

impl From<&BookFeedStatus> for FeedSummary {
    fn from(status: &BookFeedStatus) -> Self {
        Self {
            venue: status.venue.clone(),
            symbol: status.symbol.clone(),
            state: status.state,
            generation: status.generation,
            reconnects: status.reconnects,
            applied_updates: status.applied_updates,
            reason: status.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ObserveResult {
    pub report: ScanReport,
    pub accounts: [AccountData; 2],
    pub feeds: [FeedSummary; 2],
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuousStatus {
    pub running: bool,
    pub archive_path: Option<String>,
    pub gap_path: Option<String>,
    pub started_at_ms: Option<u64>,
    pub last_report_at_ms: Option<u64>,
    pub last_report: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairReconnectSmokeResult {
    pub symbol: String,
    pub binance: FeedSummary,
    pub bybit: FeedSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconnectSmokeResult {
    pub results: Vec<PairReconnectSmokeResult>,
    pub no_orders: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "report")]
pub enum PaperSmokeResult {
    B01(PaperCoreSmokeReport),
    B02(OrderFactsSmokeReport),
    B03(DoubleLegSmokeReport),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "report")]
pub enum AccountingControlSmokeResult {
    Accounting(AccountingSmokeReport),
    Reconciliation(ReconciliationSmokeReport),
    Control(ControlSmokeReport),
}

/// F-02 模拟套利烟测结果（S01–S09 + 安全边界）。
#[derive(Debug, Clone, Serialize)]
pub struct SimulationSmokeResult {
    pub schema_version: i64,
    pub s01_full_fill_at_worst: bool,
    pub s02_partial_fill_depth_shortfall: bool,
    pub s03_competed_away: bool,
    pub s04_worse_than_scan_price: bool,
    pub s05_insufficient_funds_rejected: bool,
    pub s06_unknown_query_recovered_no_duplicate: bool,
    pub s07_compensation_over_budget_manual: bool,
    pub s08_idempotent_replay: bool,
    pub s09_fact_recovery_consistent: bool,
    pub external_order_calls: usize,
}

impl From<personal_taoli_core::simulation::SimulationSmokeReport> for SimulationSmokeResult {
    fn from(report: personal_taoli_core::simulation::SimulationSmokeReport) -> Self {
        Self {
            schema_version: report.schema_version,
            s01_full_fill_at_worst: report.s01_full_fill_at_worst,
            s02_partial_fill_depth_shortfall: report.s02_partial_fill_depth_shortfall,
            s03_competed_away: report.s03_competed_away,
            s04_worse_than_scan_price: report.s04_worse_than_scan_price,
            s05_insufficient_funds_rejected: report.s05_insufficient_funds_rejected,
            s06_unknown_query_recovered_no_duplicate: report
                .s06_unknown_query_recovered_no_duplicate,
            s07_compensation_over_budget_manual: report.s07_compensation_over_budget_manual,
            s08_idempotent_replay: report.s08_idempotent_replay,
            s09_fact_recovery_consistent: report.s09_fact_recovery_consistent,
            external_order_calls: report.external_order_calls,
        }
    }
}

// ── PORT-01 codex 接入（只读分析）──────────────────────────────────────────

/// 接入就绪状态。`ready = false` 时 `reason` 给出可操作原因。
///
/// 就绪判断在**启动分析之前**完成：缺少运行时或归档时界面给出说明，而不是
/// 点击后才失败。
#[derive(Debug, Clone, Serialize)]
pub struct AgentReady {
    pub ready: bool,
    pub program: Option<String>,
    pub trace_dir: Option<String>,
    /// 沙箱可用性的人类可读描述（不可用时界面同时给出 `reason`）。
    pub sandbox: String,
    /// 领域指令的规模描述（来自受版本控制的 `docs/agent/instructions.md`）。
    pub instructions: Option<String>,
    /// 已配置的额外 skill 目录（来自 `TAOLI_AGENT_SKILL_ROOTS`）。
    pub skill_roots: Vec<String>,
    /// 已授权的额外可写根（来自 `TAOLI_AGENT_WRITE_ROOTS`）。
    ///
    /// 与 skill 根分开：能读到 skill 与允许它写自己的仓库是两件事。
    pub write_roots: Vec<String>,
    /// 新会话将使用的操作档位。
    pub access_level: String,
    pub reason: Option<String>,
}

/// 待裁决的审批请求（呈现给所有者）。
#[derive(Debug, Clone, Serialize)]
pub struct AgentApprovalRequest {
    pub request_id: i64,
    pub kind: String,
    pub summary: String,
    /// 运行时当时提供的裁决项，原样呈现，便于察觉上游词汇变化。
    pub advertised: Vec<String>,
    /// 本请求可用的裁决项，取值 `allow_once` / `allow_for_session` /
    /// `allow_always` / `deny`，顺序即建议顺序。
    ///
    /// 由后端按请求类型与是否附带了具体规则算出，而不是照抄运行时的
    /// `availableDecisions`——那个字段可选、且实测不含 `decline`。
    pub options: Vec<String>,
}

/// 已裁决的审批记录。
#[derive(Debug, Clone, Serialize)]
pub struct AgentApprovalDecision {
    pub request_id: i64,
    pub kind: String,
    pub summary: String,
    /// `allow_once` / `allow_for_session` / `allow_always` / `deny`。
    pub decision: String,
    /// `decider`（所有者裁决）/ `timeout`（超时按失败关闭）/ `unsupported`。
    pub source: String,
    pub waited_ms: u64,
}

/// 一次工具调用。
#[derive(Debug, Clone, Serialize)]
pub struct AgentToolCall {
    pub call_id: String,
    pub tool: String,
    pub success: bool,
    pub output: String,
}

/// 进行中一轮里的一步。
///
/// 与 [`AgentToolCall`] 的区别：那是轮次**结束**后的记录，这是**运行中**的视图，
/// 因此带状态（running/completed/failed/declined）与增量输出。
#[derive(Debug, Clone, Serialize)]
pub struct AgentLiveItem {
    pub item_id: String,
    /// `reasoning` / `command` / `file_change` / `tool_call` / `message` / `other`。
    pub kind: String,
    /// 一行标题：命令行走命令行，工具调用走工具名。
    pub title: String,
    /// 运行时的附加信息（命令是工作目录）。
    pub detail: Option<String>,
    /// `running` / `completed` / `failed` / `declined`。
    pub state: String,
    /// 已流式收到的输出（保留尾部，见 `agent_live`）。
    pub output: String,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}

/// 本轮 token 计费。
#[derive(Debug, Clone, Serialize)]
pub struct AgentTokens {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    /// 模型上下文窗口，运行时上报时才有。
    pub context_window: Option<u64>,
}

/// 正在进行的轮次的实时视图。
///
/// 没有「推理文本」这样的聚合字段：推理属于**它自己的步骤行**（`items` 里，
/// 由 `itemId` 对应），聚合会把文本与归属拆开，并在轮次结束时把推理一并丢掉。
///
/// 轮次结束后由后端清空：届时权威数据在 [`AgentTurn`] 里，留着会重复渲染同一批
/// 工作。
#[derive(Debug, Clone, Serialize)]
pub struct AgentLive {
    /// 属于哪一轮（与 [`AgentTurn::seq`] 对应），防止上一轮的迟到事件改写本轮。
    pub turn_seq: u64,
    /// `thinking` / `working` / `writing` / `awaiting_approval` / `done`。
    pub stage: String,
    /// 助手正文的流式文本（尾部）。
    pub message: String,
    pub items: Vec<AgentLiveItem>,
    pub tokens: Option<AgentTokens>,
}

/// 模型返回的结构，以及它是否符合 schema。
///
/// 「值 + 是否符合」放在同一个对象里，而不是两个可空字段：后者会容许「有标志却没值」
/// 这种不可能状态。此处只有两种可能——没有报告，或「这份报告，符合或不符合」。
#[derive(Debug, Clone, Serialize)]
pub struct AgentReport {
    /// 模型返回的对象，原样。
    pub value: serde_json::Value,
    /// 是否符合 schema。为 `false` 时字段名是模型自创的，**不得**当作报告渲染。
    pub conforms: bool,
}

/// 一轮分析的完整记录。
///
/// 每轮独立成条，使界面能像 codex 桌面版那样把会话呈现为**消息流**，而不是只
/// 显示最近一轮。`seq` 单调递增，供前端做稳定 key。
#[derive(Debug, Clone, Serialize)]
pub struct AgentTurn {
    pub seq: u64,
    pub prompt: String,
    pub tool_calls: Vec<AgentToolCall>,
    pub approvals: Vec<AgentApprovalDecision>,
    pub refused_requests: Vec<String>,
    /// 本轮**执行过的步骤**：命令、工具调用、文件变更、推理，均含其输出。
    ///
    /// 与 `tool_calls` 的区别是关键的：`tool_calls` 只记录**宿主工具**的分发
    /// （`item/tool/call`），而运行时自己执行的命令从不进入它。此前步骤只存在于
    /// 实时视图里，轮次一结束就被清空——实测完成后对话里**一行步骤都不剩**，
    /// 执行过的命令与输出全部消失。故在轮次结束时把实时步骤移入记录。
    pub items: Vec<AgentLiveItem>,
    /// 本轮全部 agent 消息，按时间顺序。
    ///
    /// 不是只留最后一条：分析类 skill 会边跑边汇报（stage 进度等），最后才给
    /// 结论——只留最后一条会把中间结果全丢掉，而那正是读者要在对话里看到的内容。
    pub messages: Vec<String>,
    /// 本轮被要求结构化输出时，模型实际返回的结构，以及它是否符合 schema。
    ///
    /// 直接透传 JSON：schema 是前后端共同遵循的契约（`docs/agent/report-schema.json`），
    /// 在这里再定义一遍 Rust 结构只会有两处需要同步。
    ///
    /// `None` 表示本轮压根没产出 JSON（回复是散文，见 `messages`）。**不符合 schema
    /// 的对象也会保留**：模型忽略 schema 时仍会返回「某种」结构化内容，只是字段名自创；
    /// 一次数分钟的调用不该因为三个键名不同就被丢掉，界面以通用元数据原样呈现。
    pub report: Option<AgentReport>,
    /// 要求了结构化输出却未通过校验时的原因；
    /// 界面据此说明「为什么这里显示的是正文而非报告」。
    pub report_error: Option<String>,
    /// 本轮失败原因。`None` 表示本轮正常结束。
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

/// 一个可选的操作档位，供界面渲染下拉项。
///
/// 标签与说明取自 Codex 桌面版语言包的同一组词，使两个界面用同样的话描述同一件事。
#[derive(Debug, Clone, Serialize)]
pub struct AgentAccessLevel {
    pub token: String,
    pub label: String,
    pub description: String,
    /// 该档位是否会把审批请求交给本客户端。
    ///
    /// 「帮我批准」由运行时自己的子代理判定，「完全访问」不问——两者都不会经过
    /// 本页的审批卡，界面必须如实说明，否则会让人以为每个动作仍由自己把关。
    pub consults_client: bool,
    /// 该档位是否仍受本项目沙箱约束。`完全访问权限` 为 false。
    pub confined: bool,
}

/// 运行时报告可用的一个 skill。
#[derive(Debug, Clone, Serialize)]
pub struct AgentSkill {
    /// 可能带命名空间前缀，如 `stock-deep-analyzer:uzi`——前缀标识来源。
    pub name: String,
    pub description: String,
}

/// agent 会话生命周期快照。
///
/// `phase` 取值：`stopped` | `starting` | `ready` | `running` |
/// `awaiting_approval` | `error`。
#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub phase: &'static str,
    pub program: Option<String>,
    pub thread_id: Option<String>,
    pub trace_path: Option<String>,
    pub pending_approval: Option<AgentApprovalRequest>,
    /// 已完成与进行中的轮次，按时间升序；最后一条可能是进行中的那一轮。
    pub turns: Vec<AgentTurn>,
    /// 运行时报告可用的 skill。
    pub skills: Vec<AgentSkill>,
    /// 当前会话使用的操作档位（未启动时为新会话将使用的档位）。
    pub access_level: String,
    /// 正在进行的轮次的实时视图；无进行中轮次时为 `null`。
    pub live: Option<AgentLive>,
    /// 会话级致命错误（运行时启动失败或退出）。轮次内的失败记在该轮的 `error`。
    pub error: Option<String>,
}
