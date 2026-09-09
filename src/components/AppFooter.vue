<script setup lang="ts">
defineProps<{
  symbol: string | null
  version: string | null
}>()
</script>

<template>
  <footer>
    <span>TAOLI OBSERVER · {{ symbol || 'MARKET' }}</span>
    <span>SAFE MODE / NO ORDER ROUTES / {{ version || '—' }}</span>
  </footer>
</template>
