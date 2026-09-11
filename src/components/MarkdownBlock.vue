<script setup lang="ts">
/**
 * Renders untrusted markdown from agent output.
 *
 * Sanitising happens in `renderMarkdown` (raw HTML off, link validation on);
 * this component adds the one thing HTML cannot express safely here: **links
 * must not navigate**. No external-opener plugin is installed, so following a
 * link inside the webview would replace the app with a remote page. The URL
 * stays readable in the anchor's `title`.
 */
import { computed } from 'vue'
import { renderMarkdown } from '../markdown'

const props = defineProps<{
  source: string
}>()

const html = computed(() => renderMarkdown(props.source))

/** Suppresses navigation, keeping the destination inspectable via `title`. */
function blockNavigation(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest('a')
  if (anchor) {
    event.preventDefault()
  }
}
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html -- sanitised in renderMarkdown -->
  <div class="markdown-body" @click="blockNavigation" v-html="html" />
</template>
