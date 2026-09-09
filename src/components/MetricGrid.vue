<script setup lang="ts">
import type { Account, Observe } from '../types'

defineProps<{
  observation: Observe | null
  accounts: Account[]
  feedState: string
}>()
</script>

<template>
  <section class="metric-grid">
    <article class="metric-card">
      <span class="eyebrow">观测模式</span>
      <strong>只读</strong>
      <small>未启用真实订单</small>
    </article>
    <article class="metric-card accent">
      <span class="eyebrow">预期净收益</span>
      <strong>{{ observation ? '查看报告' : '—' }}</strong>
      <small>扣除手续费 / 风险缓冲</small>
    </article>
    <article class="metric-card">
      <span class="eyebrow">数据源状态</span>
      <strong>{{ observation ? feedState : '同步中' }}</strong>
      <small>BINANCE · BYBIT</small>
    </article>
    <article class="metric-card gate">
      <span class="eyebrow">准入检查</span>
      <strong>{{ accounts.length ? '已审核' : '待加载' }}</strong>
      <div class="gate-bars">
        <i v-for="n in 8" :key="n" :class="{ on: accounts.length > 0 && n < 6 }" />
      </div>
      <small>{{ accounts.length ? '查看拒绝原因' : '加载账户状态' }}</small>
    </article>
  </section>
</template>
