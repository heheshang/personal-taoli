<script setup lang="ts">
import { computed } from 'vue'
import type { PairSummary } from '../types'
import TermHint from './TermHint.vue'

const props = defineProps<{
  pairs: PairSummary[]
  orderbookDepth: number | null
}>()

const first = computed(() => props.pairs[0] ?? null)
const extraCount = computed(() => Math.max(0, props.pairs.length - 1))
</script>

<template>
  <section class="config-strip">
    <span class="eyebrow">市场 / 配置</span>
    <b>{{ first?.symbol || '—' }}</b>
    <span v-if="first">{{ first.quantity || '—' }} {{ first.base_asset || '' }}</span>
    <span v-if="extraCount > 0" class="pairs-extra">+{{ extraCount }} 个币种</span>
    <span class="muted"><TermHint term="深度" /> {{ orderbookDepth || '—' }}</span>
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
</style>
