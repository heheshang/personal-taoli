<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { RidgeBucket, SimulationOverview } from '../types'
import { trimDecimal } from '../format'

const props = defineProps<{
  overview: SimulationOverview | null
}>()

const buckets = computed(() => props.overview?.ridge ?? [])

/** 每行预留的文字标签高度（日期/样本/中位 一行 + 轴一行）。 */
const LABEL_H = 16
const AXIS_H = 12

const visualEl = ref<HTMLElement | null>(null)
/** 视觉区实测像素尺寸；svg viewBox 直接用它，避免拉伸畸变并填满等高面板。 */
const box = ref({ w: 380, h: 200 })

let ro: ResizeObserver | null = null
function measure() {
  const el = visualEl.value
  if (!el) return
  const r = el.getBoundingClientRect()
  // 面板隐藏（祖先 display:none）时尺寸为 0：保留上次有效值，不写脏数据。
  if (r.width < 1 || r.height < 1) return
  // 亚像素抖动不重写，避免 ResizeObserver 自反馈循环。
  if (Math.abs(box.value.w - r.width) < 0.5 && Math.abs(box.value.h - r.height) < 0.5) return
  box.value = { w: r.width, h: r.height }
}
onMounted(() => {
  if (typeof ResizeObserver !== 'undefined') ro = new ResizeObserver(measure)
  measure()
})
onBeforeUnmount(() => ro?.disconnect())
// visualEl 只在 buckets 非空时才存在，故必须跟着 ref 走，不能在 onMounted 里绑定。
watch(visualEl, (el, prev) => {
  if (prev) ro?.unobserve(prev)
  if (!el) return
  ro?.observe(el)
  measure()
})
watch(buckets, () => nextTick(measure))

interface Row {
  path: string
  medianX: number
  label: string
  sample: string
  medianVal: string
  min: number
  max: number
  top: number
  curveH: number
}

const rows = computed<Row[]>(() => {
  const n = buckets.value.length
  if (!n) return []
  const rowH = box.value.h / n
  const curveH = Math.max(20, rowH - LABEL_H - AXIS_H)
  return buckets.value.map((b, index) => computeRow(b, index, box.value.w, rowH, curveH))
})

/** KDE：对桶内净盈亏点叠加高斯核，得归一化密度曲线 path（0 基线在行底）。 */
function computeRow(b: RidgeBucket, index: number, width: number, rowH: number, curveH: number): Row {
  const values = b.nets.map(v => parseFloat(v)).filter(n2 => !isNaN(n2))
  const lo = Math.min(...values)
  const hi = Math.max(...values)
  const span = hi - lo || 1
  const bins = 56
  const dent = (span / 24) ** 2 // 核宽 ~ span/24

  const top = index * rowH + LABEL_H
  const xs: number[] = []
  const ys: number[] = []
  for (let i = 0; i <= bins; i++) {
    const x = lo + (span * i) / bins
    let d = 0
    for (const v of values) d += Math.exp(-(((v - x) ** 2) / dent) / 2)
    if (!isFinite(d) || d === 0) d = 1e-9
    xs.push((i / bins) * width)
    ys.push(d)
  }
  const maxD = Math.max(...ys)
  const path = xs
    .map((x, i) => `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${(top + curveH - (ys[i] / maxD) * curveH).toFixed(1)}`)
    .join(' ')

  const sorted = [...values].sort((a, b2) => a - b2)
  const mid = Math.floor(sorted.length / 2)
  const med = sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2

  const d = new Date(b.bucket_start_ms)
  return {
    path,
    medianX: lo === hi ? width / 2 : ((med - lo) / span) * width,
    label: `${d.getMonth() + 1}/${d.getDate()}`,
    sample: `n=${b.nets.length}`,
    medianVal: trimDecimal(med.toFixed(6)),
    min: lo,
    max: hi,
    top,
    curveH,
  }
}

const overallMedian = computed(() => {
  const all = buckets.value
    .flatMap(x => x.nets.map(n => parseFloat(n)))
    .filter(n => !isNaN(n))
    .sort((a, b) => a - b)
  if (!all.length) return '—'
  const m = Math.floor(all.length / 2)
  const med = all.length % 2 ? all[m] : (all[m - 1] + all[m]) / 2
  return trimDecimal(med.toFixed(6))
})

function fmtVal(v: number): string {
  return trimDecimal(v.toFixed(6))
}
</script>

<template>
  <section class="panel sim-ridge">
    <div class="panel-head">
      <h2><em class="orange-dot" /> 套利机会山脊图</h2>
      <span class="muted">按日桶分布 · 模拟净收益密度 · 中位线</span>
    </div>
    <div v-if="!buckets.length" class="empty-state">尚无山脊数据（需至少一个含数据的日桶）</div>
    <div v-else class="ridge-body">
      <div class="ridge-stats">
        <span>日桶 <b>{{ buckets.length }}</b></span>
        <span>样本点 <b>{{ buckets.reduce((a, b) => a + b.nets.length, 0) }}</b></span>
        <span>整体中位 <b>{{ overallMedian }}</b></span>
      </div>
      <div class="ridge-visual" ref="visualEl">
        <svg
          :viewBox="`0 0 ${box.w} ${box.h}`"
          :width="box.w"
          :height="box.h"
          class="ridge-svg"
          role="img"
          aria-label="套利机会日净收益密度分布"
        >
          <g v-for="(r, i) in rows" :key="i">
            <text class="ridge-date" x="2" :y="r.top - 4">
              {{ r.label }} · {{ r.sample }} ·
              <tspan :fill="r.medianVal.startsWith('-') ? 'var(--danger)' : 'var(--accent)'">{{ r.medianVal }}</tspan>
            </text>
            <path class="ridge-curve" :d="r.path" />
            <line class="ridge-median" :x1="r.medianX" :x2="r.medianX" :y1="r.top" :y2="r.top + r.curveH" />
            <text class="ridge-axis" x="2" :y="r.top + r.curveH + 10">{{ fmtVal(r.min) }}</text>
            <text class="ridge-axis" :x="box.w - 2" text-anchor="end" :y="r.top + r.curveH + 10">{{ fmtVal(r.max) }}</text>
          </g>
        </svg>
      </div>
    </div>
  </section>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
}
.ridge-body {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: 168px 1fr;
  padding: 12px 18px 20px;
}
.ridge-stats {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 14px;
  color: var(--text-3);
  font-size: var(--fs-11);
}
.ridge-stats b {
  display: block;
  margin-top: 5px;
  color: var(--text-2);
  font-size: 16px;
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.ridge-visual {
  border-left: 1px solid var(--border);
  padding-left: 14px;
  overflow: hidden;
  min-height: 0;
}
.ridge-svg {
  display: block;
}
.ridge-curve {
  fill: none;
  stroke: var(--accent);
  stroke-width: 1.5;
  vector-effect: non-scaling-stroke;
}
.ridge-median {
  stroke: var(--danger);
  stroke-width: 1;
  stroke-dasharray: 3 3;
  opacity: 0.85;
}
.ridge-date {
  fill: var(--text-2);
  font-size: 10px;
  font-family: var(--font-mono);
}
.ridge-axis {
  fill: var(--text-3);
  font-size: 9px;
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
