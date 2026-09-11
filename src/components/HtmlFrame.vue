<script setup lang="ts">
/**
 * Renders agent-produced HTML, contained.
 *
 * The HTML is **untrusted**: it is whatever the model wrote, and the model reads
 * web pages. Plain markdown rendering turns raw HTML off for exactly that reason
 * (see `src/markdown.ts`); rendering it *as a document* here is only defensible
 * because of the container, so the container is the point of this component:
 *
 * * `sandbox=""` — an empty token list is the strictest setting: no scripts, no
 *   forms, no top-level navigation, and a unique opaque origin. Scripts are the
 *   decisive one: with them disabled the payload cannot reach
 *   `window.__TAURI_INTERNALS__`, which is what makes a hostile document merely
 *   inert rather than able to call backend commands.
 * * `default-src 'none'` inside the document — no scripts, no stylesheets, no
 *   fonts and **no remote resources** from the network. A report is expected to be
 *   self-contained; blocking remote fetches removes the tracking-pixel and
 *   data-exfiltration path that an iframe otherwise leaves open.
 *
 *   Verified by ground truth rather than by reading the policy: a document
 *   pointing an `<img>` at a local server **did** show a failed request inside the
 *   frame, but the server received nothing. So the attempt is made and the policy
 *   stops it short of the network — which is why DevTools shows a `request` event
 *   with no matching `requestfinished`. Do not read that event as a leak.
 * * `referrerpolicy="no-referrer"` — nothing about this app leaks outward.
 *
 * Two consequences worth stating rather than discovering:
 *
 * * **Scripts do not run**, so a report that draws its charts with JavaScript
 *   will render as static markup. That is the trade for containment.
 * * **Remote images and stylesheets do not load.** The document must carry its own
 *   CSS and inline its images (`data:`), which is what "self-contained report"
 *   already implies.
 */
import { computed } from 'vue'

const props = defineProps<{
  html: string
}>()

/**
 * The document handed to the frame, with its own policy attached.
 *
 * The policy is prepended rather than assumed: `srcdoc` documents inherit nothing
 * useful here, and stating it in the document means it holds even if the frame is
 * ever mounted somewhere else.
 */
const srcdoc = computed(
  () =>
    `<!doctype html><meta http-equiv="Content-Security-Policy" content="` +
    `default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; ` +
    `font-src data:; base-uri 'none'; form-action 'none'">` +
    props.html,
)
</script>

<template>
  <div class="html-frame">
    <iframe
      class="html-frame-body"
      :srcdoc="srcdoc"
      sandbox=""
      referrerpolicy="no-referrer"
      title="agent 生成的 HTML 报告"
    />
  </div>
</template>
