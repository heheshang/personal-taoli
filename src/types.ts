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
