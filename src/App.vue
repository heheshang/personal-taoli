<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import type { Status, Config, Account, Observe, Continuous } from './types'
import { COMMANDS, invokeCommand } from './commands'
import type { AppConfig } from './commands'
import SidebarNav from './components/SidebarNav.vue'
import TopBar from './components/TopBar.vue'
import ConfigStrip from './components/ConfigStrip.vue'
import MetricGrid from './components/MetricGrid.vue'
import HistoryChart from './components/HistoryChart.vue'
import ActivityLog from './components/ActivityLog.vue'
import RidgePanel from './components/RidgePanel.vue'
import VenueHandoff from './components/VenueHandoff.vue'
import StrategyLattice from './components/StrategyLattice.vue'
import RelationshipGraph from './components/RelationshipGraph.vue'
import DetailPanel from './components/DetailPanel.vue'
import SettingsPage from './components/SettingsPage.vue'
import AppFooter from './components/AppFooter.vue'

const status = ref<Status | null>(null)
const config = ref<Config | null>(null)
const accounts = ref<Account[]>([])
const observation = ref<Observe | null>(null)
const report = ref<unknown>(null)
const continuous = ref<Continuous | null>(null)
const busy = ref('')
const configPath = ref('')
const archivePath = ref('')
const databaseUrl = ref('')
const activePage = ref('overview')

const canOperate = computed(() => busy.value === '')
const liveLabel = computed(() => status.value?.real_order_capability ? '实盘' : 'PAPER / 只读')
const feedState = computed(() => observation.value?.feeds.map(feed => feed.state).join(' · ') || '等待一次观测')
const eventCount = computed(() => observation.value ? '1 条决策' : '0 条事件')

function fmtTime(ms: number | null): string {
  return ms ? new Date(ms).toISOString().replace('T', ' ').slice(0, 19) + ' UTC' : '—'
}

const POLL_INTERVAL_MS = 5000
let pollTimer: ReturnType<typeof setInterval> | null = null

