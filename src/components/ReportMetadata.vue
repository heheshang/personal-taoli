<script setup lang="ts">
/**
 * Shows what the model returned when it did not match the report schema.
 *
 * The whole point is that the shape is **unknown**: the model invented the field
 * names, so this cannot lay them out the way `ReportView` lays out a conforming
 * report. It therefore makes one distinction and only one:
 *
 * * a **scalar** gets a label/value row, which is readable;
 * * anything **structured** (an object or array) goes into a collapsed block of
 *   pretty-printed JSON, because there is no honest way to guess what it means —
 *   inventing a layout for `业务结构_2026H1` would present a guess as a design.
 *
 * Field names are shown exactly as received, including ones that look like
 * typos or near-misses (`stock_code` where the schema says `ticker`): that
 * difference is the diagnosis, so renaming or normalising here would hide it.
 */
import { computed } from 'vue'

const props = defineProps<{
  value: Record<string, unknown>
  /** Why the schema was not satisfied, as the backend reported it. */
  reason?: string | null
}>()

/** Rows for the scalar fields, in the order the model produced them. */
const scalars = computed(() =>
  Object.entries(props.value)
    .filter(([, value]) => value === null || typeof value !== 'object')
    .map(([key, value]) => ({ key, text: value === null ? 'null' : String(value) })),
)

/** The structured fields, kept whole. */
const structured = computed(() =>
  Object.entries(props.value).filter(([, value]) => value !== null && typeof value === 'object'),
)
</script>

<template>
  <section class="report report-raw">
    <header class="report-head">
      <h3 class="report-title">模型返回的结构</h3>
      <div class="report-badges">
        <span class="codex-chip warn"><i class="codex-dot" />不符合结构定义</span>
        <span class="codex-chip">原样展示</span>
      </div>
    </header>

    <!-- 说明为什么没按报告渲染，以及这里的字段名是模型自己起的 -->
    <p class="report-raw-note">
      本次未按结构定义输出，因此不按报告版式渲染；下面是模型实际返回的内容，字段名原样保留。
      <template v-if="reason">{{ reason }}</template>
    </p>

    <div v-if="scalars.length" class="report-panel">
      <h4>字段</h4>
      <table class="report-table">
        <tbody>
          <tr v-for="row in scalars" :key="row.key">
            <th class="report-raw-key">{{ row.key }}</th>
            <td>{{ row.text }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- 结构未知，故不猜版式：原样折叠展示。 -->
    <details v-for="[key, value] in structured" :key="key" class="report-raw-details">
      <summary>
        <code>{{ key }}</code>
        <span class="report-raw-kind">{{ Array.isArray(value) ? `数组 ${value.length} 项` : '对象' }}</span>
      </summary>
      <pre class="codex-tool-body">{{ JSON.stringify(value, null, 2) }}</pre>
    </details>

    <p v-if="!scalars.length && !structured.length" class="report-raw-note">
      返回的是一个空对象。
    </p>
  </section>
</template>
