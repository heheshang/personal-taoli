<script setup lang="ts">
import { computed } from 'vue'
import { ElButton, ElCard, ElTable, ElTableColumn, ElTag } from 'element-plus'
import type { Account, Continuous, Observe } from '../types'
import { ACCOUNTING_CONTROL_KINDS, PAPER_KINDS } from '../commands'
import type { AccountingControlKind, PaperKind } from '../commands'
import { trimDecimal } from '../format'
import TermHint from './TermHint.vue'

/** PAPER 三层中文业务名与悬停解释（源自 B-01/B-02/B-03 迭代语义）。 */
const PAPER_LABELS: Record<PaperKind, { label: string; hint: string }> = {
  B01: {
    label: '预留验证',
    hint: '预留层 B01：验证账户—品种单写者锁、资金预留、幂等键与不可变审计；恢复后计划、预留、意图保持原值',
  },
  B02: {
    label: '订单事实验证',
    hint: '订单事实层 B02：验证提交/查单/撤单/成交先持久化、UNKNOWN 状态机、撤单竞态与重复成交审计、崩溃恢复',
  },
  B03: {
    label: '双腿执行验证',
    hint: '执行层 B03：验证双腿部分成交差额、未匹配敞口、预算内补偿计划、超限人工升级、版本化决定与重启加载',
  },
}

const props = defineProps<{
  accounts: Account[]
  observation: Observe[] | null
  report: unknown
  canOperate: boolean
  archivePath: string
  continuous: Continuous | null
  fmtTime: (ms: number | null) => string
}>()

const emit = defineEmits<{
  loadAccounts: []
  observe: []
  replay: []
  paper: [kind: PaperKind]
  accountingControl: [kind: AccountingControlKind]
  simulation: []
  startContinuous: []
  stopContinuous: []
  reconnectSmoke: []
}>()

const scanReport = computed(() => {
  const latest = props.observation?.[props.observation.length - 1]
  if (!latest?.report) return null
  const r = latest.report as Record<string, unknown>
  return {
    symbol: r.symbol as string,
    quantity: r.quantity as string,
    directions: (r.directions as Record<string, unknown>[]) || [],
  }
})

const paperReport = computed(() => {
  if (!props.report || typeof props.report !== 'object') return null
  const r = props.report as Record<string, unknown>
  if (r.kind && r.report) {
    const report = r.report as Record<string, unknown>
    return { kind: r.kind as string, ...report }
  }
  return null
})

/** F-02 模拟套利烟测报告视图（扁平结构，按 s01_* 探针识别）。 */
const simulationReport = computed(() => {
  if (!props.report || typeof props.report !== 'object') return null
  const r = props.report as Record<string, unknown>
  if (typeof r.s01_full_fill_at_worst !== 'boolean') return null
  return r
})

function isOpportunity(d: Record<string, unknown>): boolean {
  return d.accepted === true
}

function profitColor(val: unknown): string {
  const n = parseFloat(String(val ?? ''))
  if (isNaN(n)) return '#89958c'
  return n > 0 ? '#16825e' : n < 0 ? '#de5874' : '#89958c'
}

function statusTag(val: unknown): { type: 'success' | 'danger' | 'info' | 'warning'; text: string } {
  if (val === true) return { type: 'success', text: '通过' }
  if (val === false) return { type: 'danger', text: '未通过' }
  return { type: 'info', text: String(val ?? '—') }
}