function stopPolling() {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

function startPolling() {
  if (pollTimer !== null) return
  pollTimer = setInterval(() => {
    invokeCommand<Continuous>(COMMANDS.continuousObservationStatus).then(data => {
      if (data !== undefined) {
        continuous.value = data
        // 连续观测流式展示：每轮轮询把会话最新扫描报告推入展示层
        // （机会卡 / 概览指标），复用与 observe_once 一致的 ScanReport 形状。
        if (data.running && data.last_report) {
          observation.value = {
            report: data.last_report,
            accounts: observation.value?.accounts ?? [],
            feeds: observation.value?.feeds ?? [],
            archived: true,
          }
        }
        if (!data.running) stopPolling()
      }
    })
  }, POLL_INTERVAL_MS)
}

/** 命令执行辅助：占用全局 busy（互斥所有操作）并统一解包响应。 */
async function call<T>(name: string, args: Record<string, unknown> = {}): Promise<T | undefined> {
  busy.value = name
  try {
    return await invokeCommand<T>(name, args)
  } finally {
    busy.value = ''
  }
}

async function loadConfig() {
  config.value = (await call<Config>(COMMANDS.loadConfigSummary, { configPath: configPath.value || null })) ?? null
  if (config.value && !archivePath.value) archivePath.value = config.value.archive_path
}

async function loadAccounts() {
  accounts.value = (await call<Account[]>(COMMANDS.accountStatus, { configPath: configPath.value || null })) ?? []
}

async function observe() {
  observation.value = (await call<Observe>(COMMANDS.observeOnce, { configPath: configPath.value || null, archivePath: archivePath.value || null })) ?? null
}

async function replay() {
  report.value = await call(COMMANDS.replayObservations, { path: archivePath.value })
}

async function startContinuous() {
  continuous.value = (await call<Continuous>(COMMANDS.startContinuousObservation, { configPath: configPath.value || null, archivePath: archivePath.value || null })) ?? null
  if (continuous.value?.running) startPolling()
}

async function stopContinuous() {
  continuous.value = (await call<Continuous>(COMMANDS.stopContinuousObservation)) ?? null
  if (!continuous.value?.running) stopPolling()
}

async function refreshContinuous() {
  continuous.value = (await call<Continuous>(COMMANDS.continuousObservationStatus)) ?? null
  if (continuous.value?.running) startPolling()
}

async function reconnectSmoke() {
  report.value = await call(COMMANDS.runReconnectSmoke, { configPath: configPath.value || null })
}

async function paper(kind: string) {
  report.value = await call(COMMANDS.runPaperSmoke, { kind, databaseUrlParam: databaseUrl.value || null })
}

async function accountingControl(kind: string) {
  report.value = await call(COMMANDS.runAccountingControlSmoke, { kind, databaseUrlParam: databaseUrl.value || null })
}

onMounted(async () => {
  status.value = (await call<Status>(COMMANDS.desktopStatus)) ?? null
  await loadConfig()
  const appConfig = await invokeCommand<AppConfig>(COMMANDS.loadAppConfig)
  if (appConfig?.database_url) databaseUrl.value = appConfig.database_url
  await refreshContinuous()
})

/** 配置保存后：数据库地址当前会话即刻生效（smoke 命令走 database_url_param 通道），其余重启生效。 */
async function onConfigSaved(updated: AppConfig | null) {
  if (updated?.database_url) databaseUrl.value = updated.database_url
  await loadConfig()
}

onUnmounted(stopPolling)
</script>

<template>
  <div class="app-layout">
    <SidebarNav
      :active="activePage"
      :continuous-running="continuous?.running ?? false"
      @navigate="activePage = $event"
    />

    <main class="main-content">
      <TopBar
        :version="status?.version ?? null"
        :live-label="liveLabel"
        :can-operate="canOperate"
        @refresh="loadConfig"
      />

      <ConfigStrip
        :symbol="config?.symbol ?? null"
        :quantity="config?.quantity ?? null"
        :base-asset="config?.base_asset ?? null"
        :orderbook-depth="config?.orderbook_depth ?? null"
      />

      <!-- 概览页 -->
      <div v-show="activePage === 'overview'" class="page-content">
        <MetricGrid
          :observation="observation"
          :accounts="accounts"
          :feed-state="feedState"
        />
        <section class="two-col">
          <HistoryChart />
          <ActivityLog :feed-state="feedState" :event-count="eventCount" />
        </section>
        <RidgePanel :observation="observation" />
      </div>

      <!-- 市场页 -->
      <div v-show="activePage === 'market'" class="page-content">
        <section class="two-col">
          <VenueHandoff :accounts="accounts" />
          <StrategyLattice :orderbook-depth="config?.orderbook_depth ?? null" />
        </section>
        <RelationshipGraph />
        <MetricGrid
          :observation="observation"
          :accounts="accounts"
          :feed-state="feedState"
        />
      </div>

      <!-- 控制台页（功能板块 + 对应操作） -->
      <div v-show="activePage === 'control'" class="page-content">
        <DetailPanel
          :accounts="accounts"
          :observation="observation"
          :report="report"
          :can-operate="canOperate"
          :archive-path="archivePath"
          :continuous="continuous"
          :fmt-time="fmtTime"
          @load-accounts="loadAccounts"
          @observe="observe"
          @replay="replay"
          @paper="paper"
          @accounting-control="accountingControl"
          @start-continuous="startContinuous"
          @stop-continuous="stopContinuous"
          @reconnect-smoke="reconnectSmoke"
        />
      </div>

      <!-- 设置页 -->
      <div v-show="activePage === 'settings'" class="page-content">
        <SettingsPage
          :can-operate="canOperate"
          @config-saved="onConfigSaved"
        />
      </div>

      <AppFooter
        :symbol="config?.symbol ?? null"
        :version="status?.version ?? null"
      />
    </main>
  </div>
</template>
