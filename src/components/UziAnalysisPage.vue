<script setup lang="ts">
/**
 * 个股分析（UZI-Skill）页。
 *
 * 分析核在 Rust（`crates/uzi`），取数与 HTML 渲染留在 Python skill 侧车；
 * 一条流水线：取数 → 量化因子识别 → Rust 分析 → 渲染。
 *
 * 报告在 iframe 中展示：报告是自包含 HTML（头像是内联 data URI），后端经
 * `taoli-uzi://` 自定义协议提供，并按需剥离外部引用（Google Fonts、
 * 二维码图床）。父页面 CSP 因此无需放宽——iframe 内是独立文档、自带更严策略。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ElButton, ElInput, ElTag } from 'element-plus'
import { uziCancel, uziReady, uziStart, uziStatus, uziReportUrl } from '../commands'
import type { UziReady, UziStatus } from '../commands'

const props = defineProps<{
  active: boolean
}>()

const ready = ref<UziReady | null>(null)
const readyError = ref('')
const ticker = ref('600519')
const status = ref<UziStatus | null>(null)
const reportUrl = ref('')
const starting = ref(false)

const POLL_MS = 3000
let pollTimer: ReturnType<typeof setInterval> | null = null

const STAGE_LABEL: Record<string, string> = {
  FETCHING: '采集行情数据',
  ANALYZING: 'Rust 分析中',
  RENDERING: '生成报告',
  DONE: '完成',
  FAILED: '失败',
  CANCELLED: '已取消',
}

const running = computed(() => status.value?.running ?? false)
const stageLabel = computed(() =>
  status.value?.stage ? (STAGE_LABEL[status.value.stage] ?? status.value.stage) : '',
)
const elapsed = computed(() => {
  const ms = status.value?.elapsed_ms
  if (ms === null || ms === undefined) return ''
  const total = Math.floor(ms / 1000)
  return `${Math.floor(total / 60)}分${String(total % 60).padStart(2, '0')}秒`
})

async function refresh() {
  status.value = (await uziStatus()) ?? null
  if (status.value?.stage === 'DONE') {
    reportUrl.value = (await uziReportUrl()) ?? ''
  }
}

function startPolling() {
  if (pollTimer !== null) return
  pollTimer = setInterval(async () => {
    await refresh()
    if (!running.value) stopPolling()
  }, POLL_MS)
}

function stopPolling() {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

async function run() {
  if (!ticker.value.trim()) return
  starting.value = true
  reportUrl.value = ''
  try {
    status.value = (await uziStart(ticker.value.trim())) ?? null
    startPolling()
  } finally {
    starting.value = false
  }
}

async function cancel() {
  status.value = (await uziCancel()) ?? null
  stopPolling()
}

onMounted(async () => {
  const result = await uziReady()
  if (result?.ready) {
    ready.value = result
  } else {
    readyError.value =
      '未配置 UZI-Skill。设置 TAOLI_UZI_DIR 指向 skill 检出目录、TAOLI_UZI_PYTHON 指向解释器（需 Python 3.10+）。'
  }
  await refresh()
  if (running.value) startPolling()
})

onUnmounted(stopPolling)
</script>

<template>
  <div class="page-content uzi-page">
    <section v-if="readyError" class="panel">
      <div class="panel-head"><h2><em class="orange-dot" /> 个股分析</h2></div>
      <div class="empty-state">{{ readyError }}</div>
    </section>

    <template v-else>
      <section class="panel uzi-toolbar">
        <div class="panel-head">
          <h2><em class="orange-dot" /> 个股分析 · UZI-Skill</h2>
          <span class="muted">
            分析核 Rust · 取数与渲染 Python 侧车
            <template v-if="ready?.python"> · {{ ready.python }}</template>
          </span>
        </div>

        <div class="uzi-form">
          <ElInput
            v-model="ticker"
            placeholder="股票代码或中文名，如 600519 / 贵州茅台"
            size="small"
            style="width: 260px"
            :disabled="running"
            @keyup.enter="run"
          />
          <ElButton size="small" type="primary" :loading="starting" :disabled="running" @click="run">
            开始分析
          </ElButton>
          <ElButton size="small" :disabled="!running" @click="cancel">取消</ElButton>

          <template v-if="status?.stage">
            <ElTag size="small" :type="status.stage === 'FAILED' ? 'danger' : status.stage === 'DONE' ? 'success' : 'info'">
              {{ stageLabel }}
            </ElTag>
            <span v-if="elapsed" class="uzi-elapsed">{{ elapsed }}</span>
          </template>
        </div>

        <p class="uzi-note">
          采集阶段需联网抓取行情与财报，通常数分钟；「采集行情数据」停留较久属正常。
        </p>

        <div v-if="status?.error" class="uzi-error">
          <b>分析失败</b>
          <pre>{{ status.error }}</pre>
        </div>

        <div v-if="status?.verdict_label" class="uzi-result">
          <span class="uzi-metric"><b>{{ status.overall_score }}</b><small>总分</small></span>
          <span class="uzi-metric"><b>{{ status.verdict_label }}</b><small>研判</small></span>
          <span class="uzi-metric"><b>{{ status.detected_style }}</b><small>识别风格</small></span>
          <span class="uzi-metric"><b>{{ status.investor_count }}</b><small>评委数</small></span>
        </div>
      </section>

      <section v-if="reportUrl" class="panel uzi-report">
        <div class="panel-head">
          <h2><em class="green-dot" /> 分析报告</h2>
          <span class="muted">自包含 HTML · 外部引用已剥离（字体、二维码图床）</span>
        </div>
        <iframe :src="reportUrl" class="uzi-frame" title="UZI 分析报告" />
      </section>

      <section v-else-if="!status?.stage" class="panel">
        <div class="empty-state">输入代码开始分析。</div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.uzi-page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.uzi-form {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 10px 16px 4px;
}
.uzi-elapsed {
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--fs-11);
}
.uzi-note {
  margin: 0;
  padding: 0 16px 12px;
  color: var(--text-3);
  font-size: var(--fs-11);
}
.uzi-error {
  margin: 0 16px 12px;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  font-size: var(--fs-11);
}
.uzi-error pre {
  margin: 6px 0 0;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: var(--font-mono);
  color: var(--text-2);
}
.uzi-result {
  display: flex;
  gap: 24px;
  padding: 0 16px 14px;
}
.uzi-metric {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.uzi-metric b {
  font-size: var(--fs-16);
  color: var(--text-1);
}
.uzi-metric small {
  color: var(--text-3);
  font-size: var(--fs-10);
  letter-spacing: 0.06em;
}
.uzi-report {
  display: flex;
  flex-direction: column;
  min-height: 70vh;
}
.uzi-frame {
  flex: 1;
  width: 100%;
  min-height: 68vh;
  border: 0;
  border-top: 1px solid var(--border);
  background: #fff;
}
.empty-state {
  padding: 40px 18px;
  color: var(--text-3);
  font-size: var(--fs-12);
  text-align: center;
}
</style>
