<script setup lang="ts">
/**
 * 个股分析（codex 只读分析）页 —— PORT-01 / UX-01。
 *
 * 结构与视觉对齐 **Codex 桌面版**：一条会话流（用户消息 → 工具执行块 → 助手正文）
 * 加上底部固定输入区，而不是「表单 + 结果面板」。令牌取自本机 Codex 桌面版的
 * 实际样式表，见 `src/codex-theme.css`。
 *
 * 边界不变（架构文档 §1.2）：只读分析，不接任何写操作，也不参与交易决策。
 * 审批是显式交互：运行时在执行有副作用的动作前会提问，此处必须由所有者给出
 * 「同意/拒绝」；没有倒计时放行，也没有「本会话内不再询问」。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  agentAsk,
  agentDecide,
  agentDefaultPrompt,
  agentReady,
  agentStart,
  agentStatus,
  agentStop,
} from '../commands'
import { AGENT_DECISION_LABEL } from '../commands'
import type { AgentDecision, AgentReady, AgentStatus, AgentTurn } from '../commands'

/** 运行时报告可用的 skill 列表，用于工具栏提示。 */

const props = defineProps<{
  active: boolean
}>()

const ready = ref<AgentReady | null>(null)
const status = ref<AgentStatus | null>(null)
const prompt = ref('')
const busy = ref(false)
/** 展开的工具调用卡（key = `${turn.seq}:${call.call_id}`）。 */
const expanded = ref<Set<string>>(new Set())
const thread = ref<HTMLElement | null>(null)
const composer = ref<HTMLTextAreaElement | null>(null)

const POLL_MS = 1000
let pollTimer: ReturnType<typeof setInterval> | null = null

const phase = computed(() => status.value?.phase ?? 'stopped')
const pending = computed(() => status.value?.pending_approval ?? null)
const turns = computed<AgentTurn[]>(() => status.value?.turns ?? [])
const running = computed(
  () => phase.value === 'running' || phase.value === 'awaiting_approval',
)
const sessionAlive = computed(() => phase.value !== 'stopped' && phase.value !== 'error')

const PHASE: Record<string, { label: string; tone: string }> = {
  stopped: { label: '未启动', tone: '' },
  starting: { label: '启动中', tone: 'warn' },
  ready: { label: '就绪', tone: 'ok' },
  running: { label: '分析中', tone: 'warn' },
  awaiting_approval: { label: '等待审批', tone: 'bad' },
  error: { label: '错误', tone: 'bad' },
}
/**
 * 工具栏上的状态 chip。
 *
 * 就绪状态**优先于**会话 phase：两者回答的是不同问题（能不能启动 vs 会话进行到
 * 哪），未就绪时若显示「就绪」，会与下方「无法启动分析」直接矛盾。
 */
const phaseInfo = computed(() => {
  if (ready.value && !ready.value.ready) return { label: '未就绪', tone: 'bad' }
  return PHASE[phase.value] ?? { label: phase.value, tone: '' }
})

/** 进行中的轮次：入列了但还没结束时间。 */
function isTurnOpen(turn: AgentTurn): boolean {
  return turn.finished_at_ms === null
}

/** 会话时间线里的最后一条进行中轮次才显示「进行中」，避免历史轮次误标。 */
const openTurnSeq = computed(() => {
  const last = turns.value.at(-1)
  return last && isTurnOpen(last) ? last.seq : null
})

function toolKey(turn: AgentTurn, callId: string): string {
  return `${turn.seq}:${callId}`
}

function toggleTool(turn: AgentTurn, callId: string) {
  const key = toolKey(turn, callId)
  const next = new Set(expanded.value)
  if (next.has(key)) {
    next.delete(key)
  } else {
    next.add(key)
  }
  expanded.value = next
}

function isExpanded(turn: AgentTurn, callId: string): boolean {
  return expanded.value.has(toolKey(turn, callId))
}

/** 工具输出对齐 codex：默认折叠，只把首行作为预览露出。 */
function preview(output: string): string {
  const first = output.split('\n')[0] ?? ''
  return first.length > 96 ? `${first.slice(0, 96)}…` : first
}

