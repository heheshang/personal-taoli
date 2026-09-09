<script setup lang="ts">
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
    <span class="muted">深度 {{ orderbookDepth || '—' }}</span>
  </section>
</template>