function formatKey(key: string): string {
  const map: Record<string, string> = {
    schema_version: 'Schema 版本',
    same_domain_lock_rejected: '同域锁拒绝',
    concurrent_attempts: '并发尝试次数',
    concurrent_successes: '并发成功次数',
    concurrent_rejections: '并发拒绝次数',
    insufficient_request_rolled_back: '余额不足回滚',
    idempotent_replay: '幂等重放',
    conflicting_replay_rejected: '冲突重放拒绝',
    recovered_plans: '恢复计划数',
    recovered_risk_decisions: '恢复风控决策数',
    recovered_intents: '恢复意图数',
    recovered_reservations: '恢复预留数',
    recovered_audit_events: '恢复审计事件数',
    audit_events_immutable: '审计事件不可变',
    rejected_request_rows: '拒绝请求行数',
    all_recovered_intents_not_sent: '所有恢复意图未发送',
    all_recovered_funds_local_reserved: '所有恢复资金本地预留',
    external_order_calls: '外部订单调用次数',
    external_order_requests: '外部订单请求数',
    orders_submitted: '已提交订单数',
    orders_filled: '已成交订单数',
    orders_cancelled: '已撤销订单数',
    unknown_orders: '未知状态订单数',
    facts_recorded: '已记录事实数',
    legs_matched: '双腿匹配数',
    compensation_planned: '补偿已计划数',
    manual_required: '需人工介入数',
    ledger_events: '账务事件数',
    ledger_lines: '账务行数',
    balance_snapshots: '余额快照数',
    reconciliation_diffs: '对账差异数',
    matched_entries: '已匹配条目数',
    attention_required: '需关注条目数',
    control_commands: '控制命令数',
    pause_commands: '暂停命令数',
    resume_commands: '恢复命令数',
    stop_commands: '停止命令数',
    s01_full_fill_at_worst: 'S01 足额按最差档成交',
    s02_partial_fill_depth_shortfall: 'S02 深度不足部分成交',
    s03_competed_away: 'S03 敌手占盘无成交',
    s04_worse_than_scan_price: 'S04 成交价劣于扫码基线',
    s05_insufficient_funds_rejected: 'S05 资金不足拒绝',
    s06_unknown_query_recovered_no_duplicate: 'S06 UNKNOWN查询恢复无重复',
    s07_compensation_over_budget_manual: 'S07 补偿超预算人工介入',
    s08_idempotent_replay: 'S08 幂等重放',
    s09_fact_recovery_consistent: 'S09 事实恢复一致',
  }
  return map[key] || key.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())
}

function toTitle(s: string): string {
  if (!s) return s
  return s.charAt(0).toUpperCase() + s.slice(1)
}

function fmtDuration(ms: unknown): string {
  const n = Number(ms || 0)
  const totalSec = Math.max(0, Math.floor(n / 1000))
  const d = Math.floor(totalSec / 86400)
  const h = Math.floor((totalSec % 86400) / 3600)
  const m = Math.floor((totalSec % 3600) / 60)
  const s = totalSec % 60
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}

function fmtBytes(n: unknown): string {
  const v = Number(n || 0)
  if (v >= 1048576) return `${(v / 1048576).toFixed(2)} MiB`
  if (v >= 1024) return `${(v / 1024).toFixed(1)} KiB`
  return `${v} B`
}

/** 稳定拒绝类别后缀 → 中文标签（未知类别回退原文形式）。 */
const REJECTION_LABELS: Record<string, string> = {
  region_eligibility_unconfirmed: '区域资格未确认',
  account_eligibility_unconfirmed: '账户资格未确认',
  credentials_not_configured: '凭证未配置',
  actual_fee_unavailable: '实际费率不可用',
  admission_profit_below_minimum: '准入利润低于下限',
  admission_net_bps_below_minimum: '准入净基点低于下限',
  snapshot_receive_skew: '快照接收偏斜',
}

function rejectionLabel(key: string): string {
  const idx = key.indexOf(':')
  const venue = idx > 0 ? key.slice(0, idx) : ''
  const suffix = idx > 0 ? key.slice(idx + 1) : key
  const base = REJECTION_LABELS[suffix] ?? suffix.replace(/_/g, ' ')
  return venue ? `${toTitle(venue)} · ${base}` : base
}

const STATE_LABELS: Record<string, string> = {
  VALID: '有效',
  SYNCING: '同步中',
  STALE: '过期',
  INVALID: '失效',
}
const STATE_TYPES: Record<string, 'success' | 'warning' | 'danger' | 'info'> = {
  VALID: 'success',
  SYNCING: 'warning',
  STALE: 'danger',
  INVALID: 'danger',
}

