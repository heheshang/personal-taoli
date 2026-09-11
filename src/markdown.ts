/**
 * Markdown rendering for agent output.
 *
 * The input is **untrusted**: it is whatever the model wrote, and it is rendered
 * into a webview that can reach the Rust backend. Two settings do the work of
 * sanitising it, and both are deliberate:
 *
 * - `html: false` — raw HTML in the source is escaped rather than passed
 *   through. That removes the whole class of injected `<script>`, `<img
 *   onerror>`, and `<iframe>` payloads at the source, without needing a second
 *   sanitiser pass over already-generated HTML.
 * - markdown-it's built-in link validation — `javascript:`, `vbscript:`,
 *   `file:` and most `data:` URLs are refused; a rejected link renders as plain
 *   text.
 *
 * Links are rendered but **do not navigate**. No external-opener plugin is
 * installed, so following a link inside the webview would replace the app's own
 * page with a remote document — turning a stray link in model output into a
 * broken interface. The URL is kept in the anchor's `title` so it stays
 * inspectable, and the click is suppressed in `MarkdownBlock`.
 *
 * `breaks: false` matches markdown's own rule (a single newline is not a line
 * break), which is what the model's output is written against. Tables and code
 * fences are enabled because analysis results arrive as both.
 *
 * `linkify` is **off**, which is a domain decision rather than a safety one.
 * Auto-linking turned a bare stock code into a link — `002600.SZ` became
 * `http://002600.SZ` — and this page is nothing but tickers. Explicit markdown
 * links still render; bare URLs are simply shown as text.
 */
import MarkdownIt from 'markdown-it'

const renderer = new MarkdownIt({
  html: false,
  linkify: false,
  breaks: false,
  typographer: false,
})

/**
 * Renders `source` as sanitised HTML.
 *
 * Returns `''` for empty input so callers can test truthiness rather than
 * matching against a wrapper element.
 */
export function renderMarkdown(source: string): string {
  if (!source.trim()) return ''
  return renderer.render(source)
}
