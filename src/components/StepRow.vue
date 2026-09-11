<script setup lang="ts">
/**
 * One executed step: icon, title, status, and the output behind a disclosure.
 *
 * Shared by the live view and the finished record because they show the same
 * thing — the runtime reports the same items either way, and the record is just
 * the live items kept after the turn ends. Two copies of this markup drifted
 * once already (the record lost its rows entirely); one component makes that
 * impossible.
 *
 * The output is **collapsed by default**: a turn can produce megabytes, and the
 * conversation is for reading, not scrolling. The header carries a one-line
 * preview so the row is informative while closed.
 */
import { computed } from 'vue'
import StepIcon from './StepIcon.vue'
import type { AgentLiveItem } from '../commands'

const props = defineProps<{
  item: AgentLiveItem
  expanded: boolean
}>()

const emit = defineEmits<{ toggle: [] }>()

const ITEM_LABEL: Record<string, string> = {
  reasoning: '推理',
  command: '命令',
  file_change: '文件变更',
  tool_call: '工具',
  message: '回复',
  other: '步骤',
}

const ITEM_STATE: Record<string, { label: string; tone: string }> = {
  running: { label: '进行中', tone: 'warn' },
  completed: { label: '完成', tone: 'ok' },
  failed: { label: '失败', tone: 'bad' },
  declined: { label: '已拒绝', tone: '' },
}

const kindLabel = computed(() => ITEM_LABEL[props.item.kind] ?? props.item.kind)
const state = computed(() => ITEM_STATE[props.item.state] ?? { label: props.item.state, tone: '' })

/** The first non-empty line of the output, for the closed row. */
const preview = computed(() => {
  const first = (props.item.output.split('\n').find(line => line.trim()) ?? '').trim()
  return first.length > 96 ? `${first.slice(0, 96)}…` : first
})

/** The trailing status: what happened, then how long it took or what it said. */
const trailing = computed(() => props.item.duration_ms !== null ? `${props.item.duration_ms} ms` : preview.value)
</script>

<template>
  <div class="codex-tool">
    <button
      class="codex-tool-head"
      type="button"
      :aria-expanded="expanded"
      :aria-label="`${kindLabel}：${item.title}（${state.label}）`"
      @click="emit('toggle')"
    >
      <svg
        class="codex-caret"
        :class="{ open: expanded }"
        viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M9 5l7 7-7 7" />
      </svg>
      <span class="codex-step-icon" :class="state.tone">
        <StepIcon :kind="item.kind" :title="item.title" :label="kindLabel" />
      </span>
      <span class="codex-tool-name">{{ item.title }}</span>
      <span class="codex-tool-status">
        <template v-if="item.state === 'running'">进行中…</template>
        <template v-else-if="item.exit_code !== null">退出码 {{ item.exit_code }}</template>
        <template v-else>{{ state.label }}</template>
        <template v-if="trailing"> · {{ trailing }}</template>
      </span>
    </button>
    <pre v-if="expanded && item.output" class="codex-tool-body">{{ item.output }}</pre>
  </div>
</template>