/** 重连验证 ReconnectSmokeResult → 可读视图；识别失败返回 null（留给 raw JSON 兜底）。 */
const reconnectSmoke = computed(() => {
  const r = props.report as Record<string, unknown> | null
  if (!r || typeof r !== 'object' || Array.isArray(r)) return null
  if (!('no_orders' in r)) return null
  const rawResults = r.results
  const venues: Array<{
    venue: string
    symbol: string
    state: string
    stateLabel: string
    generation: number
    reconnects: number
    appliedUpdates: number
    reason: string | null
  }> = []
  if (Array.isArray(rawResults)) {
    for (const entry of rawResults) {
      if (!entry || typeof entry !== 'object') continue
      const e = entry as Record<string, unknown>
      for (const k of ['binance', 'bybit'] as const) {
        const v = e[k]
        if (!v || typeof v !== 'object') continue
        const o = v as Record<string, unknown>
        venues.push({
          venue: String(o.venue ?? k),
          symbol: String(e.symbol ?? o.symbol ?? '—'),
          state: String(o.state ?? ''),
          stateLabel: STATE_LABELS[String(o.state ?? '')] ?? String(o.state ?? '—'),
          generation: Number(o.generation ?? 0),
          reconnects: Number(o.reconnects ?? 0),
          appliedUpdates: Number(o.applied_updates ?? 0),
          reason: o.reason === null || o.reason === undefined ? null : String(o.reason),
        })
      }
    }
  }
  if (venues.length === 0) return null
  return { venues, noOrders: r.no_orders === true }
})
const shadowReport = computed(() => {
  const r = props.report as Record<string, unknown> | null
  if (!r || typeof r !== 'object' || Array.isArray(r)) return null
  const rejections = r.rejection_reason_counts
  const samples = r.tail_samples
  if (typeof rejections !== 'object' || rejections === null || Array.isArray(rejections)) return null
  if (!Array.isArray(samples)) return null
  const asU64 = (k: string) => Number(r[k] ?? 0)
  const dist = (k: string) => {
    const d = r[k]
    if (!d || typeof d !== 'object') return null
    const o = d as Record<string, unknown>
    return { min: o.minimum, p50: o.p50, p95: o.p95, max: o.maximum }
  }
  const distOf = (k: string, pick: Array<[string, string]>) => {
    const d = r[k]
    if (!d || typeof d !== 'object') return null
    const o = d as Record<string, unknown>
    return Object.fromEntries(pick.map(([to, from]) => [to, o[from]]))
  }
  return {
    records: asU64('records'),
    decisionRecords: asU64('decision_records'),
    healthRecords: asU64('health_records'),
    directionEvaluations: asU64('direction_evaluations'),
    positiveNet: asU64('positive_net_opportunities'),
    accepted: asU64('accepted_opportunities'),
    gapRecords: asU64('gap_records'),
    droppedEvents: asU64('dropped_events'),
    replayedWithoutMismatch: r.replayed_without_mismatch === true,
    archivePath: String(r.archive_path ?? ''),
    archiveBytes: fmtBytes(r.archive_bytes),
    observed: fmtDuration(r.observed_duration_ms),
    online: fmtDuration(r.online_duration_ms),
    invalid: fmtDuration(r.invalid_duration_ms),
    reconnectTotal: Object.values((r.reconnects as Record<string, unknown>) ?? {}).reduce<number>(
      (a, b) => a + Number(b || 0),
      0,
    ),
    rejections: Object.entries(rejections as Record<string, unknown>).map(([k, v]) => ({
      label: rejectionLabel(k),
      count: Number(v ?? 0),
    })),
    samples: (samples as Record<string, unknown>[]).map(s => ({
      buy: toTitle(String(s.buy_venue ?? '')),
      sell: toTitle(String(s.sell_venue ?? '')),
      profit: trimDecimal(s.admission_profit),
      bps: trimDecimal(s.admission_net_bps),
    })),
    netProfit: dist('net_profit_distribution'),
    capacity: distOf('visible_capacity', [
      ['min', 'minimum_base_quantity'],
      ['p50', 'p50_base_quantity'],
      ['p95', 'p95_base_quantity'],
      ['max', 'maximum_base_quantity'],
    ]),
  }
})
</script>

