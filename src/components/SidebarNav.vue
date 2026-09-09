<script setup lang="ts">
defineProps<{
  active: string
  continuousRunning: boolean
}>()

const emit = defineEmits<{
  navigate: [page: string]
}>()

const pages = [
  { id: 'overview', label: '概览', icon: '◉' },
  { id: 'market', label: '市场', icon: '◎' },
  { id: 'control', label: '控制台', icon: '⚙' },
  { id: 'reports', label: '报告', icon: '▤' },
  { id: 'settings', label: '设置', icon: '⛭' },
]
</script>

<template>
  <nav class="sidebar">
    <div class="sidebar-brand">
      <span class="brand-icon">↗</span>
      <span class="brand-text">TAOLI</span>
    </div>
    <div class="nav-links">
      <button
        v-for="page in pages"
        :key="page.id"
        class="nav-item"
        :class="{ active: active === page.id }"
        @click="emit('navigate', page.id)"
      >
        <span class="nav-icon">{{ page.icon }}</span>
        <span class="nav-label">{{ page.label }}</span>
        <i v-if="page.id === 'control' && continuousRunning" class="nav-dot pulse" />
      </button>
    </div>
    <div class="sidebar-footer">
      <div class="status-dot" :class="continuousRunning ? 'on' : 'off'" />
      <span class="status-text">{{ continuousRunning ? '运行中' : '待机' }}</span>
    </div>
  </nav>
</template>

<style scoped>
.sidebar {
  width: 72px;
  min-height: 100vh;
  background: #1a2b22;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 16px 0;
  position: fixed;
  left: 0;
  top: 0;
  z-index: 100;
  border-right: 1px solid #2a3d32;
}
.sidebar-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  margin-bottom: 32px;
}
.brand-icon {
  width: 36px;
  height: 36px;
  display: grid;
  place-items: center;
  border-radius: 10px;
  color: #1a2b22;
  background: #c8ef3b;
  font-size: 20px;
  font-weight: 900;
  transform: rotate(-10deg);
}
.brand-text {
  color: #7b9b8a;
  font-size: 9px;
  letter-spacing: .12em;
  font-weight: 700;
}
.nav-links {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
}
.nav-item {
  width: 52px;
  height: 52px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  border: none;
  background: transparent;
  border-radius: 10px;
  cursor: pointer;
  transition: all .15s;
  position: relative;
  color: #5a7a6a;
}
.nav-item:hover {
  background: #243830;
  color: #a0c4b0;
}
.nav-item.active {
  background: #2a4a3a;
  color: #c8ef3b;
}
.nav-icon {
  font-size: 16px;
  line-height: 1;
}
.nav-label {
  font-size: 8px;
  letter-spacing: .04em;
  font-weight: 600;
}
.nav-dot {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #26a269;
}
.sidebar-footer {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding-top: 16px;
  border-top: 1px solid #2a3d32;
}
.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.status-dot.on {
  background: #26a269;
  box-shadow: 0 0 0 0 rgba(38, 162, 105, .55);
  animation: pulse 1.8s infinite;
}
.status-dot.off {
  background: #5a6a60;
}
.status-text {
  color: #5a7a6a;
  font-size: 7px;
  letter-spacing: .06em;
}
@keyframes pulse {
  70% { box-shadow: 0 0 0 6px rgba(38, 162, 105, 0); }
  100% { box-shadow: 0 0 0 0 rgba(38, 162, 105, 0); }
}
</style>
