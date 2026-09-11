<script setup lang="ts">
import { computed } from 'vue'
import { stockAnalysis600519 as report } from '../data/stockAnalysis'

const emit = defineEmits<{
  openSession: []
}>()

const revenueMax = Math.max(...report.financial.revenue)
const netProfitMax = Math.max(...report.financial.netProfit)
const roeMin = Math.min(...report.financial.roe)
const roeMax = Math.max(...report.financial.roe)

function barHeight(value: number, max: number): string {
  return `${Math.max(4, (value / max) * 100)}%`
}

function polylinePoints(values: readonly number[]): string {
  const width = 620
  const height = 150
  const horizontalPadding = 20
  const verticalPadding = 18
  const span = roeMax - roeMin || 1
  const step = (width - horizontalPadding * 2) / (values.length - 1)

  return values
    .map((value, index) => {
      const x = horizontalPadding + index * step
      const y =
        height -
        verticalPadding -
        ((value - roeMin) / span) * (height - verticalPadding * 2)
      return `${x},${y}`
    })
    .join(' ')
}

const roePoints = polylinePoints(report.financial.roe)
const priceRangePosition = computed(() => {
  const { price } = report.quote
  const { yearLow, yearHigh } = report.technical
  return `${Math.max(0, Math.min(100, ((price - yearLow) / (yearHigh - yearLow)) * 100))}%`
})

const summaryPoints = [
  '盈利能力和资产负债表数据较强',
  '增长、现金流与估值口径存在冲突',
  '技术面处于 Stage 4 下跌',
  '同行对标和定性复核尚未完成',
]
</script>

