<script setup lang="ts">
defineProps<{
  active: string
  continuousRunning: boolean
}>()

const emit = defineEmits<{
  navigate: [page: string]
}>()

const pages = [
  { id: 'overview', label: '概览' },
  { id: 'market', label: '市场' },
  { id: 'control', label: '控制台' },
  { id: 'simulation', label: '模拟套利' },
  { id: 'agent', label: '个股分析' },
  { id: 'settings', label: '设置' },
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
        <svg
          v-if="page.id === 'overview'"
          class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        >
          <rect x="3" y="3" width="8" height="8" rx="2" />
          <rect x="13" y="3" width="8" height="8" rx="2" />
          <rect x="3" y="13" width="8" height="8" rx="2" />
          <rect x="13" y="13" width="8" height="8" rx="2" />
        </svg>
        <svg
          v-else-if="page.id === 'market'"
          class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        >
          <path d="M3 16l5-4 4 3 6-8" />
          <path d="M15 7h3v3" />
          <path d="M3 21h18" />
        </svg>
        <svg
          v-else-if="page.id === 'control'"
          class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        >
          <rect x="3" y="4" width="18" height="6" rx="3" />
          <rect x="3" y="14" width="18" height="6" rx="3" />
          <circle cx="9" cy="7" r="2" fill="currentColor" stroke="none" />
          <circle cx="15" cy="17" r="2" fill="currentColor" stroke="none" />
        </svg>
        <svg
          v-else-if="page.id === 'simulation'"
          class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        >
          <path d="M3 5l4 3-4 3" />
          <path d="M7 8h14" />
          <path d="M21 19l-4-3 4-3" />
          <path d="M17 16H3" />
        </svg>
        <svg
          v-else-if="page.id === 'agent'"
          class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        >
          <circle cx="11" cy="11" r="6.5" />
          <path d="M16 16l4.5 4.5" />
          <path d="M8.5 11h5M11 8.5v5" />
        </svg>
        <svg
          v-else
          class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        >
          <circle cx="12" cy="12" r="4.5" />
          <circle cx="12" cy="12" r="8.5" stroke-dasharray="2.6 3.4" />
          <path d="M12 2v3M12 19v3M2 12h3M19 12h3" />
        </svg>
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
  width: 76px;
  min-height: 100vh;
  background: var(--bg-sidebar);
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 18px 0;
  position: fixed;
  left: 0;
  top: 0;
  z-index: 100;
  border-right: 1px solid #22382c;
}
.sidebar-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  margin-bottom: 30px;
}
.brand-icon {
  width: 38px;
  height: 38px;
  display: grid;
  place-items: center;
  border-radius: var(--radius-md);
  color: var(--bg-sidebar);
  background: var(--lime);
  font-size: 22px;
  font-weight: 900;
  transform: rotate(-10deg);
  box-shadow: 0 2px 8px rgba(200, 239, 59, .22);
}
.brand-text {
  color: #7b9b8a;
  font-size: var(--fs-10);
  letter-spacing: .14em;
  font-weight: 700;
}
.nav-links {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  width: 100%;
  align-items: center;
}
.nav-item {
  position: relative;
  width: 62px;
  height: 56px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background .15s, color .15s;
  color: #5a7a6a;
}
.nav-item:hover {
  background: #22372c;
  color: #a5c6b2;
}
.nav-item.active {
  background: #264134;
  color: var(--lime);
}
.nav-item.active::before {
  content: '';
  position: absolute;
  left: -7px;
  top: 50%;
  transform: translateY(-50%);
  width: 3px;
  height: 24px;
  border-radius: 0 3px 3px 0;
  background: var(--lime);
}
.nav-icon {
  width: 19px;
  height: 19px;
}
.nav-label {
  font-size: var(--fs-10);
  letter-spacing: .04em;
  font-weight: 600;
}
.nav-dot {
  position: absolute;
  top: 8px;
  right: 10px;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #26a269;
}
.sidebar-footer {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 7px;
  padding-top: 16px;
  border-top: 1px solid #22382c;
  width: 100%;
}
.status-dot {
  width: 9px;
  height: 9px;
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
  color: #6d8a7a;
  font-size: var(--fs-10);
  letter-spacing: .06em;
}
@keyframes pulse {
  70% { box-shadow: 0 0 0 6px rgba(38, 162, 105, 0); }
  100% { box-shadow: 0 0 0 0 rgba(38, 162, 105, 0); }
}
</style>