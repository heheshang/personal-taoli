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
import {
  agentAccessLevels,
  agentSetAccessLevel,
} from '../commands'
import MarkdownBlock from './MarkdownBlock.vue'
import ReportView from './ReportView.vue'
import StepRow from './StepRow.vue'
import { AGENT_DECISION_LABEL } from '../commands'
import type {
  AgentAccessLevel,
  AgentDecision,
  AgentLive,
  AgentReady,
  AgentStatus,
  AgentTurn,
} from '../commands'

/** 运行时报告可用的 skill 列表，用于工具栏提示。 */

const props = defineProps<{
  active: boolean
}>()

const ready = ref<AgentReady | null>(null)
const status = ref<AgentStatus | null>(null)
const prompt = ref('')
const busy = ref(false)
const thread = ref<HTMLElement | null>(null)
const composer = ref<HTMLTextAreaElement | null>(null)
const levels = ref<AgentAccessLevel[]>([])
/**
 * 已展开的步骤（实时与已完成共用）。
 *
 * 按 `item_id` 键控，而 id 在一条会话里唯一，故一张表即可服务两个来源；
 * 默认折叠，因为一轮可以产出兆字节输出，而对话是用来读的。
 */
const expandedSteps = ref<Set<string>>(new Set())
function toggleStep(itemId: string) {
  const next = new Set(expandedSteps.value)
  if (next.has(itemId)) {
    next.delete(itemId)
  } else {
    next.add(itemId)
  }
  expandedSteps.value = next
}

/**
 * 当前档位。
 *
 * 会话运行中显示该会话**实际**使用的档位（后端在 thread/start 时按它确定沙箱与
 * 审批设置），未运行时显示下一会话将使用的档位。
 */
const accessLevel = computed(() => status.value?.access_level ?? 'ask')
const accessInfo = computed(
  () => levels.value.find(level => level.token === accessLevel.value) ?? null,
)

/**
 * 该档位是否让本页的审批卡失去意义。
 *
 * 「帮我批准」由运行时自己的审查子代理判定，「完全访问」根本不问——两者都不会把
 * 请求交给本页。界面必须说清楚，否则使用者会误以为每个动作仍由自己把关。
 */
const approvalBypassed = computed(
  () => !!accessInfo.value && !accessInfo.value.consults_client,
)

/** 进行中轮次的实时视图。 */
const live = computed<AgentLive | null>(() => status.value?.live ?? null)

/** 实时阶段的中文说明。 */
const STAGE_LABEL: Record<string, string> = {
  thinking: '思考中',
  working: '执行中',
  writing: '生成回复',
  awaiting_approval: '等待审批',
  done: '结束',
}
const liveStageLabel = computed(() =>
  live.value ? (STAGE_LABEL[live.value.stage] ?? live.value.stage) : '',
)

/** token 用量摘要，运行时上报后才有。 */
const liveTokens = computed(() => {
  const tokens = live.value?.tokens
  if (!tokens) return null
  const window = tokens.context_window
  return window
    ? `${tokens.total_tokens.toLocaleString()} / ${window.toLocaleString()} tokens`
    : `${tokens.total_tokens.toLocaleString()} tokens`
})

const POLL_MS = 1000
let pollTimer: ReturnType<typeof setInterval> | null = null

const phase = computed(() => status.value?.phase ?? 'stopped')
const pending = computed(() => status.value?.pending_approval ?? null)
const turns = computed<AgentTurn[]>(() => status.value?.turns ?? [])
const running = computed(
  () => phase.value === 'running' || phase.value === 'awaiting_approval',
)
const sessionAlive = computed(() => phase.value !== 'stopped' && phase.value !== 'error')

/**
 * 永久放行在本页不可用。
 *
 * 后端把运行时的规则目录设为不可写（否则一次点击会改掉这台机器上**所有** codex
 * 会话的行为），而「始终允许」的全部意义就是写下那条规则。实测：规则写不进去时
 * 该裁决会被接受但**毫无效果**——下一条同样的命令仍会被拦。因此后端不再提供它，
 * 界面要说明这一点，而不是让使用者对着 codex app 少一个按钮发愣。
 */
