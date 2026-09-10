<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { ElButton, ElTag } from 'element-plus'
import type { SimulationOverview } from '../types'
import { getObserverConfig, getSimulationOverview, runSimulationSmoke } from '../commands'
import type { ObserverConfig, SimulationSmokeResult } from '../commands'
import { trimDecimal } from '../format'
import SimulationStatCards from './SimulationStatCards.vue'
import SimulationProfitChart from './SimulationProfitChart.vue'
import SimulationFlow from './SimulationFlow.vue'
import SimulationBalanceChart from './SimulationBalanceChart.vue'
import SimulationRidge from './SimulationRidge.vue'
import SimulationAccountBalances from './SimulationAccountBalances.vue'
import SimulationActivity from './SimulationActivity.vue'
import SimulationTrades from './SimulationTrades.vue'

const props = defineProps<{
  active: boolean
  databaseUrl: string
}>()

const tradesRef = ref<InstanceType<typeof SimulationTrades> | null>(null)

/** Activity 日志点击 run 归属 → 转发给机会历史组件打开详情抽屉。 */
function openRunFromActivity(runId: string) {
  tradesRef.value?.openByRunId(runId)
}

const overview = ref<SimulationOverview | null>(null)
const config = ref<ObserverConfig | null>(null)
const smoke = ref<SimulationSmokeResult | null>(null)
const smokeRunning = ref(false)
const loading = ref(false)

const SIM_POLL_MS = 10000
let pollTimer: ReturnType<typeof setInterval> | null = null

/** 配置摘要：模拟引擎参数（注入额 / 时延 / 滑点 / 预算），只在页面激活时与 overview 一同刷新。 */
const simConfig = computed(() => config.value?.simulation ?? null)

async function refreshOverview() {
  if (!props.active) return
  loading.value = true
  try {
    overview.value = (await getSimulationOverview(props.databaseUrl)) ?? null
  } finally {
    loading.value = false
  }
}

function startPolling() {
  if (pollTimer !== null) return
  pollTimer = setInterval(refreshOverview, SIM_POLL_MS)
}

function stopPolling() {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

watch(
  () => props.active,
  active => {
    if (active) {
      refreshOverview()
      startPolling()
    } else {
      stopPolling()
    }
  },
)

/** 运行一次 F-02 烟测并刷新全部视图（外部订单调用恒 0 由报告断言保证）。 */
async function runOneSmoke() {
  smokeRunning.value = true
  try {
    smoke.value = (await runSimulationSmoke(props.databaseUrl)) ?? null
    await refreshOverview()
  } finally {
    smokeRunning.value = false
  }
}

onMounted(async () => {
  config.value = (await getObserverConfig()) ?? null
  if (props.active) {
    refreshOverview()
    startPolling()
  }
})

onUnmounted(stopPolling)
</script>

<template>
  <div class="page-content sim-page">
    <section class="sim-toolbar panel sim-span-12">
      <div class="sim-title">
        <h2><em class="green-dot" /> 模拟套利引擎</h2>
        <span class="muted">只读仪表盘 · 无真实/测试网订单路径</span>
      </div>
      <div class="sim-actions">
        <span v-if="simConfig" class="cfg-summary">
          注入 {{ trimDecimal(simConfig.initial_quote_balance) }}+{{ trimDecimal(simConfig.initial_base_balance) }} · 决策→提交 {{ simConfig.decision_to_submit_ms_min }}–{{ simConfig.decision_to_submit_ms_max }}ms · 补偿预算 {{ trimDecimal(simConfig.compensation_budget) }}
        </span>
        <ElButton size="small" :disabled="smokeRunning" @click="runOneSmoke">
          {{ smokeRunning ? '烟测运行中…' : '运行合成簿烟测' }}
        </ElButton>
        <span class="probe-note">烟测用合成簿（venues s01–s09，symbol 为夹具常量），不是市场数据</span>
        <ElTag v-if="smoke" :type="smoke.external_order_calls === 0 ? 'success' : 'danger'">
          外部订单调用 {{ smoke.external_order_calls }}
        </ElTag>
        <ElTag v-if="smoke" :type="smoke.s01_full_fill_at_worst && smoke.s05_insufficient_funds_rejected ? 'success' : 'warning'">
          S01+S05 探针
        </ElTag>
      </div>
    </section>

    <SimulationStatCards class="sim-span-12" :overview="overview" :loading="loading" />

    <SimulationProfitChart class="sim-span-7" :overview="overview" />
    <SimulationRidge class="sim-span-5" :overview="overview" />

    <SimulationBalanceChart class="sim-span-12" :overview="overview" />

    <SimulationFlow class="sim-span-12" :overview="overview" />

    <SimulationAccountBalances class="sim-span-7" :overview="overview" />
    <SimulationActivity class="sim-span-5" :overview="overview" @open-run="openRunFromActivity" />

    <SimulationTrades
      ref="tradesRef"
      class="sim-span-12"
      :database-url="databaseUrl"
      :overview="overview"
    />
  </div>
</template>

<style scoped>
.sim-page {
  display: grid;
  grid-template-columns: repeat(12, 1fr);
  gap: 12px;
}
.sim-span-12 {
  grid-column: span 12;
}
.sim-span-7 {
  grid-column: span 7;
}
.sim-span-5 {
  grid-column: span 5;
}
/* 行内两面板等高：同一 grid 行取行内最高项为行高，stretch 撑平。 */
@media (max-width: 1100px) {
  .sim-span-7,
  .sim-span-5 {
    grid-column: span 12;
  }
}
.sim-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 16px;
}
.sim-title h2 {
  margin: 0;
  font-size: 15px;
  letter-spacing: 0.02em;
}
.sim-title h2 em {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 8px;
  vertical-align: 1px;
}
.sim-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  justify-content: flex-end;
}
.probe-note {
  color: var(--text-3);
  font-size: var(--fs-10);
  max-width: 260px;
  line-height: 1.4;
}
.cfg-summary {
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: 11px;
  margin-right: 4px;
}
</style>