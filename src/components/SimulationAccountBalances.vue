<script setup lang="ts">
import { computed, ref } from 'vue'
import { ElPagination } from 'element-plus'
import type { SimulationOverview } from '../types'
import { trimDecimal } from '../format'

const props = defineProps<{
  overview: SimulationOverview | null
}>()

/** 账户现值：paper_balances 当前余额快照（实时视图）。 */
const currentBalances = computed(() => props.overview?.account_balances ?? [])

/** 每页 20 行（与机会历史分页规格一致）。 */
const balancePage = ref(1)
const BALANCE_PAGE_SIZE = 20
const balancePageRows = computed(() =>
  currentBalances.value.slice(
    (balancePage.value - 1) * BALANCE_PAGE_SIZE,
    balancePage.value * BALANCE_PAGE_SIZE,
  ),
)
</script>

<template>
  <section class="panel balance-now">
    <div class="panel-head">
      <h2><em class="teal-dot" /> 账户现值</h2>
      <span class="muted">paper_balances 实时快照 · 每页 {{ BALANCE_PAGE_SIZE }}</span>
    </div>
    <div v-if="!currentBalances.length" class="empty-state">暂无现值数据（先运行烟测）</div>
    <template v-else>
      <table class="flat-table">
        <thead>
          <tr>
            <th>账户</th>
            <th>交易场所</th>
            <th>资产</th>
            <th class="num">总额</th>
            <th class="num">可用</th>
            <th class="num">本地预留</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(b, i) in balancePageRows" :key="i">
            <td class="mono">{{ b.account_id }}</td>
            <td>{{ b.venue }}</td>
            <td>{{ b.asset }}</td>
            <td class="num">{{ trimDecimal(b.observed_total) }}</td>
            <td class="num">{{ trimDecimal(b.observed_free) }}</td>
            <td class="num">{{ trimDecimal(b.local_reserved) }}</td>
          </tr>
        </tbody>
      </table>
      <div class="pager">
        <ElPagination
          v-model:current-page="balancePage"
          layout="prev, pager, next, total"
          :page-size="BALANCE_PAGE_SIZE"
          :total="currentBalances.length"
          background
          small
          hide-on-single-page
        />
      </div>
    </template>
  </section>
</template>

<style scoped>
.flat-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-11);
}
.flat-table th {
  text-align: left;
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: 0.05em;
  padding: 5px 6px;
  border-bottom: 1px solid var(--border);
}
.flat-table td {
  padding: 5px 6px;
  border-bottom: 1px solid var(--border);
  color: var(--text-2);
}
.flat-table td.num,
.flat-table th.num {
  text-align: right;
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.pager {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 10px 2px 4px;
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
  border-top: 1px solid var(--border);
}
</style>