const permanentGrantMissing = computed(
  () => !!pending.value && !pending.value.options.includes('allow_always'),
)

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

function stopPolling() {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

/**
 * 用户是否仍停留在底部。
 *
 * 自动跟随的前提。滚离底部后必须**停手**：分析中内容每隔一秒就长一截，若每次都把
 * 视图拽回底部，用户就再也读不到前面的内容——这正是「不能上划」的成因。
 */
const pinnedToBottom = ref(true)

/** 判定「仍在底部」的容差（px）：留一点余量，避免亚像素误差导致误判为已滚离。 */
const PIN_TOLERANCE_PX = 32

function atBottom(el: HTMLElement): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= PIN_TOLERANCE_PX
}

/** 滚动事件只记录位置；滚回底部即恢复自动跟随。 */
function onThreadScroll() {
  const el = thread.value
  if (el) pinnedToBottom.value = atBottom(el)
}

/**
 * 跟随新内容滚到底部，**但仅在用户仍停留在底部时**。
 *
 * `force` 用于那些「用户的动作本身就表示要看到结果」的场合——发出提问、切回本页——
 * 此时即便之前滚上去过也应该跳到底部。
 */
async function scrollToBottom(options: { force?: boolean } = {}) {
  await nextTick()
  const el = thread.value
  if (!el) return
  if (!options.force && !pinnedToBottom.value) return
  el.scrollTop = el.scrollHeight
  pinnedToBottom.value = true
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
  // 打开/切回本页时从最新处看起；此后交给 `pinnedToBottom` 决定是否跟随。
  pinnedToBottom.value = true
  await scrollToBottom({ force: true })
}

