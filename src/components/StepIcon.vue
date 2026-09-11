<script setup lang="ts">
/**
 * The icon for one step in the conversation.
 *
 * Two levels, because a chat log has two questions to answer at a glance:
 *
 * 1. **What kind of step is this?** — reasoning, a shell command, a tool call, a
 *    file change. Taken from the runtime's own item type, so it is not a guess.
 * 2. **What command is it, in particular?** — a `git` invocation and a `python`
 *    one are both "a command", but they are not read the same way.
 *
 * The second level is a **heuristic on the command text**, and presentation-only:
 * an unrecognised command falls back to a generic terminal icon, so a miss costs a
 * less specific glyph, never correctness. The list is deliberately short and named
 * in one place so it is obvious what it does and does not cover.
 *
 * Note on unwrapping: the runtime runs commands through a shell, so the text to
 * look at is inside the wrapper — `/bin/zsh -lc 'git status'` would otherwise be
 * classified by its `/bin/zsh` prefix, i.e. always "generic shell".
 *
 * Icons carry no text, so every use must supply `label` for screen readers and
 * `title` (the command itself) as the hover explanation.
 */
import { computed } from 'vue'

const props = defineProps<{
  /** Runtime item type: reasoning | command | file_change | tool_call | message | other. */
  kind: string
  /** Command line or tool name; only consulted when `kind` is a command. */
  title?: string
  /** Human-readable kind name, for `aria-label`. */
  label?: string
}>()

type IconName =
  | 'reasoning'
  | 'terminal'
  | 'vcs'
  | 'runtime'
  | 'search'
  | 'read'
  | 'net'
  | 'tool'
  | 'file'
  | 'message'
  | 'other'

/**
 * Command families, matched against the command's first meaningful word.
 *
 * Kept short on purpose: each entry is a glyph the reader will actually see often.
 * Anything absent — `make`, `docker`, a project script — is covered by `terminal`.
 */
const FAMILIES: { icon: IconName; commands: string[] }[] = [
  { icon: 'vcs', commands: ['git', 'gh', 'jj', 'hg', 'svn'] },
  {
    icon: 'runtime',
    commands: [
      'python', 'python3', 'pip', 'pip3', 'uv', 'poetry', 'pytest',
      'node', 'npm', 'npx', 'pnpm', 'yarn', 'bun', 'deno',
      'cargo', 'rustc', 'go', 'java', 'mvn', 'gradle',
    ],
  },
  { icon: 'search', commands: ['grep', 'rg', 'ag', 'ack', 'find', 'fd', 'locate', 'which'] },
  { icon: 'read', commands: ['cat', 'head', 'tail', 'less', 'more', 'wc', 'ls', 'tree', 'stat', 'file', 'jq'] },
  { icon: 'net', commands: ['curl', 'wget', 'http', 'ssh', 'scp', 'nc', 'ping', 'dig'] },
]

/** Wrappers that prefix a command without changing what it does. */
const TRANSPARENT_PREFIXES = new Set(['sudo', 'env', 'command', 'nohup', 'time', 'exec'])

/**
 * Strips the runtime's shell wrapper, leaving the script it runs.
 *
 * `/bin/zsh -lc 'git status'` → `git status`. A command without a wrapper is
 * returned unchanged, so callers need not know which form they hold.
 */
