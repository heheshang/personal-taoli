<script setup lang="ts">
defineProps<{
  symbol: string | null
  version: string | null
}>()
</script>

<template>
  <footer>
    <span>TAOLI 观测台 · {{ symbol || '市场' }}</span>
    <span>安全模式 / 无订单路由 / {{ version || '—' }}</span>
  </footer>
</template>