function stopPolling() {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

async function scrollToBottom() {
  await nextTick()
  const el = thread.value
  if (el) el.scrollTop = el.scrollHeight
}

function startPolling() {
  if (pollTimer !== null) return
  pollTimer = setInterval(async () => {
    const next = await agentStatus()
    if (next) status.value = next
  }, POLL_MS)
}

async function refresh() {
  status.value = (await agentStatus()) ?? null
  if (sessionAlive.value) startPolling()
  await scrollToBottom()
}

async function doStart() {
  busy.value = true
  try {
    status.value = (await agentStart()) ?? status.value
    startPolling()
  } finally {
    busy.value = false
  }
}

async function doStop() {
  busy.value = true
  try {
    status.value = (await agentStop()) ?? null
    stopPolling()
  } finally {
    busy.value = false
  }
}

async function doAsk() {
  const text = prompt.value.trim()
  if (!text || running.value) return
  busy.value = true
  try {
    status.value = (await agentAsk(text)) ?? status.value
    prompt.value = ''
    // 清空后必须收回高度，否则框会停在上一条长指令的尺寸。
    await resizeComposer()
    startPolling()
    await scrollToBottom()
  } finally {
    busy.value = false
  }
}

async function doDecide(decision: AgentDecision) {
  const request = pending.value
  if (!request) return
  busy.value = true
  try {
    status.value = (await agentDecide(request.request_id, decision)) ?? status.value
  } finally {
    busy.value = false
  }
}

/**
 * 单个裁决按钮的语义。
 *
 * 三个「允许」的区别只在**授权范围**，所以按钮必须把范围说清楚，而不是都叫
 * 「同意」：一次 / 本会话 / 永久（写入 execpolicy）。
 */
function decisionHint(decision: AgentDecision): string {
  switch (decision) {
    case 'allow_once':
      return '只批准本次，不记住'
    case 'allow_for_session':
      return '本会话内同类动作不再询问'
    case 'allow_always':
      return '追加一条长期规则，此后同类动作不再询问'
    case 'deny':
      return '拒绝本次，不记住'
  }
}

function decisionClass(decision: AgentDecision): string {
  if (decision === 'deny') return 'codex-btn danger'
  if (decision === 'allow_once') return 'codex-btn primary'
  return 'codex-btn ghost'
}

/**
 * 让输入框随内容增高，上限由 CSS 的 `max-height` 决定。
 *
 * 先归零再读 `scrollHeight`：不归零时高度只增不减，删掉内容后框会留在大尺寸。
 * 超出上限后由 CSS 的 `overflow-y: auto` 接管滚动。
 */
async function resizeComposer() {
  await nextTick()
  const el = composer.value
  if (!el) return
  el.style.height = 'auto'
  el.style.height = `${el.scrollHeight}px`
}

/** Enter 发送，Shift+Enter 换行（与 codex 输入区一致）。 */
function onComposerKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    doAsk()
  }
}

onMounted(async () => {
  ready.value = (await agentReady()) ?? null
  prompt.value = (await agentDefaultPrompt()) ?? ''
  await resizeComposer()
  await refresh()
  await scrollToBottom()
})

watch(
  () => props.active,
  async active => {
    if (active) {
      await refresh()
    } else {
      stopPolling()
    }
  },
)

// 新内容到达时保持在底部（会话流的常规行为）。
watch(
  () => [turns.value.length, status.value?.phase, pending.value?.request_id],
  () => void scrollToBottom(),
)

onUnmounted(stopPolling)
</script>