<template>
  <section class="stock-report">
    <div class="report-lead">
      <div class="report-identity">
        <div class="report-eyebrow">只读个股分析</div>
        <h1>{{ report.name }} <span>{{ report.ticker }}</span></h1>
        <p>{{ report.industry }} · 上市日 {{ report.listedDate }}</p>
      </div>
      <div class="report-actions">
        <span class="report-chip warn">{{ report.mode }}</span>
        <button class="report-session-btn" type="button" @click="emit('openSession')">
          打开 Codex 会话
        </button>
      </div>
    </div>

    <div class="report-boundary">
      <strong>边界</strong>
      <span>只读分析与报告，不构成交易决策；Stage 2 定性复核尚未运行。</span>
      <time>{{ report.dataAt }}</time>
    </div>

    <div class="quote-grid">
      <div class="quote-cell primary">
        <span>最新价</span>
        <strong>{{ report.quote.price }}</strong>
        <small>{{ report.quote.changePct }}</small>
      </div>
      <div class="quote-cell">
        <span>总市值</span>
        <strong>{{ report.quote.marketCap }}</strong>
        <small>基础行情字段</small>
      </div>
      <div class="quote-cell">
        <span>PE 口径 A</span>
        <strong>{{ report.quote.peTtm }}</strong>
        <small>pe_ttm</small>
      </div>
      <div class="quote-cell conflict">
        <span>PE 口径 B</span>
        <strong>{{ report.quote.peValuation }}</strong>
        <small>valuation.data.pe</small>
      </div>
      <div class="quote-cell">
        <span>PB</span>
        <strong>{{ report.quote.pb }}</strong>
        <small>两个维度一致</small>
      </div>
      <div class="quote-cell">
        <span>技术阶段</span>
        <strong class="text-value">{{ report.technical.stage }}</strong>
        <small>{{ report.technical.maAlignment }}</small>
      </div>
    </div>

    <div class="report-summary">
      <div>
        <span class="section-kicker">结论摘要</span>
        <h2>基本面较强，但尚不足以形成完整深度结论</h2>
      </div>
      <ul>
        <li v-for="point in summaryPoints" :key="point">{{ point }}</li>
      </ul>
    </div>

    <div class="report-grid">
      <section class="report-section finance-section">
        <header>
          <div>
            <span class="section-kicker">财务质量</span>
            <h2>高 ROE、高净利率、低杠杆</h2>
          </div>
          <span class="section-note">{{ report.financial.period }}</span>
        </header>

        <div class="metric-pairs">
          <div>
            <span>2025 ROE</span>
            <strong>{{ report.financial.roe.at(-1) }}</strong>
          </div>
          <div>
            <span>资产负债率</span>
            <strong>{{ report.financial.health.debtRatio }}</strong>
          </div>
          <div>
            <span>流动比率</span>
            <strong>{{ report.financial.health.currentRatio }}</strong>
          </div>
          <div>
            <span>净利率</span>
            <strong>{{ report.financial.health.netMarginPct }}</strong>
          </div>
        </div>

        <div class="chart-head">
          <span>ROE 历史</span>
          <span>DuPont 重建 {{ report.financial.dupont.roeReconstructedPct }}</span>
        </div>
        <div class="roe-chart">
          <svg viewBox="0 0 620 150" preserveAspectRatio="none" aria-label="2020 至 2025 ROE 趋势">
            <line x1="20" y1="18" x2="20" y2="132" class="axis-line" />
            <line x1="20" y1="132" x2="600" y2="132" class="axis-line" />
            <polyline :points="roePoints" class="roe-line" />
          </svg>
          <div class="series-labels">
            <div v-for="(year, index) in report.financial.years" :key="year">
              <b>{{ report.financial.roe[index] }}</b>
              <span>{{ year }}</span>
            </div>
          </div>
        </div>
      </section>

      <section class="report-section growth-section">
        <header>
          <div>
            <span class="section-kicker">增长</span>
            <h2>年度序列回落，最新同比口径冲突</h2>
          </div>
        </header>

        <div class="growth-bars">
          <div v-for="(year, index) in report.financial.years" :key="year" class="growth-year">
            <div class="bar-pair">
              <i
                class="revenue-bar"
                :style="{ height: barHeight(report.financial.revenue[index], revenueMax) }"
                :title="`营收 ${report.financial.revenue[index]}`"
              />
              <i
                class="profit-bar"
                :style="{ height: barHeight(report.financial.netProfit[index], netProfitMax) }"
                :title="`净利润 ${report.financial.netProfit[index]}`"
              />
            </div>
            <b>{{ year }}</b>
            <span>{{ report.financial.revenue[index] }}</span>
            <small>{{ report.financial.netProfit[index] }}</small>
          </div>
        </div>

        <div class="growth-legend">
          <span><i class="legend-revenue" />营收</span>
          <span><i class="legend-profit" />净利润</span>
        </div>

        <div class="conflict-list">
          <div>
            <span>2026-06-30 营收同比</span>
            <strong>{{ report.financial.revenueGrowthYoy }}</strong>
            <small>{{ report.financial.revenueGrowthLabel }}</small>
          </div>
          <div>
            <span>评分维度标签</span>
            <strong>-1.2</strong>
            <small>与上方字段冲突</small>
          </div>
          <div>
            <span>净利同比模型字段</span>
            <strong>{{ report.financial.netProfitYoyPct }}</strong>
            <small>period/source 未提供</small>
          </div>
        </div>
      </section>

      <section class="report-section technical-section">
        <header>
          <div>
            <span class="section-kicker">技术面</span>
            <h2>价格低于主要均线，动能偏弱</h2>
          </div>
          <span class="section-note">654 根 K 线</span>
        </header>

        <div class="technical-stats">
          <div>
            <span>RSI 14</span>
            <strong>{{ report.technical.rsi14 }}</strong>
          </div>
          <div>
            <span>YTD 收益</span>
            <strong>{{ report.technical.ytdReturn }}</strong>
          </div>
          <div>
            <span>距一年高点</span>
            <strong>{{ report.technical.pctFromYearHigh }}</strong>
          </div>
        </div>

        <div class="price-range">
          <div class="range-labels">
            <span>{{ report.technical.yearLow }}</span>
            <span>一年区间</span>
            <span>{{ report.technical.yearHigh }}</span>
          </div>
          <div class="range-track">
            <i :style="{ left: priceRangePosition }" />
          </div>
          <div class="range-current">当前 {{ report.quote.price }}</div>
        </div>

        <div class="ma-rows">
          <div>
            <span>MA20</span>
            <strong>{{ report.technical.ma20 }}</strong>
            <em :class="report.technical.aboveMa20 ? 'pass' : 'fail'">
              {{ report.technical.aboveMa20 ? '价格在上方' : '价格在下方' }}
            </em>
          </div>
          <div>
            <span>MA200</span>
            <strong>{{ report.technical.ma200 }}</strong>
            <em :class="report.technical.aboveMa200 ? 'pass' : 'fail'">
              {{ report.technical.aboveMa200 ? '价格在上方' : '价格在下方' }}
            </em>
          </div>
        </div>
      </section>

      <section class="report-section valuation-section">
        <header>
          <div>
            <span class="section-kicker">估值</span>
            <h2>DCF 有空间，但关键对标缺失</h2>
          </div>
          <span class="section-note">模型输出</span>
        </header>

        <div class="valuation-grid">
          <div>
            <span>DCF 内在价值</span>
            <strong>{{ report.valuation.dcf.intrinsicPerShare }}</strong>
          </div>
          <div>
            <span>当前价格</span>
            <strong>{{ report.valuation.dcf.currentPrice }}</strong>
          </div>
          <div>
            <span>安全边际</span>
            <strong>{{ report.valuation.dcf.safetyMarginPct }}%</strong>
          </div>
          <div>
            <span>终值占 EV</span>
            <strong>{{ report.valuation.dcf.terminalValuePctOfEv }}%</strong>
          </div>
        </div>

        <div class="dcf-assumptions">
          <span>WACC {{ report.valuation.dcf.wacc }}</span>
          <span>前 5 年增长 {{ report.valuation.dcf.stage1Growth }}</span>
          <span>后 5 年增长 {{ report.valuation.dcf.stage2Growth }}</span>
          <span>终值增长 {{ report.valuation.dcf.terminalGrowth }}</span>
        </div>

        <div class="sensitivity">
          <div v-for="item in report.valuation.dcf.sensitivity" :key="item.terminalGrowth">
            <span>WACC {{ item.wacc }} · g {{ item.terminalGrowth }}</span>
            <strong>{{ item.value }}</strong>
          </div>
        </div>

        <div class="valuation-caveats">
          <div>
            <span>同行样本</span>
            <strong>{{ report.valuation.peerCount }}</strong>
          </div>
          <div>
            <span>PE 5 年分位</span>
            <strong class="text-value">{{ report.valuation.peQuantile }}</strong>
          </div>
          <div>
            <span>PB 5 年分位</span>
            <strong class="text-value">{{ report.valuation.pbQuantile }}</strong>
          </div>
        </div>
      </section>

      <section class="report-section events-section">
        <header>
          <div>
            <span class="section-kicker">近期事件</span>
            <h2>新闻标题，不等同于完整财报口径</h2>
          </div>
        </header>
        <div class="event-list">
          <div v-for="event in report.events" :key="`${event.date}-${event.title}`">
            <time>{{ event.date }}</time>
            <p>{{ event.title }}</p>
          </div>
        </div>
      </section>

      <section class="report-section integrity-section">
        <header>
          <div>
            <span class="section-kicker">数据完整性</span>
            <h2>{{ report.integrity.coveragePct }}% · {{ report.integrity.passedChecks }}/{{ report.integrity.totalChecks }}</h2>
          </div>
          <span class="report-chip ok">无关键缺失</span>
        </header>

        <div class="integrity-block">
          <span>缺失或未完成</span>
          <div class="tag-list">
            <i v-for="item in report.integrity.missingDimensions" :key="item">{{ item }}</i>
          </div>
        </div>
        <div class="integrity-block">
          <span>降级维度</span>
          <div class="tag-list muted-tags">
            <i v-for="item in report.integrity.fallbackDimensions" :key="item">{{ item }}</i>
          </div>
        </div>

        <div class="source-note">
          <span>数据来源</span>
          <code>raw_data.json · dimensions.json · panel.json · _data_gaps.json</code>
          <small>stock-deep-analyzer:uzi / .cache/600519.SH</small>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.stock-report {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 18px 4px 24px;
}

