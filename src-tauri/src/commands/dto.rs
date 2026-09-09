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
