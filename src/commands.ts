import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import type {
  ApiResponse,
  SimulationOverview,
  SimulationRunDetail,
  SimulationRunsPage,
} from './types'

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
  runSimulationSmoke: 'run_simulation_smoke_command',
  getSimulationOverview: 'get_simulation_overview_command',
  getSimulationRuns: 'get_simulation_runs_command',
  getSimulationRunDetail: 'get_simulation_run_detail_command',
  startContinuousObservation: 'start_continuous_observation',
  stopContinuousObservation: 'stop_continuous_observation',
  continuousObservationStatus: 'continuous_observation_status',
  runReconnectSmoke: 'run_reconnect_smoke',
  loadAppConfig: 'load_app_config',
  saveAppConfig: 'save_app_config',
  getObserverConfig: 'get_observer_config',
  saveObserverConfig: 'save_observer_config',
  agentReady: 'agent_ready',
  agentStart: 'agent_start',
  agentAsk: 'agent_ask',
  agentDecide: 'agent_decide',
  agentStatus: 'agent_status',
  agentStop: 'agent_stop',
  agentDefaultPrompt: 'agent_default_prompt',
  agentAccessLevels: 'agent_access_levels',
  agentSetAccessLevel: 'agent_set_access_level',
} as const

export const PAPER_KINDS = ['B01', 'B02', 'B03'] as const
export type PaperKind = (typeof PAPER_KINDS)[number]

export const ACCOUNTING_CONTROL_KINDS = ['ACCOUNTING', 'RECONCILIATION', 'CONTROL'] as const
export type AccountingControlKind = (typeof ACCOUNTING_CONTROL_KINDS)[number]

