<script setup lang="ts">
import { ElCard, ElTable, ElTableColumn } from 'element-plus'
import type { Account, Observe } from '../types'

defineProps<{
  accounts: Account[]
  observation: Observe | null
  report: unknown
}>()
</script>

<template>
  <section v-if="accounts.length || observation || report" class="detail-panel">
    <ElCard v-if="accounts.length" shadow="never">
      <template #header>ACCOUNT FACTS</template>
      <ElTable :data="accounts" stripe>
        <ElTableColumn prop="venue" label="VENUE" width="120" />
        <ElTableColumn label="FEE SOURCE">
          <template #default="{ row }">
            {{ row.fee.source }} / {{ row.fee.buy_taker_rate }} / {{ row.fee.sell_taker_rate }}
          </template>
        </ElTableColumn>
        <ElTableColumn label="REJECTIONS">
          <template #default="{ row }">
            {{ row.rejection_reasons.join(' · ') || 'none' }}
          </template>
        </ElTableColumn>
      </ElTable>
    </ElCard>
    <pre v-if="observation">{{ JSON.stringify(observation.report, null, 2) }}</pre>
    <pre v-if="report">{{ JSON.stringify(report, null, 2) }}</pre>
  </section>
</template>
