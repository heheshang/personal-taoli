<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElButton, ElDrawer, ElPagination, ElSelect, ElTable, ElTableColumn, ElTag, ElOption } from 'element-plus'
import type { SimulationOverview, SimulationRunDetail, SimulationRunRow } from '../types'
import { getSimulationRunDetail, getSimulationRuns } from '../commands'
import { trimDecimal } from '../format'

const props = defineProps<{
  databaseUrl: string
  overview: SimulationOverview | null
}>()

const emit = defineEmits<{
  openRun: [runId: string]
}>()

const runs = ref<SimulationRunRow[]>([])
const total = ref(0)
const page = ref(1)
/** 每页 20 行（用户要求的分页规格）。 */
const PAGE_SIZE = 20
const symbolFilter = ref<string | null>(null)
const scenarioFilter = ref<string | null>(null)
const loading = ref(false)

const detail = ref<SimulationRunDetail | null>(null)
const detailLoading = ref(false)
const drawerOpen = ref(false)

const SCENARIO_LABELS: Record<string, string> = {
  NORMAL: '正常成交',
  DEPTH_SHORTFALL: '深度不足',
  COMPETED_AWAY: '被抢单',
  REJECTED: '拒绝',
}

const STATE_LABELS: Record<string, string> = {
  PLANNED: '已计划',
  RUNNING: '执行中',
  COMPENSATION_PLANNED: '补偿计划',
  COMPLETED: '已完成',
  MANUAL_REQUIRED: '需人工',
}

function scenarioTag(s: string): 'success' | 'warning' | 'danger' | 'info' {
  if (s === 'NORMAL') return 'success'
  if (s === 'REJECTED') return 'danger'
  if (s === 'DEPTH_SHORTFALL') return 'warning'
  return 'info'
}

function netClass(v: string): string {
  const n = parseFloat(v)
  if (isNaN(n) || n === 0) return 'net-zero'
  return n > 0 ? 'net-pos' : 'net-neg'
}

async function loadRuns() {
  loading.value = true
  try {
    const res = await getSimulationRuns({
      databaseUrl: props.databaseUrl,
      limit: PAGE_SIZE,
      offset: (page.value - 1) * PAGE_SIZE,
      symbol: symbolFilter.value,
      scenario: scenarioFilter.value,
    })
    runs.value = res?.runs ?? []
    total.value = res?.total ?? 0
  } finally {
    loading.value = false
  }
}

async function openRun(row: SimulationRunRow) {
  emit('openRun', row.run_id)
  drawerOpen.value = true
  detailLoading.value = true
  detail.value = null
  try {
    detail.value = (await getSimulationRunDetail(row.run_id, props.databaseUrl)) ?? null
  } finally {
    detailLoading.value = false
  }
}

/** 供 Activity 日志跳转：按 run_id 在当前分页/最近列表中定位行并打开详情抽屉。 */
async function openByRunId(runId: string) {
  const row = runs.value.find(r => r.run_id === runId) ?? props.overview?.recent_runs.find(r => r.run_id === runId)
  if (row) {
    await openRun(row)
    return
  }
  // 不在当前列表：临时构造行触发详情拉取。
  drawerOpen.value = true
  detailLoading.value = true
  detail.value = null
  try {
    detail.value = (await getSimulationRunDetail(runId, props.databaseUrl)) ?? null
  } finally {
    detailLoading.value = false
  }
}

defineExpose({ openByRunId })

