<script setup lang="ts">
import type { Account } from '../types'
import TermHint from './TermHint.vue'

defineProps<{
  accounts: Account[]
}>()
</script>

<template>
  <article class="panel handoff">
    <div class="panel-head">
      <h2><em class="teal-dot" /> 交易所交接</h2>
      <span><TermHint term="费率" /> / <TermHint term="权限" /></span>
    </div>
    <div class="handoff-body">
      <div class="radar">
        <span class="radar-ring r1" />
        <span class="radar-ring r2" />
        <span class="radar-ring r3" />
        <i class="radar-line" />
        <b><TermHint term="交易所" /> 2</b>
      </div>
      <div class="venue-list">
        <div v-for="venue in ['BINANCE', 'BYBIT']" :key="venue">
          <b>{{ venue }}</b>
          <span>{{ accounts.length ? '已审核' : '未加载' }}</span>
          <strong>{{ accounts.find(a => a.venue.toUpperCase() === venue)?.fee?.source || '—' }}</strong>
        </div>
      </div>
    </div>
  </article>
</template>
