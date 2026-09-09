<script setup lang="ts">
import { ElInput } from 'element-plus'

defineProps<{
  symbol: string | null
  quantity: string | null
  baseAsset: string | null
  orderbookDepth: number | null
}>()

const configPath = defineModel<string>('configPath', { default: '' })
</script>

<template>
  <section class="config-strip">
    <span class="eyebrow">MARKET / CONFIG</span>
    <b>{{ symbol || '—' }}</b>
    <span>{{ quantity || '—' }} {{ baseAsset || '' }}</span>
    <span class="muted">DEPTH {{ orderbookDepth || '—' }}</span>
    <ElInput v-model="configPath" size="small" placeholder="config/observer.toml" class="path-input" />
  </section>
</template>
