<script setup lang="ts">
import type { Observe } from '../types'

defineProps<{
  observation: Observe | null
}>()
</script>

<template>
  <section class="panel ridge">
    <div class="panel-head">
      <h2><em class="orange-dot" /> ARBITRAGE OPPORTUNITY RIDGE</h2>
      <span>GROSS / FEES / NET</span>
    </div>
    <div class="ridge-body">
      <div class="ridge-stats">
        <span>VALID FEEDS <b>{{ observation ? '2 / 2' : '0 / 2' }}</b></span>
        <span>SCANNED <b>{{ observation ? '2' : '0' }}</b></span>
        <span>ACCEPTED <b>{{ observation?.report ? 'SEE REPORT' : '—' }}</b></span>
      </div>
      <div class="ridge-visual">
        <div v-for="n in 9" :key="n" class="ridge-line" :style="{ transform: `translateY(${n * 7}px) rotate(${n < 5 ? -5 : 5}deg)`, opacity: `${1 - n * .06}` }" />
        <div class="ridge-label">
          NET AFTER FEES<br />
          <b>{{ observation ? 'CALCULATED' : 'AWAITING FEED' }}</b>
        </div>
      </div>
    </div>
  </section>
</template>
