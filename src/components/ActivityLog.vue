<script setup lang="ts">
defineProps<{
  feedState: string
  eventCount: string
}>()
</script>

<template>
  <article class="panel activity">
    <div class="panel-head">
      <h2><em class="pink-dot" /> ACTIVITY LOG</h2>
      <span>{{ eventCount }}</span>
    </div>
    <div class="log-list">
      <div>
        <time>NOW</time>
        <b class="yellow">SYSTEM</b>
        <span>dashboard ready · safe mode enforced</span>
      </div>
      <div>
        <time>—</time>
        <b class="blue">FEED</b>
        <span>{{ feedState }}</span>
      </div>
      <div>
        <time>—</time>
        <b class="pink">PAPER</b>
        <span>external order calls: 0</span>
      </div>
      <div>
        <time>—</time>
        <b class="green">AUDIT</b>
        <span>commands remain read-only</span>
      </div>
      <div>
        <time>—</time>
        <b class="gray">NOTE</b>
        <span>run an action below to populate facts</span>
      </div>
    </div>
  </article>
</template>
