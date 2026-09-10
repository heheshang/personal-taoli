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
    smokeRuns: o.smoke_runs,
    liveRuns: o.live_runs,
  }
})

/** 来源提示：合成簿探针不是市场数据；实时 run 为 0 时说明原因，避免把探针当仿真结果。 */
const provenance = computed(() => {
  const o = props.overview
  if (!o || o.total_runs === 0) return null
  if (o.live_runs === 0) {
    return {
      tone: 'warn' as const,
      text: `全部 ${o.smoke_runs} 条为合成簿探针（venues s01–s09，非市场数据）；实时 run 0 条 —— 准入未通过（凭证/资格未配置，见控制台账户状态）`,
    }
  }
  if (o.smoke_runs === 0) {
    return { tone: 'live' as const, text: `全部 ${o.live_runs} 条为实时观察 run` }
  }
  return {
    tone: 'mixed' as const,
    text: `实时 ${o.live_runs} 条 · 合成簿探针 ${o.smoke_runs} 条（探针非市场数据）`,
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

    <div v-if="provenance" class="provenance" :class="provenance.tone">
      <b>数据来源</b>
      <span>{{ provenance.text }}</span>
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
/* 来源提示：探针 / 实时必须一眼可辨，避免合成簿被读成市场仿真。 */
.provenance {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 8px;
  padding: 6px 10px;
  border-radius: 6px;
  border: 1px solid var(--border);
  font-size: var(--fs-11);
  line-height: 1.5;
}
.provenance b {
  flex: none;
  color: var(--text-2);
  letter-spacing: 0.06em;
}
.provenance.warn {
  border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  color: var(--text-1);
}
.provenance.mixed {
  border-color: color-mix(in srgb, var(--danger) 30%, transparent);
}
.provenance.live {
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
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