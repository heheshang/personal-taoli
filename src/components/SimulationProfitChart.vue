<script setup lang="ts">
import { computed } from 'vue'
import type { SimulationOverview } from '../types'
import { trimDecimal } from '../format'

const props = defineProps<{
  overview: SimulationOverview | null
}>()

interface Pt {
  x: number
  y: number
  t: number
  scanned: number
  simulated: number
}

interface ChartData {
  pts: Pt[]
  scanPath: string
  simPath: string
  scannedLast: string
  simulatedLast: string
}

const W = 760
const H = 180
const PAD = 30

/** 累计净盈亏曲线：scanned 预期（虚线）与 simulated 现实（实线），同一坐标系。
 *  前缀和由后端窗口函数算出（`NetProfitPoint` 已是累计值），前端不做浮点累加。 */
const chart = computed<ChartData | null>(() => {
  const pts = props.overview?.cumulative_points
  if (!pts || pts.length < 2) return null

  const t0 = pts[0].executed_at_ms
  const tSpan = pts[pts.length - 1].executed_at_ms - t0 || 1

  let lo = 0
  let hi = 0
  const acc = pts.map(p => {
    const scanned = parseFloat(p.scanned_net_profit)
    const simulated = parseFloat(p.simulated_net_profit)
    lo = Math.min(lo, scanned, simulated)
    hi = Math.max(hi, scanned, simulated)
    return { t: p.executed_at_ms, scanned, simulated }
  })
  const span = hi - lo || 1

  const norm = (v: number) => PAD + ((hi - v) / span) * (H - PAD * 2)
  const out: Pt[] = acc.map(a => ({
    x: PAD + ((a.t - t0) / tSpan) * (W - PAD * 2),
    y: norm(a.simulated),
    t: a.t,
    scanned: a.scanned,
    simulated: a.simulated,
  }))

  const path = (key: 'scanned' | 'simulated') =>
    out.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x},${key === 'scanned' ? norm(p.scanned) : p.y}`).join(' ')

  return {
    pts: out,
    scanPath: path('scanned'),
    simPath: path('simulated'),
    scannedLast: trimDecimal(acc[acc.length - 1].scanned.toFixed(6)),
    simulatedLast: trimDecimal(acc[acc.length - 1].simulated.toFixed(6)),
  }
})

function fmtTime(ms: number): string {
  const d = new Date(ms)
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function fmtVal(v: number): string {
  return trimDecimal(v.toFixed(6))
}
</script>

<template>
  <section class="panel">
    <div class="panel-head">
      <h2><em class="green-dot" /> 累计净盈亏曲线</h2>
      <span class="muted">扫描预期 vs 模拟现实</span>
    </div>
    <div v-if="!chart" class="empty-state">尚无累计曲线数据（至少需要 2 个 run）</div>
    <div v-else class="chart-wrap">
      <svg viewBox="0 0 760 180" preserveAspectRatio="none" role="img" aria-label="累计净盈亏曲线">
        <g v-for="g in 4" :key="g">
          <line class="grid-h" :x1="PAD" :x2="W - PAD" :y1="PAD + (g * (H - PAD * 2)) / 5" :y2="PAD + (g * (H - PAD * 2)) / 5" />
        </g>
        <path class="scan-line" :d="chart.scanPath" />
        <path class="sim-line" :d="chart.simPath" />
        <g v-for="(p, i) in chart.pts" :key="i">
          <circle class="pt-ghost" :cx="p.x" :cy="p.y" r="11">
            <title>{{ fmtTime(p.t) }} 扫描 {{ fmtVal(p.scanned) }} · 模拟 {{ fmtVal(p.simulated) }}</title>
          </circle>
          <circle class="pt-dot" :cx="p.x" :cy="p.y" r="2.5">
            <title>{{ fmtTime(p.t) }} 模拟 {{ fmtVal(p.simulated) }}</title>
          </circle>
        </g>
      </svg>
      <div class="axis">
        <span>首个 run</span>
        <span class="legend">
          <i class="sw sw-scan" /> 扫描预期 {{ chart.scannedLast }}
          <i class="sw sw-sim" /> 模拟现实 {{ chart.simulatedLast }}
        </span>
        <span>最新 run</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
}
.chart-wrap {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
}
.chart-wrap svg {
  flex: 1;
  min-height: 0;
  width: 100%;
}
.grid-h {
  stroke: var(--border);
  stroke-dasharray: 3 4;
}
.scan-line {
  fill: none;
  stroke: #b8860b;
  stroke-width: 1.6;
  stroke-dasharray: 5 4;
}
.sim-line {
  fill: none;
  stroke: #16825e;
  stroke-width: 2.2;
}
.pt-dot {
  fill: #16825e;
}
.pt-ghost {
  fill: transparent;
  cursor: pointer;
}
.legend {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.sw {
  display: inline-block;
  width: 14px;
  vertical-align: middle;
}
.sw-scan {
  border-top: 1px dashed #b8860b;
}
.sw-sim {
  height: 2px;
  background: #16825e;
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
  border-top: 1px solid var(--border);
}
</style>