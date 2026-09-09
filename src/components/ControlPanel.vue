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
      <span class="eyebrow">控制台</span>
      <b>只读操作</b>
    </div>
    <div class="control-actions">
      <ElButton :disabled="!canOperate" @click="emit('loadAccounts')">账户状态</ElButton>
      <ElButton type="primary" :disabled="!canOperate" @click="emit('observe')">执行观测</ElButton>
      <ElButton :disabled="!canOperate || !archivePath" @click="emit('replay')">回放归档</ElButton>
      <ElButton v-for="kind in ['B01', 'B02', 'B03']" :key="kind" :disabled="!canOperate" @click="emit('paper', kind)">PAPER {{ kind }}</ElButton>
      <ElButton v-for="kind in ['ACCOUNTING', 'RECONCILIATION', 'CONTROL']" :key="kind" :disabled="!canOperate" @click="emit('accountingControl', kind)">{{ kind === 'ACCOUNTING' ? '账务' : kind === 'RECONCILIATION' ? '对账' : '控制' }}</ElButton>
      <ElButton :disabled="!canOperate || continuous?.running" @click="emit('startContinuous')">启动连续</ElButton>
      <ElButton :disabled="!canOperate || !continuous?.running" @click="emit('stopContinuous')">停止连续</ElButton>
      <ElButton :disabled="!canOperate" @click="emit('reconnectSmoke')">重连验证</ElButton>
    </div>
    <div v-if="continuous" class="control-session">
      <template v-if="continuous.running">
        <i class="pulse" />
        <span>连续观测 <b>运行中</b> · {{ continuous.archive_path || '默认归档' }} · 自 {{ fmtTime(continuous.started_at_ms) }}</span>
      </template>
      <template v-else>
        <span>连续观测 <b>已停止</b>{{ continuous.error ? ' — ' + continuous.error : '' }}</span>
      </template>
    </div>
    <small class="control-note">回放需要已有归档文件；PAPER 和账务控制为数据库验证操作。</small>
  </section>
</template>
