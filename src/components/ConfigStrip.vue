<script setup lang="ts">
import { computed } from 'vue'
import type { PairSummary } from '../types'
import TermHint from './TermHint.vue'

const props = defineProps<{
  pairs: PairSummary[]
  orderbookDepth: number | null
  modelValue: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

/** 展示币种跟随聚焦选择：选中时显示该币种，未选（全部）时回退到列表首项。 */
const shown = computed(() =>
  props.pairs.find(p => p.symbol === props.modelValue) ?? props.pairs[0] ?? null,
)
const extraCount = computed(() => Math.max(0, props.pairs.length - 1))
</script>

<template>
  <section class="config-strip">
    <span class="eyebrow">市场 / 配置</span>
    <b>{{ shown?.symbol || '—' }}</b>
    <span v-if="shown">{{ shown.quantity || '—' }} {{ shown.base_asset || '' }}</span>
    <span v-if="extraCount > 0" class="pairs-extra">+{{ extraCount }} 个币种</span>
    <span class="muted"><TermHint term="深度" /> {{ orderbookDepth || '—' }}</span>

    <ElSelect
      :model-value="modelValue"
      size="small"
      clearable
      placeholder="全部币种"
      class="symbol-switch"
      title="聚焦币种：概览 / 市场 / 控制台按所选币种显示数据"
      @update:model-value="emit('update:modelValue', String($event ?? ''))"
    >
      <ElOption
        v-for="p in pairs"
        :key="p.symbol"
        :label="p.symbol"
        :value="p.symbol"
      >
        <span style="float: left">{{ p.symbol }}</span>
        <span style="float: right; color: var(--text-3); font-size: 12px">{{ p.quantity }} {{ p.base_asset }}</span>
      </ElOption>
      <template #prefix><TermHint term="聚焦币种" /></template>
    </ElSelect>
  </section>
</template>

<style scoped>
.pairs-extra {
  color: var(--accent);
  font-size: var(--fs-10);
  letter-spacing: .06em;
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  border-radius: 999px;
  padding: 1px 8px;
}
.symbol-switch {
  width: 160px;
  margin-left: 8px;
  text-align: left;
}
</style>