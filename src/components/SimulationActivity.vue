<script setup lang="ts">
import { computed, ref } from 'vue'
import { ElPagination } from 'element-plus'
import type { ActivityEvent, SimulationOverview } from '../types'

const props = defineProps<{
  overview: SimulationOverview | null
}>()

const emit = defineEmits<{
  openRun: [runId: string]
}>()

const events = computed(() => props.overview?.activity ?? [])

/** 活动日志分页：每页 20 条（与机会历史分页规格一致）。 */
const activityPage = ref(1)
const ACTIVITY_PAGE_SIZE = 20
const pageRows = computed(() =>
  events.value.slice(
    (activityPage.value - 1) * ACTIVITY_PAGE_SIZE,
    activityPage.value * ACTIVITY_PAGE_SIZE,
  ),
)

/** 事件类型降噪：剔除每日重复的高频审计噪音，其余按类型映射中文短名。 */
const EVENT_LABELS: Record<string, string> = {
  PlanReserved: '资金预留',
  PlanCommitted: '计划提交',
  PlanCancelled: '计划撤销',
  TradeFilled: '成交落账',
}

function label(e: ActivityEvent): string {
  return EVENT_LABELS[e.event_type] ?? e.event_type
}

function fmtTime(ms: number): string {
  const d = new Date(ms)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
}

function fmtDate(ms: number): string {
  const d = new Date(ms)
  return `${d.getMonth() + 1}/${d.getDate()}`
}

function colorClass(e: ActivityEvent): string {
  if (e.event_type.includes('Reject') || e.event_type.includes('Cancel')) return 'pink'
  if (e.event_type.includes('Fill') || e.event_type.includes('Settle')) return 'green'
  if (e.event_type.includes('Reserv')) return 'blue'
  return 'yellow'
}

function fmtPayload(p: Record<string, unknown>): string {
  try {
    const s = JSON.stringify(p)
    return s.length > 60 ? s.slice(0, 57) + '…' : s
  } catch {
    return ''
  }
}
</script>

<template>
  <section class="panel">
    <div class="panel-head">
      <h2><em class="purple-dot" /> 活动日志</h2>
      <span class="muted">最近 {{ events.length }} 条审计事件 · 不可变表</span>
    </div>
    <div v-if="!events.length" class="empty-state">暂无活动（先运行烟测）</div>
    <div v-else class="log-list sim-log">
      <div v-for="(e, i) in pageRows" :key="i" class="log-row">
        <time>{{ fmtDate(e.occurred_at_ms) }} {{ fmtTime(e.occurred_at_ms) }}</time>
        <b :class="colorClass(e)">{{ label(e) }}</b>
        <span class="log-agg">{{ e.aggregate_id }}</span>
        <span class="log-payload">{{ fmtPayload(e.payload) }}</span>
        <a
          v-if="e.run_id"
          class="log-run"
          href="#"
          @click.prevent="emit('openRun', e.run_id!)"
        >{{ e.run_id }}↗</a>
        <span v-else class="log-run muted">无归属</span>
      </div>
      <div class="pager">
        <ElPagination
          v-model:current-page="activityPage"
          layout="prev, pager, next, total"
          :page-size="ACTIVITY_PAGE_SIZE"
          :total="events.length"
          background
          small
          hide-on-single-page
        />
      </div>
    </div>
  </section>
</template>

<style scoped>
.sim-log {
  display: flex;
  flex-direction: column;
}
.pager {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 10px 2px 4px;
}
.log-row {
  display: grid;
  grid-template-columns: 92px 60px minmax(110px, 1fr) auto 140px;
  gap: 8px;
  align-items: baseline;
  padding: 7px 0;
  border-bottom: 1px solid var(--border);
  font-size: var(--fs-11);
}
.log-row:last-child {
  border-bottom: 0;
}
.log-agg {
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--fs-10);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.log-payload {
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--fs-10);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.log-run {
  color: var(--info);
  font-family: var(--font-mono);
  font-size: var(--fs-10);
  text-align: right;
  white-space: nowrap;
}
.muted {
  color: var(--text-3);
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
  border-top: 1px solid var(--border);
}
</style>