<script setup lang="ts">
import type { Observe } from '../types'
import TermHint from './TermHint.vue'

defineProps<{
  observation: Observe | null
}>()
</script>

<template>
  <section class="panel ridge">
    <div class="panel-head">
      <h2><em class="orange-dot" /> 套利机会山脊图</h2>
      <span>毛利 / 手续费 / 净收益</span>
    </div>
    <div class="ridge-body">
      <div class="ridge-stats">
        <span><TermHint term="有效数据源" /> <b>{{ observation ? '2 / 2' : '0 / 2' }}</b></span>
        <span><TermHint term="已扫描" /> <b>{{ observation ? '2' : '0' }}</b></span>
        <span><TermHint term="已准入" /> <b>{{ observation?.report ? '查看报告' : '—' }}</b></span>
      </div>
      <div class="ridge-visual">
        <div v-for="n in 9" :key="n" class="ridge-line" :style="{ transform: `translateY(${n * 7}px) rotate(${n < 5 ? -5 : 5}deg)`, opacity: `${1 - n * .06}` }" />
        <div class="ridge-label">
          扣除手续费后净收益<br />
          <b>{{ observation ? '已计算' : '等待数据' }}</b>
        </div>
      </div>
    </div>
  </section>
</template>