<template>
  <section class="detail-panel">

    <!-- 页首：控制台标题 + 连续观测会话状态 -->
    <div class="panel-head">
      <div class="control-title">
        <span class="eyebrow">控制台</span>
        <b>只读操作</b>
      </div>
      <div v-if="continuous" class="control-session">
        <template v-if="continuous.running">
          <i class="pulse" />
          <span>连续观测 <b>运行中</b> · {{ continuous.archive_path || '默认归档' }} · 自 {{ fmtTime(continuous.started_at_ms) }}</span>
        </template>
        <template v-else>
          <span>连续观测 <b>已停止</b>{{ continuous.error ? ' — ' + continuous.error : '' }}</span>
        </template>
      </div>
    </div>

    <!-- 账户状态表 -->
    <ElCard shadow="never">
      <template #header>
        <span class="card-title">账户状态</span>
        <ElButton size="small" :disabled="!canOperate" @click="emit('loadAccounts')">刷新</ElButton>
      </template>
      <ElTable v-if="accounts.length" :data="accounts" stripe>
        <ElTableColumn prop="venue" width="120">
          <template #header><TermHint term="交易所" /></template>
        </ElTableColumn>
        <ElTableColumn prop="fee.symbol" width="110">
          <template #header><TermHint term="币种" /></template>
        </ElTableColumn>
        <ElTableColumn>
          <template #header><TermHint term="费率来源" /></template>
          <template #default="{ row }">
            {{ row.fee.source }} / 买: {{ trimDecimal(row.fee.buy_taker_rate) }} / 卖: {{ trimDecimal(row.fee.sell_taker_rate) }}
          </template>
        </ElTableColumn>
        <ElTableColumn>
          <template #header><TermHint term="拒绝原因" /></template>
          <template #default="{ row }">
            {{ row.rejection_reasons.join(' · ') || '无' }}
          </template>
        </ElTableColumn>
      </ElTable>
      <div v-else class="empty-state">尚未查询账户状态</div>
    </ElCard>

    <!-- 观测扫描报告 -->
    <ElCard shadow="never">
      <template #header>
        <span class="card-title">扫描报告{{ scanReport ? ` · ${scanReport.symbol} · 数量 ${scanReport.quantity}` : ' — 尚未执行观测' }}{{ observation?.length ? ` · 共 ${observation.length} 对` : '' }}</span>
        <div class="card-actions">
          <ElButton type="primary" size="small" :disabled="!canOperate" @click="emit('observe')">执行观测</ElButton>
          <ElButton size="small" :disabled="!canOperate || continuous?.running" @click="emit('startContinuous')">启动连续</ElButton>
          <ElButton size="small" :disabled="!canOperate || !continuous?.running" @click="emit('stopContinuous')">停止连续</ElButton>
        </div>
      </template>
      <div v-if="!scanReport" class="empty-state">尚未执行观测</div>
      <div v-else-if="scanReport.directions.length === 0" class="empty-state">暂无套利机会</div>
      <div v-else class="opportunity-grid">
        <div
          v-for="(dir, i) in scanReport.directions"
          :key="i"
          class="opportunity-card"
          :class="{ accepted: isOpportunity(dir) }"
        >
          <div class="opp-header">
            <span class="opp-direction">
              <b>{{ dir.buy_venue }}</b> 买 → <b>{{ dir.sell_venue }}</b> 卖
            </span>
            <ElTag :type="isOpportunity(dir) ? 'success' : 'info'" size="small">
              {{ isOpportunity(dir) ? '准入' : '拒绝' }}
            </ElTag>
          </div>
          <div class="opp-metrics">
            <div class="opp-metric">
              <small><TermHint term="买入 VWAP" /></small>
              <strong>{{ trimDecimal(dir.buy_vwap) }}</strong>
            </div>
            <div class="opp-metric">
              <small><TermHint term="卖出 VWAP" /></small>
              <strong>{{ trimDecimal(dir.sell_vwap) }}</strong>
            </div>
            <div class="opp-metric">
              <small><TermHint term="毛利" /></small>
              <strong :style="{ color: profitColor(dir.gross_profit) }">{{ trimDecimal(dir.gross_profit) }}</strong>
            </div>
            <div class="opp-metric">
              <small><TermHint term="手续费" /></small>
              <strong>{{ trimDecimal(dir.fees) }}</strong>
            </div>
            <div class="opp-metric">
              <small><TermHint term="预期净收益" /></small>
              <strong :style="{ color: profitColor(dir.expected_net_profit) }">{{ trimDecimal(dir.expected_net_profit) }}</strong>
            </div>
            <div class="opp-metric">
              <small><TermHint term="准入净收益" /></small>
              <strong :style="{ color: profitColor(dir.admission_profit) }">{{ trimDecimal(dir.admission_profit) }}</strong>
            </div>
          </div>
          <div v-if="(dir.rejection_reasons as string[])?.length" class="opp-reasons">
            <small><TermHint term="拒绝原因" />: {{ (dir.rejection_reasons as string[]).join(' · ') }}</small>
          </div>
        </div>
      </div>
    </ElCard>

    <!-- PAPER / 账务控制报告 -->
    <ElCard shadow="never">
      <template #header>
        <span class="card-title">
          {{ paperReport && paperReport.kind === 'B01' ? 'PAPER 预留验证' :
             paperReport && paperReport.kind === 'B02' ? 'PAPER 订单事实验证' :
             paperReport && paperReport.kind === 'B03' ? 'PAPER 双腿执行验证' :
             paperReport && paperReport.kind === 'ACCOUNTING' ? '账务验证' :
             paperReport && paperReport.kind === 'RECONCILIATION' ? '对账验证' :
             paperReport && paperReport.kind === 'CONTROL' ? '控制面验证' :
             simulationReport ? 'F-02 模拟套利验证' : '验证报告' }}
        </span>
        <div class="card-actions">
          <ElButton size="small" :disabled="!canOperate || !archivePath" @click="emit('replay')">回放归档</ElButton>
          <ElButton v-for="kind in PAPER_KINDS" :key="kind" size="small" :disabled="!canOperate" :title="PAPER_LABELS[kind].hint" @click="emit('paper', kind)">{{ PAPER_LABELS[kind].label }}</ElButton>
          <ElButton v-for="kind in ACCOUNTING_CONTROL_KINDS" :key="kind" size="small" :disabled="!canOperate" @click="emit('accountingControl', kind)">{{ kind === 'ACCOUNTING' ? '账务' : kind === 'RECONCILIATION' ? '对账' : '控制' }}</ElButton>
          <ElButton size="small" :disabled="!canOperate" title="F-02 模拟套利：合成簿驱动 S01–S09 撮合烟测，全程无真实/测试网订单（external_order_calls=0）" @click="emit('simulation')">模拟套利</ElButton>
          <ElButton size="small" :disabled="!canOperate" @click="emit('reconnectSmoke')">重连验证</ElButton>
        </div>
      </template>
      <div v-if="paperReport" class="report-grid">
        <template v-for="(val, key) in paperReport" :key="key">
          <div v-if="key !== 'kind'" class="report-item">
            <small><TermHint :term="formatKey(String(key))" /></small>
            <ElTag v-if="typeof val === 'boolean'" v-bind="statusTag(val)" size="small">
              {{ statusTag(val).text }}
            </ElTag>
            <strong v-else>{{ trimDecimal(val) }}</strong>
          </div>
        </template>
      </div>
      <div v-else-if="simulationReport" class="report-grid">
        <template v-for="(val, key) in simulationReport" :key="key">
          <div class="report-item">
            <small><TermHint :term="formatKey(String(key))" /></small>
            <ElTag v-if="typeof val === 'boolean'" v-bind="statusTag(val)" size="small">
              {{ statusTag(val).text }}
            </ElTag>
            <strong v-else>{{ trimDecimal(val) }}</strong>
          </div>
        </template>
        <small class="smoke-note">
          只读烟测 · 未产生任何真实/测试网订单（{{ String(simulationReport.external_order_calls ?? 0) }}）
        </small>
      </div>
      <div v-else-if="reconnectSmoke" class="reconnect-grid">
        <div v-for="v in reconnectSmoke.venues" :key="v.venue" class="venue-state-card">
          <div class="vs-head">
            <b>{{ toTitle(v.venue) }}</b>
            <ElTag :type="STATE_TYPES[v.state] ?? 'info'" size="small">{{ v.stateLabel }}</ElTag>
          </div>
          <div class="vs-metrics">
            <div class="vs-metric">
              <small><TermHint term="标的" /></small>
              <strong>{{ v.symbol }}</strong>
            </div>
            <div class="vs-metric">
              <small><TermHint term="代数" /></small>
              <strong>{{ v.generation }}</strong>
            </div>
            <div class="vs-metric">
              <small><TermHint term="重连" /></small>
              <strong>{{ v.reconnects }}</strong>
            </div>
            <div class="vs-metric">
              <small><TermHint term="应用更新" /></small>
              <strong>{{ v.appliedUpdates }}</strong>
            </div>
          </div>
          <small v-if="v.reason" class="vs-reason">原因：{{ v.reason }}</small>
        </div>
        <small class="smoke-note">只读烟测{{ reconnectSmoke.noOrders ? ' · 未产生任何订单' : '' }}</small>
      </div>
      <div v-else-if="shadowReport" class="shadow-report">
        <div class="report-grid">
          <div class="report-item">
            <small><TermHint term="归档记录" /></small>
            <strong>{{ shadowReport.records }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="决策记录" /></small>
            <strong>{{ shadowReport.decisionRecords }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="健康记录" /></small>
            <strong>{{ shadowReport.healthRecords }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="方向评估" /></small>
            <strong>{{ shadowReport.directionEvaluations }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="正净收益机会" /></small>
            <strong :style="{ color: profitColor(shadowReport.positiveNet) }">{{ shadowReport.positiveNet }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="准入机会" /></small>
            <strong :style="{ color: profitColor(shadowReport.accepted) }">{{ shadowReport.accepted }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="重连次数" /></small>
            <strong>{{ shadowReport.reconnectTotal }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="缺口记录" /></small>
            <strong>{{ shadowReport.gapRecords }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="丢弃事件" /></small>
            <strong>{{ shadowReport.droppedEvents }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="归档大小" /></small>
            <strong>{{ shadowReport.archiveBytes }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="观测周期" /></small>
            <strong>{{ shadowReport.observed }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="在线时长" /></small>
            <strong>{{ shadowReport.online }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="失效时长" /></small>
            <strong>{{ shadowReport.invalid }}</strong>
          </div>
          <div class="report-item">
            <small><TermHint term="重放一致" /></small>
            <ElTag :type="shadowReport.replayedWithoutMismatch ? 'success' : 'danger'" size="small">
              {{ shadowReport.replayedWithoutMismatch ? '一致' : '不一致' }}
            </ElTag>
          </div>
          <div class="report-item path-item">
            <small><TermHint term="归档路径" /></small>
            <strong class="path">{{ shadowReport.archivePath }}</strong>
          </div>
        </div>

        <div v-if="shadowReport.netProfit" class="report-block">
          <small class="block-title"><TermHint term="净收益分布（原始 Decimal）" /></small>
          <div class="rc-rows">
            <div class="rc-row"><span>最小</span><strong>{{ trimDecimal(shadowReport.netProfit.min) }}</strong></div>
            <div class="rc-row"><span><TermHint term="P50" /></span><strong>{{ trimDecimal(shadowReport.netProfit.p50) }}</strong></div>
            <div class="rc-row"><span><TermHint term="P95" /></span><strong>{{ trimDecimal(shadowReport.netProfit.p95) }}</strong></div>
            <div class="rc-row"><span>最大</span><strong>{{ trimDecimal(shadowReport.netProfit.max) }}</strong></div>
          </div>
        </div>

        <div v-if="shadowReport.capacity" class="report-block">
          <small class="block-title"><TermHint term="可见容量（基础数量）" /></small>
          <div class="rc-rows">
            <div class="rc-row"><span>最小</span><strong>{{ trimDecimal(shadowReport.capacity.min) }}</strong></div>
            <div class="rc-row"><span><TermHint term="P50" /></span><strong>{{ trimDecimal(shadowReport.capacity.p50) }}</strong></div>
            <div class="rc-row"><span><TermHint term="P95" /></span><strong>{{ trimDecimal(shadowReport.capacity.p95) }}</strong></div>
            <div class="rc-row"><span>最大</span><strong>{{ trimDecimal(shadowReport.capacity.max) }}</strong></div>
          </div>
        </div>

        <div v-if="shadowReport.rejections.length" class="report-block">
          <small class="block-title"><TermHint term="拒绝原因计数" /></small>
          <div v-for="rc in shadowReport.rejections" :key="rc.label" class="rc-row">
            <span>{{ rc.label }}</span>
            <strong>{{ rc.count }}</strong>
          </div>
        </div>

        <div v-if="shadowReport.samples.length" class="report-block">
          <small class="block-title"><TermHint term="最近尾部样本" /></small>
          <div v-for="(s, i) in shadowReport.samples" :key="i" class="rc-row">
            <span>{{ s.buy }} ➜ {{ s.sell }}</span>
            <strong>{{ s.profit }} · {{ s.bps }} bps</strong>
          </div>
        </div>
      </div>
      <div v-else-if="report && !paperReport" class="empty-state">
        <pre class="raw-report">{{ JSON.stringify(report, null, 2) }}</pre>
      </div>
      <div v-else class="empty-state">尚未运行验证操作</div>
    </ElCard>

    <small class="control-note">回放需要已有归档文件；PAPER 和账务控制为数据库验证操作。</small>
  </section>
</template>

<style scoped>
.card-title {
  font-size: var(--fs-13);
  font-weight: 700;
  color: var(--text-1);
  letter-spacing: .03em;
}
.panel-head {
  min-height: 0;
  padding: 0 2px 4px;
  display: flex;
  justify-content: space-between;
  align-items: flex-end;
  gap: 12px;
}
.panel-head .control-title b { margin-top: 4px; }
.panel-head .control-session {
  flex-basis: auto;
  margin-top: 0;
  text-align: right;
}
.detail-panel :deep(.el-card__header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 11px 16px;
}
.card-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 6px;
}
.card-actions .el-button {
  border-color: var(--border-strong);
  background: var(--bg-card);
  color: var(--text-2);
  border-radius: var(--radius-sm);
  transition: all .15s;
}
.card-actions .el-button:hover { border-color: var(--accent); color: var(--accent); }
.card-actions .el-button--primary { background: var(--accent); border-color: var(--accent); color: white; }
.card-actions .el-button--primary:hover { background: var(--accent-2); border-color: var(--accent-2); color: white; }
.card-actions .el-button.is-disabled { background: #eef1ec; color: var(--text-3); }
.detail-panel .control-note { margin-top: 8px; }
.empty-state {
  padding: 24px;
  text-align: center;
  color: var(--text-3);
  font-size: var(--fs-12);
}
.opportunity-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
  gap: 12px;
}
.opportunity-card {
  padding: 15px 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
  transition: border-color .18s, box-shadow .18s;
}
.opportunity-card:hover {
  border-color: var(--border-strong);
  box-shadow: var(--shadow-1);
}
.opportunity-card.accepted {
  border-color: #9cc9b0;
  background: var(--accent-soft);
}
.opp-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.opp-direction {
  font-size: var(--fs-12);
  color: var(--text-2);
  font-weight: 600;
}
.opp-direction b {
  color: var(--accent);
  font-family: var(--font-mono);
}
.opp-metrics {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}
@media (max-width: 900px) {
  .opp-metrics {
    grid-template-columns: repeat(2, 1fr);
  }
}
.opp-metric small {
  display: block;
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: .05em;
  text-transform: uppercase;
  margin-bottom: 4px;
}
.opp-metric strong {
  font-size: var(--fs-13);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.opp-reasons {
  margin-top: 10px;
  padding-top: 9px;
  border-top: 1px dotted var(--border);
}
.opp-reasons small {
  color: var(--warning);
  font-size: var(--fs-10);
}
.report-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 14px;
}
.report-item {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.report-item small {
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: .04em;
  text-transform: uppercase;
}
.report-item strong {
  font-size: var(--fs-12);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.raw-report {
  max-height: 280px;
  overflow: auto;
  margin: 10px 0 0;
  padding: 14px;
  border-radius: var(--radius-sm);
  background: var(--bg-sidebar);
  color: #d7f0df;
  font-size: var(--fs-12);
  font-family: var(--font-mono);
  line-height: 1.5;
}
.detail-panel :deep(.el-table) {
  --el-table-header-bg-color: var(--bg-inset);
  --el-table-row-hover-bg-color: #eef4ef;
  --el-table-border-color: var(--border);
  font-size: var(--fs-12);
}
.detail-panel :deep(.el-table th.el-table__cell) {
  color: var(--text-2);
  font-weight: 700;
  letter-spacing: .03em;
}
.shadow-report {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.report-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.block-title {
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: .08em;
}
.rc-rows {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.rc-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 7px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-inset);
}
.rc-row span {
  color: var(--text-2);
  font-size: var(--fs-11);
}
.rc-row strong {
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: var(--fs-11);
}
.path-item {
  grid-column: 1 / -1;
}
.report-item .path {
  font-family: var(--font-mono);
  font-size: var(--fs-10);
  font-weight: 500;
  word-break: break-all;
}
.reconnect-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: 12px;
}
.venue-state-card {
  padding: 14px 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
}
.vs-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 10px;
}
.vs-head b {
  font-size: var(--fs-13);
  letter-spacing: .04em;
}
.vs-metrics {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}
.vs-metric {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.vs-metric small {
  color: var(--text-3);
  font-size: var(--fs-10);
}
.vs-metric strong {
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: var(--fs-12);
}
.vs-reason {
  display: block;
  margin-top: 10px;
  color: var(--text-3);
  font-size: var(--fs-10);
  word-break: break-all;
}
.smoke-note {
  grid-column: 1 / -1;
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: .05em;
}
</style>
