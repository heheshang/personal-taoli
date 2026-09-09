<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ElMessage } from 'element-plus'
import { invoke } from '@tauri-apps/api/core'

type ApiError = { code: string; message: string; retryable: boolean }
type ApiResponse<T> = { success: boolean; data: T | null; error: ApiError | null }
type Status = { mode: string; version: string; real_order_capability: boolean }
type Config = { symbol: string; base_asset: string; quote_asset: string; quantity: string; orderbook_depth: number; archive_path: string; binance_websocket_url: string; bybit_websocket_url: string }
type Account = { venue: string; capability: Record<string, unknown>; permission: Record<string, unknown>; fee: Record<string, unknown>; rejection_reasons: string[] }
type Observe = { report: Record<string, unknown>; accounts: Account[]; feeds: Record<string, unknown>[]; archived: boolean }
type Continuous = { running: boolean; archive_path: string | null; gap_path: string | null; started_at_ms: number | null; last_report_at_ms: number | null; last_report: Record<string, unknown> | null; error: string | null }

const status = ref<Status | null>(null)
const config = ref<Config | null>(null)
const accounts = ref<Account[]>([])
const observation = ref<Observe | null>(null)
const report = ref<unknown>(null)
const continuous = ref<Continuous | null>(null)
const busy = ref('')
const configPath = ref('')
const archivePath = ref('')
const canOperate = computed(() => busy.value === '')
const liveLabel = computed(() => status.value?.real_order_capability ? 'LIVE' : 'PAPER / READ-ONLY')
const feedState = computed(() => observation.value?.feeds.map(feed => String(feed.state)).join(' · ') || '等待一次观测')
const eventCount = computed(() => observation.value ? '1 decision' : '0 events')
function fmtTime(ms: number | null): string {
  return ms ? new Date(ms).toISOString().replace('T', ' ').slice(0, 19) + ' UTC' : '—'
}
async function call<T>(name: string, args: Record<string, unknown> = {}): Promise<T | undefined> {
  busy.value = name
  try {
    const response = await invoke<ApiResponse<T>>(name, args)
    if (!response.success || response.data === null) {
      const error = response.error
      ElMessage.error(`${error?.code ?? 'INTERNAL_ERROR'}: ${error?.message ?? 'command failed'}`)
      return undefined
    }
    return response.data
  } catch (error) {
    ElMessage.error(`IPC_ERROR: ${String(error)}`)
    return undefined
  } finally { busy.value = '' }
}
async function loadConfig() {
  config.value = (await call<Config>('load_config_summary', { configPath: configPath.value || null })) ?? null
  if (config.value && !archivePath.value) archivePath.value = config.value.archive_path
}
async function loadAccounts() { accounts.value = (await call<Account[]>('account_status', { configPath: configPath.value || null })) ?? [] }
async function observe() { observation.value = (await call<Observe>('observe_once', { configPath: configPath.value || null, archivePath: archivePath.value || null })) ?? null }
async function replay() { report.value = await call('replay_observations', { path: archivePath.value }) }
async function startContinuous() { continuous.value = (await call<Continuous>('start_continuous_observation', { configPath: configPath.value || null, archivePath: archivePath.value || null })) ?? null }
async function stopContinuous() { continuous.value = (await call<Continuous>('stop_continuous_observation')) ?? null }
async function refreshContinuous() { continuous.value = (await call<Continuous>('continuous_observation_status')) ?? null }
async function reconnectSmoke() { report.value = await call('run_reconnect_smoke', { configPath: configPath.value || null }) }
async function paper(kind: string) { report.value = await call('run_paper_smoke', { kind }) }
async function accountingControl(kind: string) { report.value = await call('run_accounting_control_smoke', { kind }) }
onMounted(async () => { status.value = (await call<Status>('desktop_status')) ?? null; await loadConfig(); await refreshContinuous() })
</script>