function fmtTime(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function fmtClock(ms: number): string {
  const d = new Date(ms)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
}

function dirLabel(direction: string): string {
  return direction === 'a' ? '买腿 → 卖腿' : direction === 'b' ? '卖腿 → 买腿' : direction
}

function keys(o: Record<string, unknown> | null | undefined): string[] {
  return o ? Object.keys(o).filter(k => k !== 'planning_warnings') : []
}

function fmtField(key: string): string {
  const map: Record<string, string> = {
    symbol: '合约',
    quantity: '数量',
    buy_venue: '买所',
    sell_venue: '卖所',
    direction: '方向',
    scenario: '场景',
    rejection_reason: '拒绝原因',
    decision_to_submit_ms: '决策→提交(ms)',
    fill_latency_ms: '成交时延(ms)',
    seed: '随机种子',
    buy_avg_price: '买均价',
    sell_avg_price: '卖均价',
    buy_fee: '买腿费用',
    sell_fee: '卖腿费用',
    bought_quantity: '买入量',
    sold_quantity: '卖出量',
    scanned_net_profit: '扫描净盈亏',
    simulated_net_profit: '模拟净盈亏',
    adverse_move_bps: '不利波动(bps)',
    competitor_take_bps: '被抢(bps)',
    compensation_decision: '补偿决策',
    execution_state: '执行状态',
    unmatched_quantity: '未匹配量',
    unmatched_exposure: '未匹配敞口',
    estimated_compensation_cost: '预估补偿成本',
    unknown_submit_tried: 'UNKNOWN提交尝试',
    query_found: '查询找到',
    idempotent_replay: '幂等重放',
    skipped_frames: '跳过帧数',
    external_order_calls: '外部订单调用',
    planning_warnings: '计划警告',
  }
  return map[key] ?? key
}

function fmtVal(v: unknown): string {
  if (v === null || v === undefined) return '—'
  if (typeof v === 'boolean') return v ? '是' : '否'
  if (typeof v === 'number') return String(v)
  if (typeof v === 'object') {
    try {
      return JSON.stringify(v)
    } catch {
      return String(v)
    }
  }
  return trimDecimal(String(v))
}

onMounted(loadRuns)
</script>

<template>
  <section class="panel sim-trades">
    <div class="panel-head">
      <h2><em class="green-dot" /> 机会历史 · 交易明细与获利</h2>
      <div class="filters">
        <ElSelect v-model="symbolFilter" placeholder="合约" clearable size="small" style="width: 110px" @change="page = 1; loadRuns()">
          <ElOption label="BTC/USDT" value="BTC/USDT" />
          <ElOption label="ETH/USDT" value="ETH/USDT" />
        </ElSelect>
        <ElSelect v-model="scenarioFilter" placeholder="场景" clearable size="small" style="width: 130px" @change="page = 1; loadRuns()">
          <ElOption v-for="(lbl, key) in SCENARIO_LABELS" :key="key" :label="lbl" :value="key" />
        </ElSelect>
        <ElButton size="small" @click="page = 1; loadRuns()">刷新</ElButton>
      </div>
    </div>

    <div v-if="!total && !runs.length" class="empty-state">暂无 run 数据（先运行烟测）</div>
    <template v-else>
      <ElTable v-loading="loading" :data="runs" stripe size="small" class="runs-table" @row-click="openRun">
        <ElTableColumn label="RUN / 时间" width="168">
          <template #default="{ row }">
            <div class="cell-run">{{ row.run_id }}</div>
            <div class="cell-sub">{{ fmtTime(row.executed_at_ms) }}</div>
          </template>
        </ElTableColumn>
        <ElTableColumn label="合约" width="92">
          <template #default="{ row }">{{ row.symbol }}</template>
        </ElTableColumn>
        <ElTableColumn label="方向/场景" width="128">
          <template #default="{ row }">
            <ElTag size="small" :type="scenarioTag(row.scenario)" effect="plain">{{ SCENARIO_LABELS[row.scenario] ?? row.scenario }}</ElTag>
            <div class="cell-sub">{{ dirLabel(row.direction) }}</div>
          </template>
        </ElTableColumn>
        <ElTableColumn label="数量" width="92" align="right">
          <template #default="{ row }">
            <div class="mono">{{ trimDecimal(row.quantity) }}</div>
            <div class="cell-sub">买{{ trimDecimal(row.bought_quantity) }} 卖{{ trimDecimal(row.sold_quantity) }}</div>
          </template>
        </ElTableColumn>
        <ElTableColumn label="买/卖均价" width="128" align="right">
          <template #default="{ row }">
            <div class="mono">{{ trimDecimal(row.buy_avg_price) }} / {{ trimDecimal(row.sell_avg_price) }}</div>
            <div class="cell-sub">费 {{ trimDecimal(row.buy_fee) }}+{{ trimDecimal(row.sell_fee) }}</div>
          </template>
        </ElTableColumn>
        <ElTableColumn label="扫描净盈亏" width="104" align="right">
          <template #default="{ row }">
            <b :class="netClass(row.scanned_net_profit)">{{ trimDecimal(row.scanned_net_profit) }}</b>
          </template>
        </ElTableColumn>
        <ElTableColumn label="模拟净盈亏" width="104" align="right">
          <template #default="{ row }">
            <b :class="netClass(row.simulated_net_profit)">{{ trimDecimal(row.simulated_net_profit) }}</b>
          </template>
        </ElTableColumn>
        <ElTableColumn label="补偿/状态" width="140">
          <template #default="{ row }">
            <ElTag size="small" effect="plain" type="info">{{ STATE_LABELS[row.execution_state] ?? row.execution_state }}</ElTag>
            <div class="cell-sub">{{ row.compensation_decision }}·外呼{{ row.external_order_calls }}·重放{{ row.idempotent_replay ? '是' : '否' }}</div>
          </template>
        </ElTableColumn>
        <ElTableColumn label="原因" min-width="120">
          <template #default="{ row }">
            <span v-if="row.rejection_reason" class="reject">{{ row.rejection_reason }}</span>
            <span v-else class="cell-sub">—</span>
          </template>
        </ElTableColumn>
      </ElTable>
      <div class="pager">
        <ElPagination
          v-model:current-page="page"
          layout="prev, pager, next, total"
          :page-size="PAGE_SIZE"
          :total="total"
          background
          small
          @current-change="loadRuns"
        />
      </div>
    </template>

    <!-- 单 run 详情：报告全字段 + 意图/成交/执行事件/审计 -->
    <ElDrawer v-model="drawerOpen" size="62%" :with-header="false" class="sim-drawer">
      <div v-loading="detailLoading" class="drawer-body">
        <template v-if="detail">
          <div class="drawer-head">
            <div>
              <h3 class="mono">{{ detail.run.run_id }}</h3>
              <div class="cell-sub">{{ fmtTime(detail.run.executed_at_ms) }} · {{ detail.run.symbol }} · {{ SCENARIO_LABELS[detail.run.scenario] ?? detail.run.scenario }} · {{ dirLabel(detail.run.direction) }}</div>
            </div>
            <div class="drawer-net">
              <span class="cell-sub">扫描净盈亏</span>
              <b :class="netClass(detail.run.scanned_net_profit)">{{ trimDecimal(detail.run.scanned_net_profit) }}</b>
              <span class="cell-sub">模拟净盈亏</span>
              <b :class="netClass(detail.run.simulated_net_profit)">{{ trimDecimal(detail.run.simulated_net_profit) }}</b>
            </div>
          </div>

          <div class="detail-grid">
            <div class="detail-card">
              <h4>报告全字段</h4>
              <div v-for="k in keys(detail.report)" :key="k" class="kv">
                <span>{{ fmtField(k) }}</span>
                <b class="mono">{{ fmtVal(detail.report[k]) }}</b>
              </div>
              <div v-if="detail.report.planning_warnings" class="kv kv-warn">
                <span>计划警告</span>
                <b class="mono warn">{{ Array.isArray(detail.report.planning_warnings) && detail.report.planning_warnings.length ? (detail.report.planning_warnings as string[]).join('；') : '—' }}</b>
              </div>
            </div>

            <div class="detail-card">
              <h4>双腿意图（{{ detail.intents.length }}）</h4>
              <div v-for="it in detail.intents" :key="it.intent_id" class="intent">
                <div class="intent-head">
                  <b class="mono">{{ it.leg_id }}</b>
                  <ElTag size="small" effect="plain" type="info">{{ it.venue }} · {{ it.side }}</ElTag>
                  <span class="cell-sub">{{ it.submission_status }} / {{ it.cancel_status }} / {{ it.reconciliation_status }}</span>
                </div>
                <div class="kv"><span>量 / 限价</span><b class="mono">{{ trimDecimal(it.quantity) }} @ {{ trimDecimal(it.limit_price) }}</b></div>
                <div class="kv"><span>已成交量</span><b class="mono">{{ trimDecimal(it.filled_quantity) }}</b></div>
                <div v-if="it.exchange_order_id" class="kv"><span>交易所单号</span><b class="mono">{{ it.exchange_order_id }}</b></div>
                <div v-for="a in it.actions" :key="a.fact_id" class="kv kv-action">
                  <span>动作 {{ a.action }}</span>
                  <b class="mono">{{ fmtClock(a.occurred_at_ms) }} {{ fmtVal(a.payload) }}</b>
                </div>
              </div>
              <div v-if="!detail.intents.length" class="cell-sub">无意图（REJECTED 分支）</div>
            </div>
          </div>

          <div class="detail-grid">
            <div class="detail-card">
              <h4>成交事由（{{ detail.trades.length }}）</h4>
              <div v-for="t in detail.trades" :key="t.trade_id" class="kv">
                <span>{{ t.venue }} · {{ trimDecimal(t.quantity) }} @ {{ trimDecimal(t.price) }}</span>
                <b class="mono">费 {{ trimDecimal(t.fee_amount) }} {{ t.fee_asset }} · {{ fmtClock(t.occurred_at_ms) }}</b>
              </div>
              <div v-if="!detail.trades.length" class="cell-sub">无成交（REJECTED / DEPTH_SHORTFALL 分支）</div>
            </div>

            <div class="detail-card">
              <h4>执行事件（{{ detail.execution_events.length }}）</h4>
              <div v-for="ev in detail.execution_events" :key="ev.event_id" class="kv">
                <span>v{{ ev.event_version }} · {{ ev.event_type }}</span>
                <b class="mono">{{ fmtClock(ev.occurred_at_ms) }} {{ fmtVal(ev.payload) }}</b>
              </div>
              <div v-if="!detail.execution_events.length" class="cell-sub">无执行事件</div>
            </div>
          </div>

          <div class="detail-card">
            <h4>账户余额快照（{{ detail.balances.length }} 点 · fund/成交后/评估后）</h4>
            <div class="bal-table">
              <div v-for="b in detail.balances" :key="b.account_id + b.node + b.asset" class="kv">
                <span>{{ b.venue }} · {{ b.asset }} · {{ b.node }}</span>
                <b class="mono">总 {{ trimDecimal(b.total) }} / 可用 {{ trimDecimal(b.free) }}</b>
              </div>
              <div v-if="!detail.balances.length" class="cell-sub">无快照</div>
            </div>
          </div>

          <div class="detail-card">
            <h4>审计事件（{{ detail.audit.length }} · 不可变表）</h4>
            <div v-for="a in detail.audit" :key="a.event_id" class="kv">
              <span>{{ fmtClock(a.occurred_at_ms) }} · {{ a.event_type }} · {{ a.aggregate_id }}</span>
              <b class="mono">{{ fmtVal(a.payload) }}</b>
            </div>
            <div v-if="!detail.audit.length" class="cell-sub">无审计事件</div>
          </div>
        </template>
        <div v-else-if="!detailLoading" class="empty-state">请求详情失败</div>
      </div>
    </ElDrawer>
  </section>
</template>

<style scoped>
.filters {
  display: flex;
  gap: 6px;
  align-items: center;
}
.runs-table {
  width: 100%;
}
.runs-table :deep(.el-table__row) {
  cursor: pointer;
}
.cell-run {
  font-family: var(--font-mono);
  font-size: var(--fs-10);
  color: var(--text-2);
}
.cell-sub {
  color: var(--text-3);
  font-size: var(--fs-10);
  margin-top: 2px;
}
.mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.net-pos {
  color: var(--accent);
}
.net-neg {
  color: var(--danger);
}
.net-zero {
  color: var(--text-3);
}
.reject {
  color: var(--danger);
  font-size: var(--fs-11);
}
.pager {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
  padding: 10px 16px;
}
.pager-info {
  color: var(--text-3);
  font-size: var(--fs-11);
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
  border-top: 1px solid var(--border);
}
.drawer-body {
  padding: 18px 22px;
}
.drawer-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  margin-bottom: 16px;
}
.drawer-head h3 {
  margin: 0 0 4px;
  font-size: 16px;
}
.drawer-net {
  display: grid;
  grid-template-columns: auto auto;
  gap: 2px 14px;
  align-items: baseline;
}
.drawer-net b {
  font-family: var(--font-mono);
  font-size: 15px;
}
.detail-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  margin-bottom: 12px;
}
.detail-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 10px 12px;
}
.detail-card h4 {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--text-2);
  border-bottom: 1px solid var(--border);
  padding-bottom: 6px;
}
.kv {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 3px 0;
  font-size: 11px;
}
.kv > span {
  color: var(--text-3);
  flex-shrink: 0;
}
.kv > b {
  text-align: right;
  color: var(--text-1);
  word-break: break-all;
}
.intent {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 6px 8px;
  margin-bottom: 6px;
}
.intent-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}
.kv-action {
  border-top: 1px dashed var(--border);
  margin-top: 4px;
}
.warn {
  color: var(--warning);
}
.bal-table {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 4px 16px;
}
@media (max-width: 1100px) {
  .detail-grid {
    grid-template-columns: 1fr;
  }
  .bal-table {
    grid-template-columns: 1fr;
  }
}
</style>