<script setup lang="ts">
import { ElButton } from 'element-plus'

defineProps<{
  version: string | null
  liveLabel: string
  canOperate: boolean
}>()

const emit = defineEmits<{
  refresh: []
}>()
</script>

<template>
  <header class="topbar">
    <div class="brand">
      <span class="brand-mark">↗</span>
      <div>
        <strong>TAOLI OBSERVER</strong>
        <small>PERSONAL ARBITRAGE DESK</small>
      </div>
    </div>
    <div class="top-actions">
      <span class="clock">{{ version ? `v${version}` : 'booting' }}</span>
      <span class="live-pill"><i />{{ liveLabel }}</span>
      <ElButton text class="refresh" :disabled="!canOperate" @click="emit('refresh')">↻</ElButton>
    </div>
  </header>
</template>