.report-lead,
.report-boundary,
.quote-grid,
.report-summary,
.report-section {
  border: 1px solid var(--cx-gray-150);
  background: var(--cx-gray-0);
}

.report-lead {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 18px;
  padding: 18px 20px;
  border-radius: var(--cx-radius-xl);
}

.report-eyebrow,
.section-kicker {
  display: block;
  color: var(--cx-text-3);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: .1em;
  text-transform: uppercase;
}

.report-identity h1 {
  margin: 6px 0 3px;
  color: var(--cx-text);
  font-size: 24px;
  line-height: 1.15;
}

.report-identity h1 span {
  color: var(--cx-text-3);
  font-family: var(--cx-mono);
  font-size: 14px;
  font-weight: 500;
}

.report-identity p {
  margin: 0;
  color: var(--cx-text-2);
  font-size: 12px;
}

.report-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.report-chip {
  display: inline-flex;
  align-items: center;
  min-height: 24px;
  padding: 0 9px;
  border-radius: var(--cx-radius-full);
  background: var(--cx-gray-75);
  color: var(--cx-text-2);
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
}

.report-chip.warn {
  background: color-mix(in srgb, var(--cx-orange) 11%, transparent);
  color: var(--cx-orange);
}

.report-chip.ok {
  background: color-mix(in srgb, var(--cx-green) 10%, transparent);
  color: var(--cx-green);
}

