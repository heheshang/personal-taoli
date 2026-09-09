<script setup lang="ts">
defineProps<{
  orderbookDepth: number | null
}>()
</script>

<template>
  <article class="panel lattice">
    <div class="panel-head">
      <h2><em class="blue-dot" /> 5D STRATEGY LATTICE</h2>
      <span>RISK / COST / LATENCY</span>
    </div>
    <div class="lattice-body">
      <div class="cube">
        <i /><i /><i /><i /><i /><i />
      </div>
      <div class="lattice-values">
        <span>DEPTH <b>{{ orderbookDepth || '—' }}</b></span>
        <span>RISK BUFFER <b>CONFIGURED</b></span>
        <span>EXECUTION <b>PAPER ONLY</b></span>
      </div>
    </div>
  </article>
</template>
