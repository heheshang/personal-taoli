<script setup lang="ts">
import { computed } from 'vue'
import type { BalancePoint, SimulationOverview } from '../types'
import { trimDecimal } from '../format'

const props = defineProps<{
  overview: SimulationOverview | null
}>()

const W = 760
const H = 180
const PAD = 30

/** 每 run 两账户（quote 腿 / base 腿）的三节点快照分组，最新在前。 */
const accountCards = computed<BalancePoint[][]>(() => {
  const hist = props.overview?.balance_history ?? []
  const byRun = new Map<string, BalancePoint[]>()
  for (const p of hist) {
    const arr = byRun.get(p.run_id)
    if (arr) arr.push(p)
    else byRun.set(p.run_id, [p])
  }
  return [...byRun.values()].sort((a, b) => b[0].observed_at_ms - a[0].observed_at_ms)
})

/** 曲线：按资产分线（quote / base 两条资产线），x=时间，y=total。 */
const curves = computed<{ legend: string[]; paths: string[]; dots: { x: number; y: number; legend: string; colorIdx: number; label: string }[] } | null>(() => {
  const hist = (props.overview?.balance_history ?? []).filter(p => p.node === 'evaluated')
  if (hist.length < 2) return null
  const byAsset = new Map<string, BalancePoint[]>()
  for (const p of hist) {
    const arr = byAsset.get(p.asset)
    if (arr) arr.push(p)
    else byAsset.set(p.asset, [p])
  }
  const assets = [...byAsset.keys()].sort()
  const ordered = assets.flatMap(a => byAsset.get(a)!).sort((a, b) => a.observed_at_ms - b.observed_at_ms)
  const t0 = ordered[0].observed_at_ms
  const tSpan = ordered[ordered.length - 1].observed_at_ms - t0 || 1
  let lo = Infinity
  let hi = -Infinity
  for (const p of ordered) {
    const v = parseFloat(p.total)
    lo = Math.min(lo, v)
    hi = Math.max(hi, v)
  }
  const span = hi - lo || 1
  const norm = (v: number) => PAD + ((hi - v) / span) * (H - PAD * 2)
  const xOf = (p: BalancePoint) => PAD + ((p.observed_at_ms - t0) / tSpan) * (W - PAD * 2)

  const colors = ['#16825e', '#4689ae', '#b8860b']
  const paths: string[] = []
  const dots: { x: number; y: number; legend: string; colorIdx: number; label: string }[] = []
  const legend: string[] = []
  assets.forEach((asset, i) => {
    const pts = [...byAsset.get(asset)!].sort((a, b) => a.observed_at_ms - b.observed_at_ms)
    const color = colors[i % colors.length]
    legend.push(asset)
    paths.push(pts.map((p, j) => `${j === 0 ? 'M' : 'L'}${xOf(p).toFixed(1)},${norm(parseFloat(p.total)).toFixed(1)}`).join(' '))
    for (const p of pts) {
      dots.push({
        x: xOf(p),
        y: norm(parseFloat(p.total)),
        legend: asset,
        colorIdx: i,
        label: `${p.account_id} ${p.venue} ${p.asset} node=${p.node} total=${trimDecimal(p.total)}`,
      })
    }
  })
  // 兼容：用 node 颜色信息（dot 均取 evaluated 数据）
  return { legend, paths, dots }
})

/** 账户现值表格：已拆分为独立组件 SimulationAccountBalances.vue */