function unwrapShell(command: string): string {
  const wrapped = command.match(/^\S*(?:ba|z|da|k|fi)?sh\s+-\S*\s+([\s\S]*)$/)
  const body = (wrapped ? wrapped[1] : command).trim()
  const quoted = body.match(/^(['"])([\s\S]*)\1$/)
  return (quoted ? quoted[2] : body).trim()
}

/** The first word that names a command, skipping wrappers and assignments. */
function firstCommandWord(command: string): string {
  let tokens = unwrapShell(command)
    .split(/\s+/)
    // `FOO=bar cmd` sets an environment variable; the command follows it.
    .filter(token => token !== '' && !/^\w+=/.test(token))
  while (tokens.length > 0 && TRANSPARENT_PREFIXES.has(tokens[0].toLowerCase())) {
    tokens = tokens.slice(1)
  }
  // `/usr/bin/python3` and `python.exe` name the same command as `python3`.
  return (tokens[0] ?? '').split('/').pop()?.replace(/\.(exe|cmd|bat)$/i, '') ?? ''
}

const icon = computed<IconName>(() => {
  switch (props.kind) {
    case 'reasoning':
      return 'reasoning'
    case 'tool_call':
      return 'tool'
    case 'file_change':
      return 'file'
    case 'message':
      return 'message'
    case 'command': {
      const word = firstCommandWord(props.title ?? '')
      return FAMILIES.find(family => family.commands.includes(word))?.icon ?? 'terminal'
    }
    default:
      return 'other'
  }
})
</script>

<template>
  <svg
    class="step-icon"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    role="img"
    :aria-label="label ?? kind"
  >
    <!-- 推理：灯泡。过程，不是结论。 -->
    <template v-if="icon === 'reasoning'">
      <path d="M12 3a6 6 0 0 0-3.8 10.6V16h7.6v-2.4A6 6 0 0 0 12 3z" />
      <path d="M9.5 19h5" />
      <path d="M10.5 21.5h3" />
    </template>

    <!-- 通用 shell：终端提示符 -->
    <template v-else-if="icon === 'terminal'">
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" />
      <path d="M7.5 10l2.5 2.5-2.5 2.5" />
      <path d="M13 15h3.5" />
    </template>

    <!-- 版本控制：分支 -->
    <template v-else-if="icon === 'vcs'">
      <circle cx="6.5" cy="5.5" r="2.5" />
      <circle cx="6.5" cy="18.5" r="2.5" />
      <circle cx="17.5" cy="9.5" r="2.5" />
      <path d="M6.5 8v8" />
      <path d="M17.5 12a6.5 6.5 0 0 1-6.5 6.5H8.5" />
    </template>

    <!-- 语言/运行时：尖括号 -->
    <template v-else-if="icon === 'runtime'">
      <path d="M8.5 8.5 5 12l3.5 3.5" />
      <path d="M15.5 8.5 19 12l-3.5 3.5" />
      <path d="M13.5 5.5l-3 13" />
    </template>

    <!-- 搜索：放大镜 -->
    <template v-else-if="icon === 'search'">
      <circle cx="11" cy="11" r="6.5" />
      <path d="M16 16l4 4" />
    </template>

    <!-- 读取：眼睛。
         原本用「文档+横线」，但 file_change 是「文档+加号」——两者共用同一个文档轮廓，
         16px 下几乎分不出来。轮廓不同比细节不同更重要：小而常看的东西靠剪影区分。 -->
    <template v-else-if="icon === 'read'">
      <path d="M2.5 12S6.5 6 12 6s9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z" />
      <circle cx="12" cy="12" r="2.5" />
    </template>

    <!-- 网络：地球 -->
    <template v-else-if="icon === 'net'">
      <circle cx="12" cy="12" r="8" />
      <path d="M4 12h16" />
      <path d="M12 4c2.6 2.8 2.6 13.2 0 16-2.6-2.8-2.6-13.2 0-16z" />
    </template>

    <!-- 工具调用：扳手 -->
    <template v-else-if="icon === 'tool'">
      <path
        d="M14.9 5.6a4.6 4.6 0 0 1 5.6 5.6l-7.7 7.7a2.2 2.2 0 0 1-3.1 0l-2.5-2.5a2.2 2.2 0 0 1 0-3.1z"
      />
      <path d="M9.5 9.5l-5 5a2.2 2.2 0 0 0 3.1 3.1" />
    </template>

    <!-- 文件变更：文档内加号 -->
    <template v-else-if="icon === 'file'">
      <path d="M6.5 3h7l4 4v14h-11z" />
      <path d="M13.5 3v4h4" />
      <path d="M12 11.5v6" />
      <path d="M9 14.5h6" />
    </template>

    <!-- 助手消息：对话气泡 -->
    <template v-else-if="icon === 'message'">
      <path d="M4 6.5h16v10H9.5L4 20.5z" />
    </template>

    <!-- 其它：中性方块，不假装是已知类型 -->
    <template v-else>
      <rect x="9" y="9" width="6" height="6" rx="1.5" />
    </template>
  </svg>
</template>
