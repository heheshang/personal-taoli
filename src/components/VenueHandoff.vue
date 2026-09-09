<script setup lang="ts">
import type { Account } from '../types'

defineProps<{
  accounts: Account[]
}>()
</script>

<template>
  <article class="panel handoff">
    <div class="panel-head">
      <h2><em class="teal-dot" /> VENUE HANDOFF</h2>
      <span>FEE / PERMISSION</span>
    </div>
    <div class="handoff-body">
      <div class="radar">
        <span class="radar-ring r1" />
        <span class="radar-ring r2" />
        <span class="radar-ring r3" />
        <i class="radar-line" />
        <b>2 VENUES</b>
      </div>
      <div class="venue-list">
        <div v-for="venue in ['BINANCE', 'BYBIT']" :key="venue">
          <b>{{ venue }}</b>
          <span>{{ accounts.length ? 'REVIEWED' : 'NOT LOADED' }}</span>
          <strong>{{ accounts.find(a => a.venue.toUpperCase() === venue)?.fee?.source || '—' }}</strong>
        </div>
      </div>
    </div>
  </article>
</template>
