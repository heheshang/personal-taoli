<script setup lang="ts">
import TermHint from './TermHint.vue'

defineProps<{
  symbol: string | null
  quantity: string | null
  baseAsset: string | null
  orderbookDepth: number | null
}>()
</script>

<template>
  <section class="config-strip">
    <span class="eyebrow">市场 / 配置</span>
    <b>{{ symbol || '—' }}</b>
    <span>{{ quantity || '—' }} {{ baseAsset || '' }}</span>
    <span class="muted"><TermHint term="深度" /> {{ orderbookDepth || '—' }}</span>
  </section>
</template>
