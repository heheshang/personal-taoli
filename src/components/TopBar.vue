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
        <strong>TAOLI 观测台</strong>
        <small>个人套利控制台</small>
      </div>
    </div>
    <div class="top-actions">
      <span class="clock">{{ version ? `v${version}` : '启动中' }}</span>
      <span class="live-pill"><i />{{ liveLabel }}</span>
      <ElButton text class="refresh" :disabled="!canOperate" @click="emit('refresh')">↻</ElButton>
    </div>
  </header>
</template>
