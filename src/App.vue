<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ElMessage } from 'element-plus'
import { invoke } from '@tauri-apps/api/core'
import type { ApiResponse, Status, Config, Account, Observe, Continuous } from './types'
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
import ControlPanel from './components/ControlPanel.vue'
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
  } finally {
    busy.value = ''
  }
}

async function loadConfig() {
  config.value = (await call<Config>('load_config_summary', { configPath: configPath.value || null })) ?? null
  if (config.value && !archivePath.value) archivePath.value = config.value.archive_path
}

async function loadAccounts() {
  accounts.value = (await call<Account[]>('account_status', { configPath: configPath.value || null })) ?? []
}

async function observe() {
  observation.value = (await call<Observe>('observe_once', { configPath: configPath.value || null, archivePath: archivePath.value || null })) ?? null
}

async function replay() {
  report.value = await call('replay_observations', { path: archivePath.value })
}

async function startContinuous() {
  continuous.value = (await call<Continuous>('start_continuous_observation', { configPath: configPath.value || null, archivePath: archivePath.value || null })) ?? null
}

async function stopContinuous() {
  continuous.value = (await call<Continuous>('stop_continuous_observation')) ?? null
}

async function refreshContinuous() {
  continuous.value = (await call<Continuous>('continuous_observation_status')) ?? null
}

async function reconnectSmoke() {
  report.value = await call('run_reconnect_smoke', { configPath: configPath.value || null })
}

async function paper(kind: string) {
  report.value = await call('run_paper_smoke', { kind, databaseUrlParam: databaseUrl.value || null })
}

async function accountingControl(kind: string) {
  report.value = await call('run_accounting_control_smoke', { kind, databaseUrlParam: databaseUrl.value || null })
}

onMounted(async () => {
  status.value = (await call<Status>('desktop_status')) ?? null
  await loadConfig()
  await refreshContinuous()
})
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

      <!-- 控制台页 -->
      <div v-show="activePage === 'control'" class="page-content">
        <ControlPanel
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

      <!-- 报告页 -->
      <div v-show="activePage === 'reports'" class="page-content">
        <DetailPanel
          :accounts="accounts"
          :observation="observation"
          :report="report"
        />
      </div>

      <!-- 设置页 -->
      <div v-show="activePage === 'settings'" class="page-content">
        <SettingsPage
          :can-operate="canOperate"
          @config-saved="loadConfig"
        />
      </div>

      <AppFooter
        :symbol="config?.symbol ?? null"
        :version="status?.version ?? null"
      />
    </main>
  </div>
</template>