async function doSetAccessLevel(token: string) {
  busy.value = true
  try {
    status.value = (await agentSetAccessLevel(token)) ?? status.value
  } finally {
    busy.value = false
  }
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
    // 用户的动作本身表示「我要看这轮的结果」，因此强制跳到底部。
    await scrollToBottom({ force: true })
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
  levels.value = (await agentAccessLevels()) ?? []
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

/**
 * 内容增长时跟随到底部。
 *
 * getter 必须返回**标量**。此前写的是 `() => [a, b, c]`——数组字面量每次求值都是新引用，
 * Vue 以 `Object.is` 比较返回值，永远判定为「变了」，于是这个 watch 在**每次轮询**都触发，
 * 把自动滚动变成每秒一次、无法挣脱的拽动。标量求和既表达了「内容变长了」，也真的只在
 * 长度变化时才触发。
 */
watch(
  () =>
    (status.value?.live?.items.length ?? 0) +
    (status.value?.live?.reasoning.length ?? 0) +
    (status.value?.live?.message.length ?? 0) +
    (status.value?.pending_approval ? 1 : 0) +
    turns.value.length,
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
        <!-- 档位与 Codex 桌面版的权限下拉同构：会话运行中禁用，因为沙箱与审批
             设置在 thread/start 时确定，无法追加到已在运行的进程上。 -->
        <select
          v-if="levels.length"
          class="codex-access"
          :value="accessLevel"
          :disabled="busy || sessionAlive"
          :title="accessInfo?.description ?? '选择 Codex 应如何请求批准'"
          @change="doSetAccessLevel(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="level in levels" :key="level.token" :value="level.token">
            {{ level.label }}
          </option>
        </select>
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
        <div ref="thread" class="codex-thread" @scroll.passive="onThreadScroll">
        <div v-if="!turns.length" class="codex-empty">
          <div>尚未发起分析。</div>
          <div>说明要分析哪只个股即可；能力来自可用的 skill 与运行时工具。</div>
        </div>

        <!-- 进行中轮次的实时视图：阶段 / token / 步骤流。轮次结束后后端清空，
             权威数据转入 turns。 -->
        <section v-if="live" class="codex-live">
          <div class="codex-live-head">
            <span class="codex-chip warn"><i class="codex-dot" />{{ liveStageLabel }}</span>
            <span v-if="liveTokens" class="muted agent-id">{{ liveTokens }}</span>
            <span class="muted agent-id">第 {{ live.turn_seq + 1 }} 轮</span>
          </div>

          <!-- 推理摘要流式文本 -->
          <div v-if="live.reasoning" class="codex-live-reasoning">{{ live.reasoning }}</div>

          <!-- 步骤流：命令 / 工具 / 文件变更，含运行状态与增量输出。 -->
          <StepRow
            v-for="item in live.items"
            :key="item.item_id"
            :item="item"
            :expanded="expandedSteps.has(item.item_id)"
            @toggle="toggleStep(item.item_id)"
          />

          <!-- 正文字流式文本：同样按 Markdown 渲染，否则末尾会看到源码 -->
          <MarkdownBlock
            v-if="live.message"
            class="codex-assistant"
            :source="live.message"
          />
        </section>

        <article v-for="turn in turns" :key="turn.seq" class="codex-turn">
          <!-- 用户消息 -->
          <div class="codex-user">{{ turn.prompt }}</div>

          <!-- 本轮执行过的步骤，含输出。这是对话里能看到「跑过什么」的主来源：
               它来自运行时上报的 item，命令、工具、文件变更都在其中。 -->
          <StepRow
            v-for="item in turn.items"
            :key="item.item_id"
            :item="item"
            :expanded="expandedSteps.has(item.item_id)"
            @toggle="toggleStep(item.item_id)"
          />

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

          <!-- 结构化报告：本轮按 schema 输出并通过校验时才渲染。 -->
          <ReportView v-if="turn.report" :report="turn.report" />

          <!-- 助手正文：本轮**全部**消息，按 Markdown 渲染。
               分析类 skill 会边跑边汇报，只显示最后一条会把中间结果丢掉。
               有报告时，承载它的最后一条消息不再重复显示（同一内容两种排版）。 -->
          <MarkdownBlock
            v-for="(message, index) in turn.report ? turn.messages.slice(0, -1) : turn.messages"
            :key="`m-${index}`"
            class="codex-assistant"
            :source="message"
          />

          <!-- 要求了结构化输出但未通过校验：**显示原因**。
               只给结论而不给依据，会让「模型没按结构」与「模型写错了字段」无法区分——
               二者要做的事完全不同（改指示 vs 记录模型能力）。故原因既可见也在 title 里。 -->
          <div v-if="turn.report_error" class="report-error">
            <div class="codex-chip warn">
              <i class="codex-dot" />未按结构输出，已回退为正文
            </div>
            <p class="report-error-detail" :title="turn.report_error">{{ turn.report_error }}</p>
          </div>
          <div
            v-else-if="turn.report === null && turn.messages.length && !turn.error"
            class="codex-chip"
          >
            本轮无结构化报告
          </div>

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
              「允许一次」只批准本次；「允许此对话」在本会话内不再询问。未裁决将按失败关闭。
            </div>
            <div v-if="permanentGrantMissing" class="codex-approval-hint">
              本页没有「始终允许」：运行时的规则目录被设为不可写，永久放行无法生效。
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
          <!-- 档位会绕过审批或去掉沙箱时必须说清楚：这不是可以忽略的细节。 -->
          <div v-if="approvalBypassed" class="codex-composer-hint agent-note-warn">
            当前档位「{{ accessInfo?.label }}」：{{ accessInfo?.description }}。
            审批请求<strong>不会</strong>出现在上方卡片里——
            {{ accessLevel === 'auto_approve' ? '由运行时自己的审查子代理判定' : '该档位不请求批准' }}。
            <template v-if="accessInfo && !accessInfo.confined">
              该档位<strong>不施加沙箱</strong>：agent 可不受限制地读写文件与访问网络。
            </template>
          </div>
        </div>
      </template>
  </div>
</template>
