<script setup lang="ts">
import { computed } from 'vue'
import { GLOSSARY } from '../glossary'

const props = defineProps<{
  /** 词条键；优先级高于 slot 文本。 */
  term: string
}>()

const hint = computed(() => GLOSSARY[props.term] ?? '')
</script>

<template>
  <el-tooltip v-if="hint" :content="hint" placement="top" :show-after="80">
    <span class="term-hint"><slot>{{ term }}</slot></span>
  </el-tooltip>
  <span v-else class="term-plain"><slot>{{ term }}</slot></span>
</template>

<style scoped>
.term-hint {
  border-bottom: 1px dashed var(--text-3);
  cursor: help;
}
</style>