<template>
  <div class="page-content codex-scope">
    <!-- 顶部工具栏：对齐 codex 的 46px 工具条 -->
    <header class="codex-toolbar">
      <div class="codex-toolbar-title">
        <strong>个股分析</strong>
        <span class="codex-chip" :class="phaseInfo.tone">
          <i class="codex-dot" />{{ phaseInfo.label }}
        </span>
      </div>
      <div class="codex-toolbar-meta">
        <template v-if="ready?.ready">
          <span v-if="status?.thread_id" :title="status.thread_id">
            thread {{ status.thread_id.slice(0, 8) }}
          </span>
          <!-- trace 文件名即「按 thread id 回放」的线索（轮次 D），保留可见。 -->
          <span v-if="status?.trace_path" :title="status.trace_path">
            trace {{ status.trace_path.split('/').pop() }}
          </span>
          <span v-if="ready.sandbox" :title="ready.sandbox">沙箱</span>
          <span v-if="ready.instructions" :title="`领域指令 ${ready.instructions}`">
            指令 {{ ready.instructions }}
          </span>
          <!-- skill 是运行时认可的，不是我们请求注册的：名字带前缀（如
               stock-deep-analyzer:uzi）说明它来自额外 skill 根。 -->
          <span
            v-if="status?.skills.length"
            :title="status.skills.map(s => `${s.name} — ${s.description}`).join('\n\n')"
          >
            skill {{ status.skills.length }}
          </span>
          <!-- skill 通常靠「跑脚本」工作，而脚本会写自己的仓库。配了 skill 根却
               没配可写根时，模型会读完 SKILL.md、拿到批准、然后卡在写入上——
               这组合看起来正常，所以在这里显式提示。 -->
          <span
            v-if="ready.skill_roots.length && !ready.write_roots.length"
            class="codex-chip warn"
            title="已配置 skill 根但未授权任何可写根：skill 的脚本若需写自己的仓库将失败"
          >
            <i class="codex-dot" />缺可写根
          </span>
        </template>
      </div>
    </header>

    <!-- 未就绪：给出可操作原因与逐项检查，不提供启动入口 -->
    <section v-if="ready && !ready.ready" class="codex-blocked">
      <div class="codex-blocked-title"><i class="codex-dot" style="color: var(--cx-red)" />无法启动分析</div>
      <p class="codex-blocked-reason">{{ ready.reason }}</p>
      <div class="codex-checks">
        <div class="codex-check">
          <span>运行时</span><code>{{ ready.program ?? '未找到' }}</code>
        </div>
        <div class="codex-check">
          <span>trace</span><code>{{ ready.trace_dir ?? '不可写' }}</code>
        </div>
        <div class="codex-check">
          <span>沙箱</span><code>{{ ready.sandbox }}</code>
        </div>
        <div class="codex-check">
          <span>skill 根</span>
          <code>{{ ready.skill_roots.length ? ready.skill_roots.join(' , ') : '未配置' }}</code>
        </div>
        <div class="codex-check">
          <span>可写根</span>
          <code>{{ ready.write_roots.length ? ready.write_roots.join(' , ') : '仅 codex home 与 trace' }}</code>
        </div>
      </div>
    </section>

    <template v-else>
      <!-- 会话流 -->
      <div ref="thread" class="codex-thread">
        <div v-if="!turns.length" class="codex-empty">
          <div>尚未发起分析。</div>
          <div>说明要分析哪只个股即可；能力来自可用的 skill 与运行时工具。</div>
        </div>

        <article v-for="turn in turns" :key="turn.seq" class="codex-turn">
          <!-- 用户消息 -->
          <div class="codex-user">{{ turn.prompt }}</div>

          <!-- 工具执行块 -->
          <div v-for="call in turn.tool_calls" :key="call.call_id" class="codex-tool">
            <button
              class="codex-tool-head"
              type="button"
              @click="toggleTool(turn, call.call_id)"
            >
              <i class="codex-tool-caret" :class="{ open: isExpanded(turn, call.call_id) }">▶</i>
              <span class="codex-tool-name">{{ call.tool }}</span>
              <span class="codex-tool-status">
                {{ call.success ? preview(call.output) : '失败' }}
              </span>
            </button>
            <pre v-if="isExpanded(turn, call.call_id)" class="codex-tool-body">{{ call.output }}</pre>
          </div>

          <!-- 审批记录（已裁决）。拒绝用中性色、「无条件放行」的三档用成功色，
               让长期授权比一次性授权更显眼。 -->
          <div
            v-for="record in turn.approvals"
            :key="`a-${record.request_id}`"
            class="codex-chip"
            :class="record.decision === 'deny' ? '' : 'ok'"
          >
            {{ AGENT_DECISION_LABEL[record.decision] }} ·
            {{ record.kind }} ·
            <code>{{ record.summary }}</code>
            <template v-if="record.source === 'timeout'"> · 超时未裁决</template>
          </div>

          <!-- 助手正文 -->
          <div v-if="turn.final_message" class="codex-assistant">{{ turn.final_message }}</div>

          <!-- 待审批：内联在会话流里 -->
          <div v-if="pending && turn.seq === openTurnSeq" class="codex-approval">
            <div class="codex-approval-head">
              <i class="codex-dot" style="color: var(--cx-orange)" />
              需要审批后才会执行
            </div>
            <div class="codex-approval-cmd">{{ pending.summary }}</div>
            <!-- 四个动作与 Codex 桌面版审批卡一致：允许一次 / 允许此对话 /
                 始终允许 / 拒绝。选项由后端按请求类型给出（例如未附带具体规则
                 时不会出现「始终允许」），界面只负责呈现与回传。 -->
            <div class="codex-approval-actions">
              <button
                v-for="option in pending.options"
                :key="option"
                :class="decisionClass(option)"
                type="button"
                :disabled="busy"
                :title="decisionHint(option)"
                @click="doDecide(option)"
              >
                {{ AGENT_DECISION_LABEL[option] }}
              </button>
            </div>
            <div class="codex-approval-hint">
              「允许一次」不记住；「允许此对话」在本会话内不再询问；「始终允许」会追加一条长期规则。
              未裁决将按失败关闭。
            </div>
            <div v-if="pending.advertised.length" class="codex-approval-advertised">
              运行时提供：{{ pending.advertised.join(' / ') }}
            </div>
          </div>

          <!-- 本轮失败 -->
          <div v-if="turn.error" class="codex-turn-error">
            <div>
              <b>本轮失败</b>
              <pre>{{ turn.error }}</pre>
            </div>
          </div>

          <!-- 进行中：中性色而非橙色——待审批时信号在审批卡上，同色相邻会让
               两者看成一块，反而弱化了真正需要注意的那一个。 -->
          <div v-if="turn.seq === openTurnSeq && !turn.error" class="codex-chip">
            <i class="codex-dot" />分析中…
          </div>

          <!-- 未实现而被拒的请求（安全信号） -->
          <div v-if="turn.refused_requests.length" class="codex-chip">
            已拒绝未实现的请求：{{ turn.refused_requests.join('、') }}
          </div>
        </article>
      </div>

      <!-- 会话级错误（如运行时退出） -->
      <div v-if="status?.error" class="codex-turn-error">
        <div><b>会话错误</b><pre>{{ status.error }}</pre></div>
      </div>

      <!-- 底部输入区：codex composer。用 div 而非 footer：style.css 有一条裸
           `footer` 元素选择器（给 AppFooter 用），会把 display 改成 flex，
           使这里的输入框与提示行并排而非堆叠。 -->
      <div class="codex-composer">
        <div class="codex-composer-box">
          <textarea
            ref="composer"
            v-model="prompt"
            rows="3"
            :disabled="running"
            placeholder="例如：分析 600519.SH，先说明用哪个 skill，再给结论与依据"
            @keydown="onComposerKeydown"
            @input="resizeComposer"
          />
          <button
            v-if="!sessionAlive"
            class="codex-btn primary"
            type="button"
            :disabled="busy"
            @click="doStart"
          >
            启动
          </button>
          <template v-else>
            <button
              class="codex-btn send"
              type="button"
              :disabled="busy || running || !prompt.trim()"
              title="发送"
              @click="doAsk"
            >
              ↑
            </button>
            <button class="codex-btn ghost" type="button" :disabled="busy" @click="doStop">
              停止
            </button>
          </template>
        </div>
        <div class="codex-composer-hint">
          Enter 发送 · Shift+Enter 换行 · 宿主不注册工具 · 副作用动作需审批
        </div>
      </div>
    </template>
  </div>
</template>
