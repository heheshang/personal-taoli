<script setup lang="ts">
/**
 * Renders a structured analysis report.
 *
 * The shape comes from `docs/agent/report-schema.json`, which the backend sends to
 * the model and validates the result against. Because that schema is **advisory on
 * this runtime** — measured, the model sometimes emits JSON with its own field names
 * — the backend only sets `report` when validation passed. So everything here can
 * assume the three required fields exist, and must still tolerate the optional
 * sections being absent: the domain instructions forbid inventing data, so a field
 * the model could not obtain is *missing*, and the panel hides rather than showing
 * a fabricated zero.
 */
import { computed } from 'vue'
import MarkdownBlock from './MarkdownBlock.vue'

const props = defineProps<{
  report: Record<string, unknown>
}>()

/** Reads a nested value, tolerating an absent section. */
function section<T = Record<string, unknown>>(key: string): T | null {
  const value = props.report[key]
  return value && typeof value === 'object' ? (value as T) : null
}

function text(value: unknown): string | null {
  if (value === null || value === undefined || value === '') return null
  return String(value)
}

/** Formats a number for display, keeping the source's precision. */
function num(value: unknown): string | null {
  if (value === null || value === undefined || value === '') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed.toLocaleString(undefined, { maximumFractionDigits: 4 }) : String(value)
}

function pct(value: unknown): string | null {
  const formatted = num(value)
  return formatted === null ? null : `${formatted}%`
}

const ticker = computed(() => text(props.report.ticker))
const name = computed(() => text(props.report.name))
const summary = computed(() => text(props.report.summary) ?? '')
const verdict = computed(() => text(props.report.verdict))
const score = computed(() => num(props.report.score))
const highlights = computed(() => (props.report.highlights as string[] | undefined) ?? [])

const quote = computed(() => section('quote'))
const financial = computed(() => section('financial'))
const technical = computed(() => section('technical'))
const valuation = computed(() => section('valuation'))
const events = computed(() => (props.report.events as { date: string; title: string }[] | undefined) ?? [])
const integrity = computed(() => section('integrity'))

const health = computed(() => (financial.value?.health as Record<string, unknown>) ?? null)
const dupont = computed(() => (financial.value?.dupont as Record<string, unknown>) ?? null)
const dcf = computed(() => (valuation.value?.dcf as Record<string, unknown>) ?? null)
const sensitivity = computed(
  () => (dcf.value?.sensitivity as { wacc: string; terminalGrowth: string; value: number }[] | undefined) ?? [],
)

/** The quote rows that have a value; the rest are omitted, not shown as zeros. */
const quoteRows = computed(() =>
  [
    { label: '最新价', value: num(quote.value?.price) },
    { label: '昨收', value: num(quote.value?.prevClose) },
    { label: '涨跌幅', value: pct(quote.value?.changePct) },
    { label: '总市值', value: text(quote.value?.marketCap) },
    { label: 'PE(TTM)', value: num(quote.value?.peTtm) },
    { label: 'PB', value: text(quote.value?.pb) },
  ].filter(row => row.value !== null),
)

const verdictTone = computed(() => {
  switch (verdict.value) {
    case '看多':
      return 'ok'
    case '看空':
      return 'bad'
    default:
      return 'warn'
  }
})
</script>

