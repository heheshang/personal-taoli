<script setup lang="ts">
defineProps<{
  orderbookDepth: number | null
}>()
</script>

<template>
  <article class="panel lattice">
    <div class="panel-head">
      <h2><em class="blue-dot" /> 五维策略晶格</h2>
      <span>风险 / 成本 / 延迟</span>
    </div>
    <div class="lattice-body">
      <div class="cube">
        <i /><i /><i /><i /><i /><i />
      </div>
      <div class="lattice-values">
        <span>深度 <b>{{ orderbookDepth || '—' }}</b></span>
        <span>风险缓冲 <b>已配置</b></span>
        <span>执行模式 <b>仅 PAPER</b></span>
      </div>
    </div>
  </article>
</template>
