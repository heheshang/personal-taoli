<script setup lang="ts">
defineProps<{
  feedState: string
  eventCount: string
}>()
</script>

<template>
  <article class="panel activity">
    <div class="panel-head">
      <h2><em class="pink-dot" /> 活动日志</h2>
      <span>{{ eventCount }}</span>
    </div>
    <div class="log-list">
      <div>
        <time>现在</time>
        <b class="yellow">系统</b>
        <span>仪表盘就绪 · 安全模式已启用</span>
      </div>
      <div>
        <time>—</time>
        <b class="blue">数据源</b>
        <span>{{ feedState }}</span>
      </div>
      <div>
        <time>—</time>
        <b class="pink">PAPER</b>
        <span>外部订单调用: 0</span>
      </div>
      <div>
        <time>—</time>
        <b class="green">审计</b>
        <span>所有命令保持只读</span>
      </div>
      <div>
        <time>—</time>
        <b class="gray">备注</b>
        <span>执行下方操作以生成数据</span>
      </div>
    </div>
  </article>
</template>
