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
      <span class="eyebrow">OBSERVATION MODE</span>
      <strong>READ ONLY</strong>
      <small>NO REAL ORDERS ENABLED</small>
    </article>
    <article class="metric-card accent">
      <span class="eyebrow">EXPECTED NET</span>
      <strong>{{ observation ? 'SEE REPORT' : '—' }}</strong>
      <small>AFTER FEES / RISK BUFFER</small>
    </article>
    <article class="metric-card">
      <span class="eyebrow">FEED STATUS</span>
      <strong>{{ observation ? feedState : 'SYNCING' }}</strong>
      <small>BINANCE · BYBIT</small>
    </article>
    <article class="metric-card gate">
      <span class="eyebrow">ADMISSION GATE</span>
      <strong>{{ accounts.length ? 'REVIEWED' : 'PENDING' }}</strong>
      <div class="gate-bars">
        <i v-for="n in 8" :key="n" :class="{ on: accounts.length > 0 && n < 6 }" />
      </div>
      <small>{{ accounts.length ? 'CHECK REJECTION REASONS' : 'LOAD ACCOUNT STATUS' }}</small>
    </article>
  </section>
</template>
