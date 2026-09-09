<script setup lang="ts">
import { computed } from 'vue'
import { ElCard, ElTable, ElTableColumn, ElTag } from 'element-plus'
import type { Account, Observe } from '../types'

const props = defineProps<{
  accounts: Account[]
  observation: Observe | null
  report: unknown
}>()

const scanReport = computed(() => {
  if (!props.observation?.report) return null
  const r = props.observation.report as Record<string, unknown>
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
  }
  return map[key] || key.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())
}
</script>

<template>
  <section v-if="accounts.length || observation || report" class="detail-panel">

    <!-- 账户状态表 -->
    <ElCard v-if="accounts.length" shadow="never">
      <template #header>
        <span class="card-title">账户状态</span>
      </template>
      <ElTable :data="accounts" stripe>
        <ElTableColumn prop="venue" label="交易所" width="120" />
        <ElTableColumn label="费率来源">
          <template #default="{ row }">
            {{ row.fee.source }} / 买: {{ row.fee.buy_taker_rate }} / 卖: {{ row.fee.sell_taker_rate }}
          </template>
        </ElTableColumn>
        <ElTableColumn label="拒绝原因">
          <template #default="{ row }">
            {{ row.rejection_reasons.join(' · ') || '无' }}
          </template>
        </ElTableColumn>
      </ElTable>
    </ElCard>

    <!-- 观测扫描报告 -->
    <ElCard v-if="scanReport" shadow="never">
      <template #header>
        <span class="card-title">扫描报告 · {{ scanReport.symbol }} · 数量 {{ scanReport.quantity }}</span>
      </template>
      <div v-if="scanReport.directions.length === 0" class="empty-state">暂无套利机会</div>
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
              <small>买入 VWAP</small>
              <strong>{{ dir.buy_vwap }}</strong>
            </div>
            <div class="opp-metric">
              <small>卖出 VWAP</small>
              <strong>{{ dir.sell_vwap }}</strong>
            </div>
            <div class="opp-metric">
              <small>毛利</small>
              <strong :style="{ color: profitColor(dir.gross_profit) }">{{ dir.gross_profit }}</strong>
            </div>
            <div class="opp-metric">
              <small>手续费</small>
              <strong>{{ dir.fees }}</strong>
            </div>
            <div class="opp-metric">
              <small>预期净收益</small>
              <strong :style="{ color: profitColor(dir.expected_net_profit) }">{{ dir.expected_net_profit }}</strong>
            </div>
            <div class="opp-metric">
              <small>准入净收益</small>
              <strong :style="{ color: profitColor(dir.admission_profit) }">{{ dir.admission_profit }}</strong>
            </div>
          </div>
          <div v-if="(dir.rejection_reasons as string[])?.length" class="opp-reasons">
            <small>拒绝原因: {{ (dir.rejection_reasons as string[]).join(' · ') }}</small>
          </div>
        </div>
      </div>
    </ElCard>

    <!-- PAPER / 账务控制报告 -->
    <ElCard v-if="paperReport" shadow="never">
      <template #header>
        <span class="card-title">
          {{ paperReport.kind === 'B01' ? 'PAPER 预留验证' :
             paperReport.kind === 'B02' ? 'PAPER 订单事实验证' :
             paperReport.kind === 'B03' ? 'PAPER 双腿执行验证' :
             paperReport.kind === 'ACCOUNTING' ? '账务验证' :
             paperReport.kind === 'RECONCILIATION' ? '对账验证' :
             paperReport.kind === 'CONTROL' ? '控制面验证' : '验证报告' }}
        </span>
      </template>
      <div class="report-grid">
        <template v-for="(val, key) in paperReport" :key="key">
          <div v-if="key !== 'kind'" class="report-item">
            <small>{{ formatKey(String(key)) }}</small>
            <ElTag v-if="typeof val === 'boolean'" v-bind="statusTag(val)" size="small">
              {{ statusTag(val).text }}
            </ElTag>
            <strong v-else>{{ String(val ?? '—') }}</strong>
          </div>
        </template>
      </div>
    </ElCard>

    <!-- 通用 JSON 报告（兜底） -->
    <pre v-if="report && !paperReport" class="raw-report">{{ JSON.stringify(report, null, 2) }}</pre>
  </section>
</template>

<style scoped>
.card-title {
  font-size: 12px;
  font-weight: 700;
  color: #3c5a4a;
  letter-spacing: .03em;
}
.empty-state {
  padding: 24px;
  text-align: center;
  color: #9ca79f;
  font-size: 11px;
}
.opportunity-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
  gap: 10px;
}
.opportunity-card {
  padding: 14px;
  border: 1px solid #d7ddd6;
  border-radius: 6px;
  background: #fbfcfa;
  transition: border-color .2s;
}
.opportunity-card.accepted {
  border-color: #4a9272;
  background: #f3faf6;
}
.opp-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.opp-direction {
  font-size: 11px;
  color: #59675e;
}
.opp-direction b {
  color: #16825e;
}
.opp-metrics {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
}
.opp-metric small {
  display: block;
  color: #9ca79f;
  font-size: 8px;
  letter-spacing: .05em;
  text-transform: uppercase;
  margin-bottom: 3px;
}
.opp-metric strong {
  font-size: 13px;
  color: #34423a;
}
.opp-reasons {
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px dotted #e2e7e2;
}
.opp-reasons small {
  color: #b89731;
  font-size: 9px;
}
.report-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 12px;
}
.report-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.report-item small {
  color: #9ca79f;
  font-size: 9px;
  letter-spacing: .04em;
  text-transform: uppercase;
}
.report-item strong {
  font-size: 12px;
  color: #34423a;
}
.raw-report {
  max-height: 280px;
  overflow: auto;
  margin: 10px 0 0;
  padding: 14px;
  border-radius: 5px;
  background: #25332c;
  color: #d7f0df;
  font-size: 11px;
}
</style>
