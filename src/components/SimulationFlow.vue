<script setup lang="ts">
import { computed } from 'vue'
import type { SimulationOverview } from '../types'

const props = defineProps<{
  overview: SimulationOverview | null
}>()

const flow = computed(() => props.overview?.flow ?? null)

/** 执行状态机模板（需求/R04/D7）：EVALUATED → COMPENSATION_DECIDED → COMPLETED/MANUAL_ESCALATED。 */
const EXEC_NODES = [
  { key: 'evaluated', label: 'EVALUATED', x: 10, y: 44 },
  { key: 'compensated', label: 'COMPENSATION_DECIDED', x: 245, y: 44 },
  { key: 'completed', label: 'COMPLETED', x: 470, y: 14 },
  { key: 'escalated', label: 'MANUAL_ESCALATED', x: 470, y: 74 },
] as const

const EXEC_W = 150
const EXEC_H = 30

/** 资金流模板：注入 → 预留 → 双腿成交 → 补偿评估。 */
const FUND_NODES = [
  { key: 'injected', label: '注入', x: 8, y: 30 },
  { key: 'reserved', label: '预留', x: 178, y: 30 },
  { key: 'filled', label: '双腿成交', x: 348, y: 30 },
  { key: 'compensated', label: '补偿评估', x: 518, y: 30 },
] as const

const FUND_W = 110
const FUND_H = 30

function execCount(key: string): number {
  const f = flow.value
  if (!f) return 0
  switch (key) {
    case 'evaluated':
      return f.evaluated_runs
    case 'compensated':
      return f.compensated_runs
    case 'completed':
      return f.completed_runs
    case 'escalated':
      return f.escalated_runs
    default:
      return 0
  }
}

function fundCount(key: string): number {
  const f = flow.value
  if (!f) return 0
  switch (key) {
    case 'injected':
      return f.evaluated_runs // 每 run 注入后即为评估 run
    case 'reserved':
      return f.reserved_runs
    case 'filled':
      return f.filled_runs
    case 'compensated':
      return f.compensated_runs
    default:
      return 0
  }
}
</script>

<template>
  <section class="panel sim-flow">
    <div class="panel-head">
      <h2><em class="blue-dot" /> 执行流转图</h2>
      <span class="muted">不可变事件表聚合 · 最近 run</span>
    </div>
    <div v-if="!flow" class="empty-state">尚无执行数据（先运行烟测）</div>
    <div v-else class="flow-wrap">
      <div class="flow-section">
        <div class="flow-title">执行状态机</div>
        <svg viewBox="0 0 640 126" role="img" aria-label="执行状态机流转图">
          <!-- 连线：EVALUATED → COMPENSATION_DECIDED；COMPENSATION_DECIDED → COMPLETED/MANUAL_ESCALATED -->
          <path d="M160 59 L245 59" class="flow-link" />
          <path d="M395 59 L470 29" class="flow-link" />
          <path d="M395 59 L470 89" class="flow-link" />
          <g v-for="n in EXEC_NODES" :key="n.key">
            <rect class="flow-node" :class="{ lit: execCount(n.key) > 0 }" :x="n.x" :y="n.y" :width="EXEC_W" :height="EXEC_H" rx="6" />
            <text :x="n.x + EXEC_W / 2" :y="n.y + 19" text-anchor="middle" class="node-label">{{ n.label }}</text>
            <text :x="n.x + EXEC_W / 2" :y="n.y + 46" text-anchor="middle" class="node-count">{{ execCount(n.key) }}</text>
          </g>
        </svg>
      </div>
      <div class="flow-section">
        <div class="flow-title">资金流</div>
        <svg viewBox="0 0 640 100" role="img" aria-label="资金流转图">
          <path d="M118 45 L178 45" class="flow-link" />
          <path d="M288 45 L348 45" class="flow-link" />
          <path d="M458 45 L518 45" class="flow-link" />
          <g v-for="n in FUND_NODES" :key="n.key">
            <rect class="flow-node" :class="{ lit: fundCount(n.key) > 0 }" :x="n.x" :y="n.y" :width="FUND_W" :height="FUND_H" rx="6" />
            <text :x="n.x + FUND_W / 2" :y="n.y + 19" text-anchor="middle" class="node-label">{{ n.label }}</text>
            <text :x="n.x + FUND_W / 2" :y="n.y + 46" text-anchor="middle" class="node-count">{{ fundCount(n.key) }}</text>
          </g>
        </svg>
      </div>
    </div>
  </section>
</template>

<style scoped>
.flow-wrap {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 20px;
  align-items: start;
  padding: 14px 16px 16px;
}
.flow-title {
  margin-bottom: 8px;
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: 0.08em;
}
.flow-section svg {
  width: 100%;
  height: auto;
  display: block;
}
.flow-link {
  stroke: var(--border-strong);
  stroke-width: 1.5;
  fill: none;
}
.flow-node {
  fill: var(--bg-inset);
  stroke: var(--border-strong);
  stroke-width: 1;
}
.flow-node.lit {
  fill: var(--accent-soft);
  stroke: var(--accent);
}
.node-label {
  fill: var(--text-2);
  font-size: 11px;
  font-family: var(--font-mono);
}
.flow-node.lit .node-label {
  fill: var(--accent-2);
}
.node-count {
  fill: var(--text-3);
  font-size: 10px;
  font-family: var(--font-mono);
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
  border-top: 1px solid var(--border);
}
</style>