.report-session-btn {
  height: 30px;
  padding: 0 12px;
  border: 1px solid var(--cx-gray-200);
  border-radius: var(--cx-radius-full);
  background: var(--cx-gray-0);
  color: var(--cx-text);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.report-session-btn:hover {
  background: var(--cx-gray-75);
}

.report-boundary {
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-radius: var(--cx-radius-md);
  color: var(--cx-text-2);
  font-size: 11px;
}

.report-boundary strong {
  color: var(--cx-orange);
}

.report-boundary time {
  color: var(--cx-text-3);
  font-family: var(--cx-mono);
}

.quote-grid {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  overflow: hidden;
  border-radius: var(--cx-radius-md);
}

.quote-cell {
  min-width: 0;
  padding: 13px 14px;
  border-right: 1px solid var(--cx-gray-150);
}

.quote-cell:last-child {
  border-right: 0;
}

.quote-cell > span,
.metric-pairs span,
.technical-stats span,
.valuation-grid span,
.valuation-caveats span {
  display: block;
  color: var(--cx-text-3);
  font-size: 10px;
}

.quote-cell strong,
.metric-pairs strong,
.technical-stats strong,
.valuation-grid strong,
.valuation-caveats strong {
  display: block;
  margin-top: 5px;
  color: var(--cx-text);
  font-family: var(--cx-mono);
  font-size: 18px;
  font-weight: 650;
  line-height: 1.2;
  overflow-wrap: anywhere;
}

.quote-cell small {
  display: block;
  margin-top: 4px;
  color: var(--cx-text-3);
  font-size: 10px;
}

.quote-cell.primary strong {
  color: var(--cx-blue-strong);
}

.quote-cell.conflict {
  background: color-mix(in srgb, var(--cx-orange) 5%, var(--cx-gray-0));
}

.quote-cell.conflict strong {
  color: var(--cx-orange);
}

.text-value {
  font-family: inherit !important;
  font-size: 13px !important;
}

.report-summary {
  display: grid;
  grid-template-columns: minmax(230px, .9fr) 1.3fr;
  gap: 24px;
  padding: 18px 20px;
  border-radius: var(--cx-radius-md);
}

.report-summary h2,
.report-section h2 {
  margin: 6px 0 0;
  color: var(--cx-text);
  font-size: 15px;
  line-height: 1.35;
}

.report-summary ul {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px 18px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.report-summary li {
  position: relative;
  padding-left: 14px;
  color: var(--cx-text-2);
  font-size: 12px;
  line-height: 1.55;
}

.report-summary li::before {
  content: '';
  position: absolute;
  top: .6em;
  left: 0;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--cx-blue);
}

.report-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.report-section {
  min-width: 0;
  padding: 17px 18px 18px;
  border-radius: var(--cx-radius-md);
}

.report-section > header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 14px;
}

.section-note {
  color: var(--cx-text-3);
  font-family: var(--cx-mono);
  font-size: 10px;
  white-space: nowrap;
}