<template>
  <section class="report">
    <header class="report-head">
      <h3 class="report-title">
        {{ name ?? ticker }}
        <span v-if="ticker && name" class="report-ticker">{{ ticker }}</span>
      </h3>
      <div class="report-badges">
        <span v-if="verdict" class="codex-chip" :class="verdictTone">{{ verdict }}</span>
        <span v-if="score" class="codex-chip">评分 {{ score }}</span>
        <span v-if="text(report.mode)" class="codex-chip">{{ text(report.mode) }}</span>
      </div>
    </header>

    <div v-if="text(report.industry) || text(report.dataAt)" class="report-meta">
      <span v-if="text(report.industry)">{{ text(report.industry) }}</span>
      <span v-if="text(report.dataAt)">数据 {{ text(report.dataAt) }}</span>
      <span v-if="text(report.listedDate)">上市 {{ text(report.listedDate) }}</span>
    </div>

    <!-- 结论：Markdown，因为它是叙述而非字段 -->
    <MarkdownBlock v-if="summary" class="report-summary" :source="summary" />

    <ul v-if="highlights.length" class="report-highlights">
      <li v-for="(item, index) in highlights" :key="index">{{ item }}</li>
    </ul>

    <!-- 行情 -->
    <div v-if="quoteRows.length" class="report-panel">
      <h4>行情</h4>
      <div class="report-grid">
        <div v-for="row in quoteRows" :key="row.label" class="report-cell">
          <span class="report-cell-label">{{ row.label }}</span>
          <strong class="report-cell-value">{{ row.value }}</strong>
        </div>
      </div>
    </div>

    <!-- 财务 -->
    <div v-if="health || dupont || num(financial?.revenueGrowthYoy) !== null" class="report-panel">
      <h4>财务</h4>
      <div class="report-grid">
        <div v-if="num(financial?.revenueGrowthYoy) !== null" class="report-cell">
          <span class="report-cell-label">营收同比</span>
          <strong class="report-cell-value">{{ pct(financial?.revenueGrowthYoy) }}</strong>
          <span v-if="text(financial?.revenueGrowthPeriod)" class="report-cell-note">
            {{ text(financial?.revenueGrowthPeriod) }}
          </span>
        </div>
        <div v-if="num(financial?.netProfitYoyPct) !== null" class="report-cell">
          <span class="report-cell-label">净利同比</span>
          <strong class="report-cell-value">{{ pct(financial?.netProfitYoyPct) }}</strong>
        </div>
        <div v-if="num(health?.netMarginPct) !== null" class="report-cell">
          <span class="report-cell-label">净利率</span>
          <strong class="report-cell-value">{{ pct(health?.netMarginPct) }}</strong>
        </div>
        <div v-if="num(health?.debtRatio) !== null" class="report-cell">
          <span class="report-cell-label">资产负债率</span>
          <strong class="report-cell-value">{{ pct(health?.debtRatio) }}</strong>
        </div>
        <div v-if="num(health?.currentRatio) !== null" class="report-cell">
          <span class="report-cell-label">流动比率</span>
          <strong class="report-cell-value">{{ num(health?.currentRatio) }}</strong>
        </div>
        <div v-if="num(dupont?.roeReconstructedPct) !== null" class="report-cell">
          <span class="report-cell-label">ROE（杜邦重构）</span>
          <strong class="report-cell-value">{{ pct(dupont?.roeReconstructedPct) }}</strong>
          <span v-if="text(dupont?.roeQuality)" class="report-cell-note">
            {{ text(dupont?.roeQuality) }}
          </span>
        </div>
      </div>
    </div>

    <!-- 技术面 -->
    <div v-if="technical && Object.keys(technical).length" class="report-panel">
      <h4>技术面</h4>
      <div class="report-grid">
        <div v-if="text(technical.stage)" class="report-cell">
          <span class="report-cell-label">阶段</span>
          <strong class="report-cell-value">{{ text(technical.stage) }}</strong>
        </div>
        <div v-if="text(technical.maAlignment)" class="report-cell">
          <span class="report-cell-label">均线</span>
          <strong class="report-cell-value">{{ text(technical.maAlignment) }}</strong>
        </div>
        <div v-if="num(technical.rsi14) !== null" class="report-cell">
          <span class="report-cell-label">RSI(14)</span>
          <strong class="report-cell-value">{{ num(technical.rsi14) }}</strong>
        </div>
        <div v-if="text(technical.ytdReturn)" class="report-cell">
          <span class="report-cell-label">年初至今</span>
          <strong class="report-cell-value">{{ text(technical.ytdReturn) }}</strong>
        </div>
        <div v-if="num(technical.ma20) !== null" class="report-cell">
          <span class="report-cell-label">MA20</span>
          <strong class="report-cell-value">{{ num(technical.ma20) }}</strong>
        </div>
        <div v-if="num(technical.ma200) !== null" class="report-cell">
          <span class="report-cell-label">MA200</span>
          <strong class="report-cell-value">{{ num(technical.ma200) }}</strong>
        </div>
      </div>
    </div>

    <!-- 估值 -->
    <div v-if="dcf || text(valuation?.peQuantile)" class="report-panel">
      <h4>估值</h4>
      <div class="report-grid">
        <div v-if="num(dcf?.intrinsicPerShare) !== null" class="report-cell">
          <span class="report-cell-label">DCF 内在价值</span>
          <strong class="report-cell-value">{{ num(dcf?.intrinsicPerShare) }}</strong>
        </div>
        <div v-if="num(dcf?.safetyMarginPct) !== null" class="report-cell">
          <span class="report-cell-label">安全边际</span>
          <strong class="report-cell-value">{{ pct(dcf?.safetyMarginPct) }}</strong>
        </div>
        <div v-if="num(dcf?.wacc) !== null" class="report-cell">
          <span class="report-cell-label">WACC</span>
          <strong class="report-cell-value">{{ num(dcf?.wacc) }}</strong>
        </div>
        <div v-if="text(valuation?.peQuantile)" class="report-cell">
          <span class="report-cell-label">PE 分位</span>
          <strong class="report-cell-value">{{ text(valuation?.peQuantile) }}</strong>
        </div>
      </div>
      <table v-if="sensitivity.length" class="report-table">
        <thead>
          <tr><th>WACC</th><th>永续增长</th><th>估值</th></tr>
        </thead>
        <tbody>
          <tr v-for="(row, index) in sensitivity" :key="index">
            <td>{{ row.wacc }}</td>
            <td>{{ row.terminalGrowth }}</td>
            <td>{{ num(row.value) }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- 事件 -->
    <div v-if="events.length" class="report-panel">
      <h4>重大事件</h4>
      <ul class="report-events">
        <li v-for="(event, index) in events" :key="index">
          <span class="report-event-date">{{ event.date }}</span>
          <span>{{ event.title }}</span>
        </li>
      </ul>
    </div>

    <!-- 数据完整度：让读者知道这份结论建立在多少数据上 -->
    <div v-if="integrity" class="report-panel report-integrity">
      <h4>数据完整度</h4>
      <div class="report-grid">
        <div v-if="num(integrity.coveragePct) !== null" class="report-cell">
          <span class="report-cell-label">覆盖率</span>
          <strong class="report-cell-value">{{ pct(integrity.coveragePct) }}</strong>
        </div>
        <div v-if="num(integrity.passedChecks) !== null" class="report-cell">
          <span class="report-cell-label">检查通过</span>
          <strong class="report-cell-value">
            {{ num(integrity.passedChecks) }} / {{ num(integrity.totalChecks) ?? '—' }}
          </strong>
        </div>
      </div>
      <p v-if="(integrity.missingDimensions as string[] | undefined)?.length" class="report-missing">
        <b>缺失维度</b>：{{ (integrity.missingDimensions as string[]).join('、') }}
      </p>
      <p v-if="(integrity.fallbackDimensions as string[] | undefined)?.length" class="report-missing">
        <b>降级维度</b>：{{ (integrity.fallbackDimensions as string[]).join('、') }}
      </p>
    </div>

    <p class="report-disclaimer">
      以上为分析结论，不构成投资建议。缺失字段表示本次未能取得该数据。
    </p>
  </section>
</template>
