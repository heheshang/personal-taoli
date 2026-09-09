import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import type { ApiResponse } from './types'

/**
 * Tauri 后台命令名，与 `src-tauri/src/lib.rs` 的 `invoke_handler` 注册集一一对应。
 * 集中成常量，消除散落各组件中的魔法字符串。
 */
export const COMMANDS = {
  desktopStatus: 'desktop_status',
  loadConfigSummary: 'load_config_summary',
  accountStatus: 'account_status',
  observeOnce: 'observe_once',
  replayObservations: 'replay_observations',
  runPaperSmoke: 'run_paper_smoke',
  runAccountingControlSmoke: 'run_accounting_control_smoke',
  startContinuousObservation: 'start_continuous_observation',
  stopContinuousObservation: 'stop_continuous_observation',
  continuousObservationStatus: 'continuous_observation_status',
  runReconnectSmoke: 'run_reconnect_smoke',
  loadAppConfig: 'load_app_config',
  saveAppConfig: 'save_app_config',
  getObserverConfig: 'get_observer_config',
  saveObserverConfig: 'save_observer_config',
} as const

export const PAPER_KINDS = ['B01', 'B02', 'B03'] as const
export type PaperKind = (typeof PAPER_KINDS)[number]

export const ACCOUNTING_CONTROL_KINDS = ['ACCOUNTING', 'RECONCILIATION', 'CONTROL'] as const
export type AccountingControlKind = (typeof ACCOUNTING_CONTROL_KINDS)[number]

/** `load_app_config` / `save_app_config` 的配置形状，与 `settings.rs` 的 `AppConfig` 一致。 */
export interface AppConfig {
  database_url: string
  binance_api_key: string
  binance_api_secret: string
  bybit_api_key: string
  bybit_api_secret: string
}

/** 交易场所配置，与 `config.rs` 的 `VenueConfig` 一致（`api_key_env` 是环境变量名字符串）。 */
export interface VenueConfig {
  base_url: string
  websocket_url: string
  fallback_taker_fee_rate: string
  api_key_env: string
  api_secret_env: string
  region_eligible_confirmed: boolean
  account_eligible_confirmed: boolean
}

/** 单个观察交易对（`crates/core/src/config.rs` 的 `PairConfig`）。 */
export interface PairConfig {
  symbol: string
  base_asset: string
  quote_asset: string
  quantity: string
}

/**
 * 完整观察配置，与 `crates/core/src/config.rs` 的 `ObserverConfig` 逐字段一致。
 * 金额/费率类字段为 `Decimal`，经 serde-str 以字符串序列化。
 */
export interface ObserverConfig {
  pairs: PairConfig[]
  poll_interval_ms: number
  http_timeout_ms: number
  stream_start_timeout_ms: number
  reconnect_delay_ms: number
  max_snapshot_age_ms: number
  max_pair_skew_ms: number
  orderbook_depth: number
  account_refresh_interval_ms: number
  max_fee_age_ms: number
  auth_recv_window_ms: number
  archive: {
    path: string
    queue_capacity: number
    raw_retention_days: number
  }
  binance: VenueConfig
  bybit: VenueConfig
  strategy: {
    min_net_profit: string
    min_net_bps: string
    latency_loss_bps: string
    risk_buffer_bps: string
    rebalance_cost: string
    other_direct_cost: string
  }
}

/**
 * 前端默认值模板：观察配置与后端 `ObserverConfig::default_config()` 一致，
 * 由前端持有并作为首次编辑/保存的基底。
 */
export const DEFAULT_OBSERVER_CONFIG: ObserverConfig = {
  pairs: [
    { symbol: 'BTCUSDT', base_asset: 'BTC', quote_asset: 'USDT', quantity: '0.001' },
  ],
  poll_interval_ms: 2000,
  http_timeout_ms: 3000,
  stream_start_timeout_ms: 10000,
  reconnect_delay_ms: 1000,
  max_snapshot_age_ms: 1000,
  max_pair_skew_ms: 500,
  orderbook_depth: 50,
  account_refresh_interval_ms: 300000,
  max_fee_age_ms: 900000,
  auth_recv_window_ms: 5000,
  archive: {
    path: 'data/archive/observations.ndjson',
    queue_capacity: 1024,
    raw_retention_days: 30,
  },
  binance: {
    base_url: 'https://api.binance.com',
    websocket_url: 'wss://stream.binance.com:9443',
    fallback_taker_fee_rate: '0.001',
    api_key_env: 'TAOLI_BINANCE_API_KEY',
    api_secret_env: 'TAOLI_BINANCE_API_SECRET',
    region_eligible_confirmed: false,
    account_eligible_confirmed: false,
  },
  bybit: {
    base_url: 'https://api.bybit.com',
    websocket_url: 'wss://stream.bybit.com/v5/public/spot',
    fallback_taker_fee_rate: '0.001',
    api_key_env: 'TAOLI_BYBIT_API_KEY',
    api_secret_env: 'TAOLI_BYBIT_API_SECRET',
    region_eligible_confirmed: false,
    account_eligible_confirmed: false,
  },
  strategy: {
    min_net_profit: '0.01',
    min_net_bps: '1',
    latency_loss_bps: '1',
    risk_buffer_bps: '1',
    rebalance_cost: '0',
    other_direct_cost: '0',
  },
}

/** 前端默认值模板：凭据域为空（连接串由用户按环境填写）。 */
export const DEFAULT_APP_CONFIG: AppConfig = {
  database_url: '',
  binance_api_key: '',
  binance_api_secret: '',
  bybit_api_key: '',
  bybit_api_secret: '',
}

/**
 * 统一 IPC 封装：解包 `ApiResponse<T>`，失败时按 `CODE: message` 弹错误提示后返回
 * `undefined`，成功时返回 `data`。
 *
 * 判失败仅看 `success` 字段，不把 `data === null` 当作失败：后端 `ApiResponse<()>`
 * 的命令（`save_app_config`）成功时 `data` 序列化为 `null`。
 */
export async function invokeCommand<T>(
  name: string,
  args: Record<string, unknown> = {},
): Promise<T | undefined> {
  try {
    const response = await invoke<ApiResponse<T>>(name, args)
    if (!response.success) {
      const error = response.error
      ElMessage.error(`${error?.code ?? 'INTERNAL_ERROR'}: ${error?.message ?? 'command failed'}`)
      return undefined
    }
    return response.data as T
  } catch (error) {
    ElMessage.error(`IPC_ERROR: ${String(error)}`)
    return undefined
  }
}

/** 获取当前生效的完整观察配置（json 优先，toml 兜底，内置默认兜底）。 */
export async function getObserverConfig(
  configPath?: string,
): Promise<ObserverConfig | undefined> {
  return invokeCommand<ObserverConfig>(COMMANDS.getObserverConfig, {
    configPath: configPath || null,
  })
}

/** 保存完整观察配置为 json，保存后即时生效（json 优先链）。返回是否成功。 */
export async function saveObserverConfig(
  config: ObserverConfig,
  configPath?: string,
): Promise<boolean> {
  const result = await invokeCommand<null>(COMMANDS.saveObserverConfig, {
    config: config,
    configPath: configPath || null,
  })
  // 成功时 data 序列化为 null（≠ undefined），失败时 invokeCommand 返回 undefined
  return result !== undefined
}