.metric-pairs,
.technical-stats,
.valuation-grid,
.valuation-caveats {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
}

.technical-stats,
.valuation-caveats {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.metric-pairs > div,
.technical-stats > div,
.valuation-grid > div,
.valuation-caveats > div {
  min-width: 0;
  padding: 10px;
  border-radius: var(--cx-radius-sm);
  background: var(--cx-gray-50);
}

.metric-pairs strong,
.technical-stats strong,
.valuation-grid strong,
.valuation-caveats strong {
  font-size: 15px;
}

.chart-head {
  display: flex;
  justify-content: space-between;
  margin: 17px 0 4px;
  color: var(--cx-text-3);
  font-size: 10px;
}

.roe-chart svg {
  display: block;
  width: 100%;
  height: 130px;
  overflow: visible;
}

.axis-line {
  stroke: var(--cx-gray-150);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
}

.roe-line {
  fill: none;
  stroke: var(--cx-blue);
  stroke-width: 2.2;
  vector-effect: non-scaling-stroke;
}

.series-labels {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: 4px;
  text-align: center;
}

.series-labels b,
.series-labels span {
  display: block;
  font-family: var(--cx-mono);
}

.series-labels b {
  color: var(--cx-text);
  font-size: 10px;
}

.series-labels span {
  margin-top: 2px;
  color: var(--cx-text-3);
  font-size: 9px;
}

.growth-bars {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  align-items: end;
  gap: 9px;
  height: 178px;
  padding: 12px 4px 0;
  border-bottom: 1px solid var(--cx-gray-150);
}

.growth-year {
  display: flex;
  min-width: 0;
  height: 100%;
  flex-direction: column;
  justify-content: flex-end;
  text-align: center;
}

.bar-pair {
  display: flex;
  align-items: flex-end;
  justify-content: center;
  gap: 4px;
  height: 120px;
}

.bar-pair i {
  display: block;
  width: 14px;
  min-height: 4px;
  border-radius: 3px 3px 0 0;
}

.revenue-bar {
  background: var(--cx-blue);
}

.profit-bar {
  background: var(--cx-green);
}

.growth-year b {
  margin-top: 7px;
  color: var(--cx-text-2);
  font-family: var(--cx-mono);
  font-size: 10px;
}

.growth-year span,
.growth-year small {
  display: block;
  margin-top: 2px;
  font-family: var(--cx-mono);
  font-size: 9px;
}

.growth-year span {
  color: var(--cx-blue-strong);
}

.growth-year small {
  color: var(--cx-green);
}

.growth-legend {
  display: flex;
  gap: 14px;
  margin-top: 10px;
  color: var(--cx-text-3);
  font-size: 10px;
}

.growth-legend span {
  display: flex;
  align-items: center;
  gap: 5px;
}

.growth-legend i {
  width: 8px;
  height: 8px;
  border-radius: 2px;
}

.legend-revenue {
  background: var(--cx-blue);
}

.legend-profit {
  background: var(--cx-green);
}

.conflict-list {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
  margin-top: 14px;
}

.conflict-list > div {
  min-width: 0;
  padding: 9px 10px;
  border-left: 2px solid var(--cx-orange);
  background: color-mix(in srgb, var(--cx-orange) 5%, var(--cx-gray-0));
}

.conflict-list span,
.conflict-list small {
  display: block;
  color: var(--cx-text-3);
  font-size: 9px;
  line-height: 1.45;
}

.conflict-list strong {
  display: block;
  margin: 3px 0;
  color: var(--cx-text);
  font-family: var(--cx-mono);
  font-size: 14px;
}

.price-range {
  margin: 18px 0;
}

.range-labels,
.range-current {
  display: flex;
  justify-content: space-between;
  color: var(--cx-text-3);
  font-family: var(--cx-mono);
  font-size: 9px;
}

.range-track {
  position: relative;
  height: 8px;
  margin: 7px 0 5px;
  border-radius: var(--cx-radius-full);
  background: linear-gradient(90deg, var(--cx-red), var(--cx-orange), var(--cx-green));
}

.range-track i {
  position: absolute;
  top: -4px;
  width: 3px;
  height: 16px;
  border-radius: 2px;
  background: var(--cx-gray-1000);
  transform: translateX(-50%);
}

.range-current {
  justify-content: flex-end;
  color: var(--cx-text-2);
}

.ma-rows {
  border-top: 1px solid var(--cx-gray-150);
}

.ma-rows > div {
  display: grid;
  grid-template-columns: 52px 1fr auto;
  align-items: center;
  gap: 10px;
  min-height: 38px;
  border-bottom: 1px solid var(--cx-gray-150);
}

.ma-rows span {
  color: var(--cx-text-3);
  font-size: 10px;
}

.ma-rows strong {
  min-width: 0;
  color: var(--cx-text);
  font-family: var(--cx-mono);
  font-size: 11px;
  overflow-wrap: anywhere;
}

.ma-rows em {
  font-size: 10px;
  font-style: normal;
}

.ma-rows em.pass {
  color: var(--cx-green);
}

.ma-rows em.fail {
  color: var(--cx-red);
}

.dcf-assumptions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 12px;
}

