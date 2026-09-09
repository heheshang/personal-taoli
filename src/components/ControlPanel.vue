<script setup lang="ts">
import { ElButton } from 'element-plus'
import type { Continuous } from '../types'

defineProps<{
  canOperate: boolean
  archivePath: string
  continuous: Continuous | null
  fmtTime: (ms: number | null) => string
}>()

const emit = defineEmits<{
  loadAccounts: []
  observe: []
  replay: []
  paper: [kind: string]
  accountingControl: [kind: string]
  startContinuous: []
  stopContinuous: []
  reconnectSmoke: []
}>()
</script>

<template>
  <section class="control-panel">
    <div class="control-title">
      <span class="eyebrow">CONTROL DECK</span>
      <b>READ-ONLY ACTIONS</b>
    </div>
    <div class="control-actions">
      <ElButton :disabled="!canOperate" @click="emit('loadAccounts')">ACCOUNT STATUS</ElButton>
      <ElButton type="primary" :disabled="!canOperate" @click="emit('observe')">OBSERVE ONCE</ElButton>
      <ElButton :disabled="!canOperate || !archivePath" @click="emit('replay')">REPLAY</ElButton>
      <ElButton v-for="kind in ['B01', 'B02', 'B03']" :key="kind" :disabled="!canOperate" @click="emit('paper', kind)">PAPER {{ kind }}</ElButton>
      <ElButton v-for="kind in ['ACCOUNTING', 'RECONCILIATION', 'CONTROL']" :key="kind" :disabled="!canOperate" @click="emit('accountingControl', kind)">ACCOUNTING {{ kind }}</ElButton>
      <ElButton :disabled="!canOperate || continuous?.running" @click="emit('startContinuous')">CONTINUOUS START</ElButton>
      <ElButton :disabled="!canOperate || !continuous?.running" @click="emit('stopContinuous')">CONTINUOUS STOP</ElButton>
      <ElButton :disabled="!canOperate" @click="emit('reconnectSmoke')">RECONNECT SMOKE</ElButton>
    </div>
    <div v-if="continuous" class="control-session">
      <template v-if="continuous.running">
        <i class="pulse" />
        <span>CONTINUOUS <b>RUNNING</b> · {{ continuous.archive_path || 'archive from config' }} · since {{ fmtTime(continuous.started_at_ms) }}</span>
      </template>
      <template v-else>
        <span>CONTINUOUS STOPPED{{ continuous.error ? ' — ' + continuous.error : '' }}</span>
      </template>
    </div>
    <small class="control-note">REPLAY requires an existing archive; PAPER and accounting controls are database-backed smoke operations.</small>
  </section>
</template>
