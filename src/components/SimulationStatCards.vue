<script setup lang="ts">
import { computed } from 'vue'
import { ElTag } from 'element-plus'
import type { SimulationOverview } from '../types'
import { trimDecimal } from '../format'

const props = defineProps<{
  overview: SimulationOverview | null
  loading: boolean
}>()

const stats = computed(() => {
  const o = props.overview
  if (!o) return null
  return {
    totalRuns: o.total_runs,
    totalSuccess: o.total_success,
    profitRuns: o.profit_runs,
    totalNetProfit: o.total_net_profit,
    scannedNetProfit: o.scanned_net_profit,
    successRate: o.total_runs > 0 ? Math.round((o.total_success / o.total_runs) * 100) : 0,
  }
})

/** 场景中文名（与 F-02 场景语义一致）。 */
const SCENARIO_LABELS: Record<string, string> = {
  NORMAL: '正常成交',
  DEPTH_SHORTFALL: '深度不足',
  COMPETED_AWAY: '被抢单',
  REJECTED: '拒绝',
}

function profitClass(val: string): string {
  const n = parseFloat(val)
  if (isNaN(n) || n === 0) return 'neutral'
  return n > 0 ? 'profit' : 'loss'
}

function fmtNum(val: string): string {
  const n = parseFloat(val)
  return isNaN(n) ? '—' : n.toLocaleString('zh-CN', { maximumFractionDigits: 6 })
}
</script>

<template>
  <div>
    <div class="metric-grid sim-stats">
      <div class="metric-card">
        <small>模拟 RUN 总数</small>
        <strong>{{ stats?.totalRuns ?? '—' }}</strong>
        <div class="sub-note">成功 {{ stats?.totalSuccess ?? '—' }} · 成功率 {{ stats?.successRate ?? '—' }}%</div>
      </div>
      <div class="metric-card">
        <small>盈利 RUN 数</small>
        <strong>{{ stats?.profitRuns ?? '—' }}</strong>
      </div>
      <div class="metric-card accent">
        <small>合计模拟净盈亏</small>
        <strong :class="stats ? profitClass(stats.totalNetProfit) : ''">
          {{ stats ? fmtNum(stats.totalNetProfit) : '—' }}
        </strong>
        <div class="sub-note">扫描预期累计 {{ stats ? fmtNum(stats.scannedNetProfit) : '—' }}</div>
      </div>
      <div class="metric-card">
        <small>运行负载</small>
        <strong>{{ loading ? '刷新中' : '就绪' }}</strong>
      </div>
    </div>

    <div v-if="overview?.by_scenario.length" class="scenario-strip">
      <ElTag
        v-for="s in overview.by_scenario"
        :key="s.scenario"
        size="small"
        effect="plain"
        class="scenario-tag"
      >
        {{ SCENARIO_LABELS[s.scenario] ?? s.scenario }} × {{ s.count }}
      </ElTag>
    </div>
  </div>
</template>

<style scoped>
.sim-stats .metric-card strong {
  font-size: var(--fs-18);
}
.sim-stats .sub-note {
  margin-top: 6px;
  color: var(--text-3);
  font-size: var(--fs-10);
  font-family: var(--font-mono);
}
.profit {
  color: var(--accent);
}
.loss {
  color: var(--danger);
}
.neutral {
  color: var(--text-2);
}
.scenario-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 2px;
}
.scenario-tag {
  --el-tag-font-size: 11px;
}
</style>