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
    pub archive_path: Option<String>,
    pub trace_dir: Option<String>,
    /// 沙箱可用性的人类可读描述（不可用时界面同时给出 `reason`）。
    pub sandbox: String,
    /// 领域指令的规模描述（来自受版本控制的 `docs/agent/instructions.md`）。
    pub instructions: Option<String>,
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
}

/// 已裁决的审批记录。
#[derive(Debug, Clone, Serialize)]
pub struct AgentApprovalDecision {
    pub request_id: i64,
    pub kind: String,
    pub summary: String,
    /// `allow` / `deny`。
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
    pub final_message: Option<String>,
    /// 本轮失败原因。`None` 表示本轮正常结束。
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
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
    /// 会话级致命错误（运行时启动失败或退出）。轮次内的失败记在该轮的 `error`。
    pub error: Option<String>,
}