<template>
  <main class="dashboard">
    <header class="topbar">
      <div class="brand"><span class="brand-mark">↗</span><div><strong>TAOLI OBSERVER</strong><small>PERSONAL ARBITRAGE DESK</small></div></div>
      <div class="top-actions"><span class="clock">{{ status?.version ? `v${status.version}` : 'booting' }}</span><span class="live-pill"><i />{{ liveLabel }}</span><el-button text class="refresh" :disabled="!canOperate" @click="loadConfig">↻</el-button></div>
    </header>

    <section class="config-strip">
      <span class="eyebrow">MARKET / CONFIG</span><b>{{ config?.symbol || '—' }}</b><span>{{ config?.quantity || '—' }} {{ config?.base_asset || '' }}</span><span class="muted">DEPTH {{ config?.orderbook_depth || '—' }}</span>
      <el-input v-model="configPath" size="small" placeholder="config/observer.toml" class="path-input" />
    </section>

    <section class="metric-grid">
      <article class="metric-card"><span class="eyebrow">OBSERVATION MODE</span><strong>READ ONLY</strong><small>NO REAL ORDERS ENABLED</small></article>
      <article class="metric-card accent"><span class="eyebrow">EXPECTED NET</span><strong>{{ observation ? 'SEE REPORT' : '—' }}</strong><small>AFTER FEES / RISK BUFFER</small></article>
      <article class="metric-card"><span class="eyebrow">FEED STATUS</span><strong>{{ observation ? feedState : 'SYNCING' }}</strong><small>BINANCE · BYBIT</small></article>
      <article class="metric-card gate"><span class="eyebrow">ADMISSION GATE</span><strong>{{ accounts.length ? 'REVIEWED' : 'PENDING' }}</strong><div class="gate-bars"><i v-for="n in 8" :key="n" :class="{ on: accounts.length > 0 && n < 6 }" /></div><small>{{ accounts.length ? 'CHECK REJECTION REASONS' : 'LOAD ACCOUNT STATUS' }}</small></article>
    </section>

    <section class="two-col">
      <article class="panel history-panel"><div class="panel-head"><h2><em class="green-dot" /> BALANCE / OPPORTUNITY HISTORY</h2><span>UTC / SESSION</span></div><div class="chart-wrap"><svg viewBox="0 0 640 210" preserveAspectRatio="none" aria-label="history chart"><defs><linearGradient id="fill" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#9adbc4" stop-opacity=".42" /><stop offset="1" stop-color="#9adbc4" stop-opacity="0" /></linearGradient></defs><path d="M0 162 C70 160 95 166 155 151 S245 158 296 144 S354 149 400 142 S455 150 492 136 S526 141 552 75 S590 106 640 43 L640 210 L0 210Z" fill="url(#fill)" /><path d="M0 162 C70 160 95 166 155 151 S245 158 296 144 S354 149 400 142 S455 150 492 136 S526 141 552 75 S590 106 640 43" fill="none" stroke="#28352f" stroke-width="3" /><circle cx="552" cy="75" r="6" fill="#dc5974" /><line x1="552" y1="75" x2="552" y2="210" stroke="#dc5974" stroke-dasharray="3 5" /></svg><div class="axis"><span>START</span><span>OBSERVATION WINDOW</span><span>NOW</span></div></div></article>
      <article class="panel activity"><div class="panel-head"><h2><em class="pink-dot" /> ACTIVITY LOG</h2><span>{{ eventCount }}</span></div><div class="log-list"><div><time>NOW</time><b class="yellow">SYSTEM</b><span>dashboard ready · safe mode enforced</span></div><div><time>—</time><b class="blue">FEED</b><span>{{ feedState }}</span></div><div><time>—</time><b class="pink">PAPER</b><span>external order calls: 0</span></div><div><time>—</time><b class="green">AUDIT</b><span>commands remain read-only</span></div><div><time>—</time><b class="gray">NOTE</b><span>run an action below to populate facts</span></div></div></article>
    </section>

    <section class="panel ridge"><div class="panel-head"><h2><em class="orange-dot" /> ARBITRAGE OPPORTUNITY RIDGE</h2><span>GROSS / FEES / NET</span></div><div class="ridge-body"><div class="ridge-stats"><span>VALID FEEDS <b>{{ observation ? '2 / 2' : '0 / 2' }}</b></span><span>SCANNED <b>{{ observation ? '2' : '0' }}</b></span><span>ACCEPTED <b>{{ observation?.report ? 'SEE REPORT' : '—' }}</b></span></div><div class="ridge-visual"><div v-for="n in 9" :key="n" class="ridge-line" :style="{ transform: `translateY(${n * 7}px) rotate(${n < 5 ? -5 : 5}deg)`, opacity: `${1 - n * .06}` }" /><div class="ridge-label">NET AFTER FEES<br /><b>{{ observation ? 'CALCULATED' : 'AWAITING FEED' }}</b></div></div></div></section>

    <section class="two-col lower-grid">
      <article class="panel handoff"><div class="panel-head"><h2><em class="teal-dot" /> VENUE HANDOFF</h2><span>FEE / PERMISSION</span></div><div class="handoff-body"><div class="radar"><span class="radar-ring r1" /><span class="radar-ring r2" /><span class="radar-ring r3" /><i class="radar-line" /><b>2 VENUES</b></div><div class="venue-list"><div v-for="venue in ['BINANCE', 'BYBIT']" :key="venue"><b>{{ venue }}</b><span>{{ accounts.length ? 'REVIEWED' : 'NOT LOADED' }}</span><strong>{{ accounts.find(a => a.venue.toUpperCase() === venue)?.fee?.source || '—' }}</strong></div></div></div></article>
      <article class="panel lattice"><div class="panel-head"><h2><em class="blue-dot" /> 5D STRATEGY LATTICE</h2><span>RISK / COST / LATENCY</span></div><div class="lattice-body"><div class="cube"><i /><i /><i /><i /><i /><i /></div><div class="lattice-values"><span>DEPTH <b>{{ config?.orderbook_depth || '—' }}</b></span><span>RISK BUFFER <b>CONFIGURED</b></span><span>EXECUTION <b>PAPER ONLY</b></span></div></div></article>
    </section>

    <section class="panel graph"><div class="panel-head"><h2><em class="purple-dot" /> RELATIONSHIP GRAPH / SIMULATION</h2><span>VENUES · FLOW / LIABILITY</span></div><div class="graph-body"><div class="node left">BINANCE<small>PUBLIC FEED</small></div><div class="dashed-flow">········································▶</div><div class="node center">TAOLI CORE<small>DECISION / PAPER</small></div><div class="node right">BYBIT<small>PUBLIC FEED</small></div></div></section>
    <section class="control-panel"><div class="control-title"><span class="eyebrow">CONTROL DECK</span><b>READ-ONLY ACTIONS</b></div><div class="control-actions"><el-button :disabled="!canOperate" @click="loadAccounts">ACCOUNT STATUS</el-button><el-button type="primary" :disabled="!canOperate" @click="observe">OBSERVE ONCE</el-button><el-button :disabled="!canOperate || !archivePath" @click="replay">REPLAY</el-button><el-button v-for="kind in ['B01','B02','B03']" :key="kind" :disabled="!canOperate" @click="paper(kind)">PAPER {{ kind }}</el-button><el-button v-for="kind in ['ACCOUNTING','RECONCILIATION','CONTROL']" :key="kind" :disabled="!canOperate" @click="accountingControl(kind)">ACCOUNTING {{ kind }}</el-button><el-button :disabled="!canOperate || continuous?.running" @click="startContinuous">CONTINUOUS START</el-button><el-button :disabled="!canOperate || !continuous?.running" @click="stopContinuous">CONTINUOUS STOP</el-button><el-button :disabled="!canOperate" @click="reconnectSmoke">RECONNECT SMOKE</el-button></div><div v-if="continuous" class="control-session"><template v-if="continuous.running"><i class="pulse" /><span>CONTINUOUS <b>RUNNING</b> · {{ continuous.archive_path || 'archive from config' }} · since {{ fmtTime(continuous.started_at_ms) }}</span></template><template v-else><span>CONTINUOUS STOPPED{{ continuous.error ? ' — ' + continuous.error : '' }}</span></template></div><small class="control-note">REPLAY requires an existing archive; PAPER and accounting controls are database-backed smoke operations.</small></section>

    <section v-if="accounts.length || observation || report" class="detail-panel"><el-card v-if="accounts.length" shadow="never"><template #header>ACCOUNT FACTS</template><el-table :data="accounts" stripe><el-table-column prop="venue" label="VENUE" width="120" /><el-table-column label="FEE SOURCE"><template #default="{ row }">{{ row.fee.source }} / {{ row.fee.buy_taker_rate }} / {{ row.fee.sell_taker_rate }}</template></el-table-column><el-table-column label="REJECTIONS"><template #default="{ row }">{{ row.rejection_reasons.join(' · ') || 'none' }}</template></el-table-column></el-table></el-card><pre v-if="observation">{{ JSON.stringify(observation.report, null, 2) }}</pre><pre v-if="report">{{ JSON.stringify(report, null, 2) }}</pre></section>
    <footer><span>TAOLI OBSERVER · {{ config?.symbol || 'MARKET' }}</span><span>SAFE MODE / NO ORDER ROUTES / {{ status?.version || '—' }}</span></footer>
  </main>
</template>