/** F-02 模拟套利烟测结果，与 `dto.rs` 的 `SimulationSmokeResult` 一致。 */
export interface SimulationSmokeResult {
  schema_version: number
  s01_full_fill_at_worst: boolean
  s02_partial_fill_depth_shortfall: boolean
  s03_competed_away: boolean
  s04_worse_than_scan_price: boolean
  s05_insufficient_funds_rejected: boolean
  s06_unknown_query_recovered_no_duplicate: boolean
  s07_compensation_over_budget_manual: boolean
  s08_idempotent_replay: boolean
  s09_fact_recovery_consistent: boolean
  external_order_calls: number
}

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
  simulation: {
    enabled: boolean
    seed: number
    initial_quote_balance: string
    initial_base_balance: string
    decision_to_submit_ms_min: number
    decision_to_submit_ms_max: number
    fill_latency_ms_min: number
    fill_latency_ms_max: number
    adverse_move_bps: string
    competitor_take_bps: string
    unknown_submit_probability_bps: string
    query_found_probability_bps: string
    max_unmatched_exposure: string
    compensation_budget: string
    compensation_markup_bps: string
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
  simulation: {
    enabled: false,
    seed: 0,
    initial_quote_balance: '10000',
    initial_base_balance: '2',
    decision_to_submit_ms_min: 50,
    decision_to_submit_ms_max: 400,
    fill_latency_ms_min: 20,
    fill_latency_ms_max: 250,
    adverse_move_bps: '15',
    competitor_take_bps: '200',
    unknown_submit_probability_bps: '300',
    query_found_probability_bps: '6000',
    max_unmatched_exposure: '100',
    compensation_budget: '20',
    compensation_markup_bps: '30',
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

/** F-02 模拟套利烟测（S01–S09，PostgreSQL 合成簿场景）。 */
export async function runSimulationSmoke(
  databaseUrl?: string,
): Promise<SimulationSmokeResult | undefined> {
  return invokeCommand<SimulationSmokeResult>(COMMANDS.runSimulationSmoke, {
    databaseUrlParam: databaseUrl || null,
  })
}

/** G-01 模拟套利仪表盘概览（聚合 + 最近列表 + 余额曲线 + 山脊 + 活动流）。 */
export async function getSimulationOverview(
  databaseUrl?: string,
): Promise<SimulationOverview | undefined> {
  return invokeCommand<SimulationOverview>(COMMANDS.getSimulationOverview, {
    databaseUrlParam: databaseUrl || null,
  })
}

/** G-01 run 分页列表（limit ≤ 200；symbol / scenario 可选过滤）。 */
export async function getSimulationRuns(
  options: {
    limit?: number
    offset?: number
    symbol?: string | null
    scenario?: string | null
    databaseUrl?: string
  } = {},
): Promise<SimulationRunsPage | undefined> {
  return invokeCommand<SimulationRunsPage>(COMMANDS.getSimulationRuns, {
    databaseUrlParam: options.databaseUrl || null,
    limit: options.limit ?? 100,
    offset: options.offset ?? 0,
    symbol: options.symbol ?? null,
    scenario: options.scenario ?? null,
  })
}

/** G-01 单 run 详情（报告全文 + 意图/成交/执行事件/审计/余额快照）。 */
export async function getSimulationRunDetail(
  runId: string,
  databaseUrl?: string,
): Promise<SimulationRunDetail | undefined> {
  return invokeCommand<SimulationRunDetail>(COMMANDS.getSimulationRunDetail, {
    databaseUrlParam: databaseUrl || null,
    runId,
  })
}
// ── PORT-01 codex 接入（只读分析）────────────────────────────────────────

/** 接入就绪状态。`ready = false` 时 `reason` 给出可操作原因。 */
export interface AgentReady {
  ready: boolean
  program: string | null
  trace_dir: string | null
  /** 沙箱可用性的人类可读描述。不可用时 `reason` 同时给出说明。 */
  sandbox: string
  /** 领域指令的规模描述（来自受版本控制的 `docs/agent/instructions.md`）。 */
  instructions: string | null
  /** 已配置的额外 skill 目录（来自 `TAOLI_AGENT_SKILL_ROOTS`）。 */
  skill_roots: string[]
  /** 已授权的额外可写根（来自 `TAOLI_AGENT_WRITE_ROOTS`）。 */
  write_roots: string[]
  /** 新会话将使用的操作档位。 */
  access_level: string
  reason: string | null
}

/**
/**
 * 一个可选的操作档位。
 *
 * 字段名与后端 serde 输出逐字一致（`consults_client` / `confined`）：改成
 * camelCase 会让读取永远得到 `undefined`，而 `undefined` 在布尔位置是 falsy ——
 * 于是「是否绕过审批」的判断会静默走错分支。
 *
 * 标签与说明取自 Codex 桌面版语言包的同一组词，使两个界面用同样的话描述同一件事。
 */
export interface AgentAccessLevel {
  token: string
  label: string
  description: string
  /** 该档位是否会把审批请求交给本页。 */
  consults_client: boolean
  /** 该档位是否仍受本项目沙箱约束；`完全访问权限` 为 false。 */
  confined: boolean
}

/** 运行时报告可用的一个 skill。名称可能带命名空间前缀，如 `stock-deep-analyzer:uzi`。 */
export interface AgentSkill {
  name: string
  description: string
}

/** 待裁决的审批请求。 */
export interface AgentApprovalRequest {
  request_id: number
  kind: string
  summary: string
  advertised: string[]
  /** 本请求可用的裁决项，顺序即建议顺序。 */
  options: AgentDecision[]
}

/**
 * 裁决 token，对应 Codex 桌面版审批卡的动作：允许一次 / 允许此对话 / 始终允许 / 拒绝。
 *
 * `allow_always` 只在宿主允许运行时写自己的规则目录时才会出现。本页的宿主不允许
 * （否则一次点击会改掉这台机器上所有 codex 会话的行为），故实际只会收到三个值。
 */
export type AgentDecision = 'allow_once' | 'allow_for_session' | 'allow_always' | 'deny'

/** 裁决的中文标签，取自 Codex 桌面版语言包的同一组文案。 */
export const AGENT_DECISION_LABEL: Record<AgentDecision, string> = {
  allow_once: '允许一次',
  allow_for_session: '允许此对话',
  allow_always: '始终允许',
  deny: '拒绝',
}

/** 已裁决的审批记录。 */
export interface AgentApprovalDecision {
  request_id: number
  kind: string
  summary: string
  decision: AgentDecision
  source: 'decider' | 'timeout' | 'unsupported'
  waited_ms: number
}

export interface AgentToolCall {
  call_id: string
  tool: string
  success: boolean
  output: string
}

/** 一轮分析的完整记录。 */
export interface AgentTurn {
  seq: number
  prompt: string
  tool_calls: AgentToolCall[]
  approvals: AgentApprovalDecision[]
  refused_requests: string[]
  /** 本轮全部 agent 消息，按时间顺序；最后一条是其结论。 */
  messages: string[]
  /**
   * 本轮的结构化报告；仅当本轮被要求按 schema 输出**且**结果通过校验时才有。
   *
   * 字段形状见 `docs/agent/report-schema.json`。注意该 schema 在本运行时是
   * **建议性**的（实测模型有时会用自己的字段名），因此后端会校验，不通过时
   * 此字段为 null，界面回退到 `messages`。
   */
  report: Record<string, unknown> | null
  /** 要求了结构化输出却未通过校验的原因。 */
  report_error: string | null
  /** 本轮失败原因；`null` 表示本轮正常结束。 */
  error: string | null
  started_at_ms: number
  finished_at_ms: number | null
}

/** 会话生命周期快照。 */
export interface AgentStatus {
  phase: 'stopped' | 'starting' | 'ready' | 'running' | 'awaiting_approval' | 'error'
  program: string | null
  thread_id: string | null
  trace_path: string | null
  pending_approval: AgentApprovalRequest | null
  /** 已完成与进行中的轮次，按时间升序。 */
  turns: AgentTurn[]
  /** 运行时报告可用的 skill（注册生效与否由运行时说了算）。 */
  skills: AgentSkill[]
  /** 当前会话使用的操作档位。 */
  access_level: string
  /** 正在进行的轮次的实时视图；无进行中轮次时为 null。 */
  live: AgentLive | null
  /** 会话级致命错误；轮次内的失败记在该轮的 `error`。 */
  error: string | null
}

/** 进行中一轮里的一步。 */
export interface AgentLiveItem {
  item_id: string
  /** `reasoning` / `command` / `file_change` / `tool_call` / `message` / `other`。 */
  kind: string
  /** 一行标题：命令行走命令行，工具调用走工具名。 */
  title: string
  detail: string | null
  /** `running` / `completed` / `failed` / `declined`。 */
  state: string
  /** 已流式收到的输出（后端保留尾部）。 */
  output: string
  exit_code: number | null
  duration_ms: number | null
}

/** 本轮 token 计费。 */
export interface AgentTokens {
  input_tokens: number
  cached_input_tokens: number
  output_tokens: number
  reasoning_output_tokens: number
  total_tokens: number
  context_window: number | null
}

/** 正在进行的轮次的实时视图。 */
export interface AgentLive {
  turn_seq: number
  /** `thinking` / `working` / `writing` / `awaiting_approval` / `done`。 */
  stage: string
  reasoning: string
  message: string
  items: AgentLiveItem[]
  tokens: AgentTokens | null
}

/** 是否已配置 codex 运行时、归档与 trace 目录（未就绪时界面给出说明）。 */
export async function agentReady(): Promise<AgentReady | undefined> {
  return invokeCommand<AgentReady>(COMMANDS.agentReady)
}

/** 启动分析会话（拉起 codex 运行时并建立线程）。 */
export async function agentStart(): Promise<AgentStatus | undefined> {
  return invokeCommand<AgentStatus>(COMMANDS.agentStart)
}

/** 发起一轮只读分析。 */
export async function agentAsk(prompt: string): Promise<AgentStatus | undefined> {
  return invokeCommand<AgentStatus>(COMMANDS.agentAsk, { prompt })
}

/** 裁决待审批动作。`decision` 必须是该请求 `options` 中的一项。 */
export async function agentDecide(
  requestId: number,
  decision: AgentDecision,
): Promise<AgentStatus | undefined> {
  return invokeCommand<AgentStatus>(COMMANDS.agentDecide, { requestId, decision })
}

export async function agentStatus(): Promise<AgentStatus | undefined> {
  return invokeCommand<AgentStatus>(COMMANDS.agentStatus)
}

export async function agentStop(): Promise<AgentStatus | undefined> {
  return invokeCommand<AgentStatus>(COMMANDS.agentStop)
}

export async function agentDefaultPrompt(): Promise<string | undefined> {
  return invokeCommand<string>(COMMANDS.agentDefaultPrompt)
}

// ── 操作档位（PORT-01-L）────────────────────────────────────────────────

/** 可选档位清单。 */
export async function agentAccessLevels(): Promise<AgentAccessLevel[] | undefined> {
  return invokeCommand<AgentAccessLevel[]>(COMMANDS.agentAccessLevels)
}

/** 设置新会话使用的档位。会话运行中改档会被后端拒绝，需先停止会话。 */
export async function agentSetAccessLevel(level: string): Promise<AgentStatus | undefined> {
  return invokeCommand<AgentStatus>(COMMANDS.agentSetAccessLevel, { level })
}
