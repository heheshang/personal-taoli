export type ApiError = {
  code: string
  message: string
  retryable: boolean
}

export type ApiResponse<T> = {
  success: boolean
  data: T | null
  error: ApiError | null
}

export type Status = {
  mode: string
  version: string
  real_order_capability: boolean
}

export type PairSummary = {
  symbol: string
  base_asset: string
  quote_asset: string
  quantity: string
}

export type Config = {
  pairs: PairSummary[]
  orderbook_depth: number
  archive_path: string
  binance_websocket_url: string
  bybit_websocket_url: string
}

export type FeeSchedule = {
  symbol: string
  source: string
  buy_taker_rate: string
  sell_taker_rate: string
}

export type Account = {
  venue: string
  capability: Record<string, unknown>
  permission: Record<string, unknown>
  fee: FeeSchedule
  rejection_reasons: string[]
}

export type FeedSummary = {
  venue: string
  symbol: string
  state: string
  generation: number
  reconnects: number
  applied_updates: number
  reason: string | null
}

export type Observe = {
  report: Record<string, unknown>
  accounts: Account[]
  feeds: FeedSummary[]
  archived: boolean
}

export type Continuous = {
  running: boolean
  archive_path: string | null
  gap_path: string | null
  started_at_ms: number | null
  last_report_at_ms: number | null
  last_report: Record<string, unknown> | null
  error: string | null
}

/* ===================== G-01 模拟套利仪表盘 ===================== */
/** `simulation_query.rs` 的 `SimulationOverview`。金额均为 Decimal 字符串（serde-str）。 */
export type SimulationOverview = {
  total_runs: number
  total_success: number
  total_net_profit: string
  scanned_net_profit: string
  profit_runs: number
  by_scenario: ScenarioCount[]
  recent_runs: SimulationRunRow[]
  cumulative_points: NetProfitPoint[]
  account_balances: AccountSnapshot[]
  balance_history: BalancePoint[]
  ridge: RidgeBucket[]
  activity: ActivityEvent[]
  flow: FlowAggregate
}

/** `simulation_query.rs` 的 `FlowAggregate`：执行流转图计数。 */
export type FlowAggregate = {
  evaluated_runs: number
  reserved_runs: number
  filled_runs: number
  compensated_runs: number
  completed_runs: number
  escalated_runs: number
}

export type ScenarioCount = { scenario: string; count: number }

export type NetProfitPoint = {
  executed_at_ms: number
  scanned_net_profit: string
  simulated_net_profit: string
}

/** 列表行（不含 report JSONB 全文）。 */
export type SimulationRunRow = {
  run_id: string
  executed_at_ms: number
  evaluated_at_ms: number
  symbol: string
  buy_venue: string
  sell_venue: string
  direction: string
  scenario: string
  rejection_reason: string | null
  quantity: string
  bought_quantity: string
  sold_quantity: string
  buy_avg_price: string
  sell_avg_price: string
  buy_fee: string
  sell_fee: string
  scanned_net_profit: string
  simulated_net_profit: string
  compensation_decision: string
  execution_state: string
  idempotent_replay: boolean
  external_order_calls: number
}

export type SimulationRunsPage = { total: number; runs: SimulationRunRow[] }

export type SimulationRunDetail = {
  run: SimulationRunRow
  report: Record<string, unknown>
  intents: IntentFactRow[]
  trades: TradeFactRow[]
  execution_events: ExecutionEventRow[]
  audit: AuditEventRow[]
  balances: BalancePoint[]
}

export type IntentFactRow = {
  intent_id: string
  leg_id: string
  client_order_id: string
  venue: string
  side: string
  quantity: string
  limit_price: string
  submission_status: string
  filled_quantity: string
  exchange_order_id: string | null
  cancel_status: string
  reconciliation_status: string
  created_at_ms: number
  actions: ActionFactRow[]
}

export type ActionFactRow = {
  fact_id: string
  fact_version: number
  action: string
  occurred_at_ms: number
  payload: Record<string, unknown>
}

export type TradeFactRow = {
  venue: string
  exchange_order_id: string
  trade_id: string
  intent_id: string
  quantity: string
  price: string
  fee_asset: string
  fee_amount: string
  occurred_at_ms: number
}

export type ExecutionEventRow = {
  event_id: string
  plan_id: string
  event_version: number
  event_type: string
  occurred_at_ms: number
  payload: Record<string, unknown>
}

export type AuditEventRow = {
  event_id: string
  aggregate_id: string
  aggregate_version: number
  event_type: string
  correlation_id: string
  occurred_at_ms: number
  payload: Record<string, unknown>
}

export type AccountSnapshot = {
  account_id: string
  venue: string
  asset: string
  observed_total: string
  observed_free: string
  local_reserved: string
  observed_at_ms: number
}

export type BalancePoint = {
  run_id: string
  account_id: string
  venue: string
  asset: string
  node: string
  total: string
  free: string
  observed_at_ms: number
}

export type RidgeBucket = {
  bucket_start_ms: number
  bucket_end_ms: number
  nets: string[]
}

export type ActivityEvent = {
  occurred_at_ms: number
  event_type: string
  aggregate_id: string
  correlation_id: string
  payload: Record<string, unknown>
  run_id: string | null
}