function fmtTime(ms: number): string {
  const d = new Date(ms)
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function legLabel(p: BalancePoint): string {
  return p.asset === 'quote' || p.asset === 'USDT' ? `买腿 · ${p.venue}` : `卖腿 · ${p.venue}`
}

const NODE_LABELS: Record<string, string> = {
  fund: '注入后',
  filled: '成交后',
  evaluated: '评估后',
}
</script>

<template>
  <section class="panel">
    <div class="panel-head">
      <h2><em class="teal-dot" /> 账户变化图</h2>
      <span class="muted">每 run 买腿 / 卖腿两账户 · 三节点快照</span>
    </div>

    <template v-if="overview && overview.balance_history.length">
      <div class="chart-wrap">
        <svg v-if="curves" viewBox="0 0 760 180" preserveAspectRatio="none" role="img" aria-label="账户余额变化曲线">
          <g v-for="g in 4" :key="g">
            <line class="grid-h" :x1="PAD" :x2="W - PAD" :y1="PAD + (g * (H - PAD * 2)) / 5" :y2="PAD + (g * (H - PAD * 2)) / 5" />
          </g>
          <path v-for="(pa, i) in curves.paths" :key="i" class="asset-line" :stroke="['#16825e', '#4689ae', '#b8860b'][i % 3]" :d="pa" />
          <g v-for="(d, i) in curves.dots" :key="i">
            <circle class="pt-ghost" :cx="d.x" :cy="d.y" r="10">
              <title>{{ d.label }}</title>
            </circle>
            <circle class="pt-dot" :cx="d.x" :cy="d.y" r="2.5" :fill="['#16825e', '#4689ae', '#b8860b'][d.colorIdx % 3]">
              <title>{{ d.label }}</title>
            </circle>
          </g>
        </svg>
        <div v-else class="empty-state">余额历史点不足（少于 2 个评估点）</div>
        <div v-if="curves" class="axis">
          <span>早期</span>
          <span class="legend">
            <i v-for="(l, i) in curves.legend" :key="l" class="sw" :style="{ background: ['#16825e', '#4689ae', '#b8860b'][i % 3] }">{{ l }}</i>
          </span>
          <span>最新评估</span>
        </div>
      </div>

      <!-- 每 run 两账户详情卡：买腿 / 卖腿 × 三节点 -->
      <div v-if="accountCards.length" class="account-cards">
        <div v-for="(pts, i) in accountCards.slice(0, 4)" :key="pts[0].run_id" class="account-card">
          <div class="ac-head">
            <b>{{ pts[0].run_id }}</b>
            <span class="muted">{{ fmtTime(pts[0].observed_at_ms) }}</span>
          </div>
          <div class="ac-legs">
            <div v-for="(p, j) in pts" :key="j" class="ac-leg">
              <div class="ac-leg-title">
                <b>{{ legLabel(p) }}</b>
                <span class="muted">{{ p.asset }}</span>
              </div>
              <div v-if="pts.filter(q => q.asset === p.asset).length >= 3" class="ac-nodes">
                <div v-for="(q, k) in pts.filter(r => r.asset === p.asset).sort((x, y) => x.observed_at_ms - y.observed_at_ms)" :key="k" class="ac-node">
                  <span>{{ NODE_LABELS[q.node] ?? q.node }}</span>
                  <b>{{ trimDecimal(q.total) }}</b>
                </div>
              </div>
              <div v-else class="ac-nodes">
                <div class="ac-node">
                  <span>total</span>
                  <b>{{ trimDecimal(p.total) }}</b>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

    </template>
    <div v-else class="empty-state">尚无账户余额数据（先运行烟测）</div>
  </section>
</template>

<style scoped>
.chart-wrap svg {
  height: 180px;
}
.grid-h {
  stroke: var(--border);
  stroke-dasharray: 3 4;
}
.asset-line {
  fill: none;
  stroke-width: 2;
}
.pt-dot {
  stroke: #fff;
  stroke-width: 1;
}
.pt-ghost {
  fill: transparent;
  cursor: pointer;
}
.legend {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.sw {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  width: auto;
  height: 2px;
  padding: 0 4px;
  color: var(--text-3);
  font-style: normal;
  font-size: var(--fs-10);
}
.account-cards {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
  padding: 0 16px 14px;
}
.account-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  background: var(--bg-inset);
}
.ac-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 6px;
}
.ac-head b {
  font-family: var(--font-mono);
  font-size: var(--fs-10);
  max-width: 70%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ac-legs {
  display: flex;
  gap: 10px;
}
.ac-leg {
  flex: 1;
  border-left: 2px solid var(--border-strong);
  padding-left: 8px;
}
.ac-leg-title {
  display: flex;
  justify-content: space-between;
  margin-bottom: 4px;
}
.ac-leg-title b {
  font-size: var(--fs-11);
  color: var(--text-2);
}
.ac-nodes {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.ac-node {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-10);
}
.ac-node span {
  color: var(--text-3);
}
.ac-node b {
  font-family: var(--font-mono);
  color: var(--text-1);
  font-variant-numeric: tabular-nums;
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
  border-top: 1px solid var(--border);
}
</style>