.dcf-assumptions span {
  padding: 4px 7px;
  border: 1px solid var(--cx-gray-150);
  border-radius: var(--cx-radius-full);
  color: var(--cx-text-2);
  font-family: var(--cx-mono);
  font-size: 9px;
}

.sensitivity {
  margin-top: 12px;
  border-top: 1px solid var(--cx-gray-150);
}

.sensitivity > div {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 32px;
  border-bottom: 1px solid var(--cx-gray-150);
}

.sensitivity span {
  color: var(--cx-text-3);
  font-size: 10px;
}

.sensitivity strong {
  color: var(--cx-text);
  font-family: var(--cx-mono);
  font-size: 12px;
}

.valuation-caveats {
  margin-top: 12px;
}

.event-list {
  border-top: 1px solid var(--cx-gray-150);
}

.event-list > div {
  display: grid;
  grid-template-columns: 88px 1fr;
  gap: 10px;
  padding: 10px 0;
  border-bottom: 1px solid var(--cx-gray-150);
}

.event-list time {
  color: var(--cx-text-3);
  font-family: var(--cx-mono);
  font-size: 10px;
}

.event-list p {
  margin: 0;
  color: var(--cx-text-2);
  font-size: 11px;
  line-height: 1.55;
}

.integrity-block {
  margin-top: 14px;
}

.integrity-block > span,
.source-note > span {
  display: block;
  margin-bottom: 7px;
  color: var(--cx-text-3);
  font-size: 10px;
}

.tag-list {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}

.tag-list i {
  padding: 4px 7px;
  border-radius: var(--cx-radius-full);
  background: color-mix(in srgb, var(--cx-orange) 8%, transparent);
  color: var(--cx-orange);
  font-size: 9px;
  font-style: normal;
}

.muted-tags i {
  background: var(--cx-gray-75);
  color: var(--cx-text-2);
}

.source-note {
  margin-top: 16px;
  padding-top: 13px;
  border-top: 1px solid var(--cx-gray-150);
}

.source-note code,
.source-note small {
  display: block;
  color: var(--cx-text-2);
  font-family: var(--cx-mono);
  font-size: 10px;
  line-height: 1.6;
  overflow-wrap: anywhere;
}

.source-note small {
  color: var(--cx-text-3);
}

@media (max-width: 1240px) {
  .quote-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .quote-cell:nth-child(3) {
    border-right: 0;
  }

  .quote-cell:nth-child(-n + 3) {
    border-bottom: 1px solid var(--cx-gray-150);
  }
}

@media (max-width: 1050px) {
  .report-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 760px) {
  .report-lead,
  .report-summary {
    grid-template-columns: 1fr;
  }

  .report-lead {
    flex-direction: column;
  }

  .report-actions {
    width: 100%;
    justify-content: space-between;
  }

  .report-boundary {
    grid-template-columns: 1fr;
  }

  .report-summary ul {
    grid-template-columns: 1fr;
  }

  .metric-pairs,
  .valuation-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .conflict-list {
    grid-template-columns: 1fr;
  }
}
</style>
