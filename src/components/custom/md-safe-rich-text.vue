<script setup lang="ts">
import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { computed, onMounted } from 'vue';

import { normalizeExternalUrl } from '@/lib/utils/external-url';
import { LoggerService } from '@/lib/services/logger-service';

const props = withDefaults(
  defineProps<{
    content: string;
    variant?: 'default' | 'answer';
    allowLinks?: boolean;
  }>(),
  { variant: 'default', allowLinks: true }
);
const emit = defineEmits<{
  openLink: [url: string];
}>();

const SAFE_FORMATTING_TAGS = [
  'a',
  'blockquote',
  'br',
  'code',
  'del',
  'em',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'hr',
  'li',
  'ol',
  'p',
  'pre',
  'strong',
  'table',
  'tbody',
  'td',
  'th',
  'thead',
  'tr',
  'ul',
];

const html = computed(() => {
  // Unsupported DOMs make DOMPurify return input unchanged. Fail closed to
  // Vue text interpolation instead of trusting that compatibility fallback.
  if (!DOMPurify.isSupported) return null;
  const rendered = marked.parse(props.content, {
    async: false,
    breaks: true,
    gfm: true,
  }) as string;

  // Both release notes and model output are untrusted. Re-sanitize every
  // partial stream: incomplete Markdown/HTML must never bypass this boundary.
  // AI links remain readable labels, not navigable or executable actions.
  return DOMPurify.sanitize(rendered, {
    ALLOWED_ATTR: props.allowLinks ? ['href', 'title'] : [],
    ALLOWED_TAGS: props.allowLinks ? SAFE_FORMATTING_TAGS : SAFE_FORMATTING_TAGS.filter(tag => tag !== 'a'),
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto):)/iu,
  });
});

onMounted(() => {
  if (!DOMPurify.isSupported) LoggerService.warn('rich_text', 'sanitizer_unavailable');
});

function openLink(event: MouseEvent) {
  const target = event.target;
  if (!(target instanceof Element)) return;

  const anchor = target.closest('a');
  if (!anchor || !event.currentTarget || !(event.currentTarget instanceof Element)) return;
  if (!event.currentTarget.contains(anchor)) return;

  event.preventDefault();
  const url = normalizeExternalUrl(anchor.getAttribute('href') ?? '');
  if (url) emit('openLink', url);
}
</script>

<template>
  <div class="md-safe-rich-text" :class="{ 'md-safe-rich-text-answer': variant === 'answer' }" @click="openLink">
    <div v-if="html === null" class="whitespace-pre-wrap">{{ content }}</div>
    <!-- eslint-disable-next-line vue/no-v-html -- content is sanitized above with an explicit allowlist -->
    <div v-else v-html="html" />
  </div>
</template>

<style scoped>
@reference "@assets/main.css";

.md-safe-rich-text {
  overflow-wrap: anywhere;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  line-height: 1.6;
}

.md-safe-rich-text :deep(:first-child) {
  margin-top: 0;
}

.md-safe-rich-text :deep(:last-child) {
  margin-bottom: 0;
}

.md-safe-rich-text :deep(p),
.md-safe-rich-text :deep(ul),
.md-safe-rich-text :deep(ol),
.md-safe-rich-text :deep(blockquote),
.md-safe-rich-text :deep(pre),
.md-safe-rich-text :deep(table) {
  margin: 0.55em 0;
}

.md-safe-rich-text :deep(ul),
.md-safe-rich-text :deep(ol) {
  padding-inline-start: 1.4em;
}

.md-safe-rich-text :deep(ul) {
  list-style: disc;
}

.md-safe-rich-text :deep(ol) {
  list-style: decimal;
}

.md-safe-rich-text :deep(h1),
.md-safe-rich-text :deep(h2),
.md-safe-rich-text :deep(h3),
.md-safe-rich-text :deep(h4),
.md-safe-rich-text :deep(h5),
.md-safe-rich-text :deep(h6) {
  margin: 0.8em 0 0.35em;
  @apply text-foreground;
  font-size: var(--font-content-body);
  font-weight: 600;
  line-height: 1.35;
}

.md-safe-rich-text :deep(a) {
  cursor: pointer;
  text-decoration-line: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 2px;
  @apply text-primary;
}

.md-safe-rich-text :deep(a:hover) {
  @apply text-primary/80;
}

.md-safe-rich-text :deep(blockquote) {
  border-inline-start-width: 3px;
  padding-inline-start: 0.8em;
  @apply border-border;
}

.md-safe-rich-text :deep(code) {
  border-radius: 4px;
  padding: 0.1em 0.3em;
  @apply bg-background text-foreground;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 0.92em;
}

.md-safe-rich-text :deep(pre) {
  overflow-x: auto;
  border-radius: 7px;
  padding: 0.75em;
  @apply bg-background;
}

.md-safe-rich-text :deep(pre code) {
  padding: 0;
  background: transparent;
}

.md-safe-rich-text :deep(table) {
  width: 100%;
  border-collapse: collapse;
}

.md-safe-rich-text :deep(th),
.md-safe-rich-text :deep(td) {
  border-width: 1px;
  padding: 0.35em 0.5em;
  text-align: start;
  @apply border-border;
}

.md-safe-rich-text :deep(hr) {
  margin: 0.75em 0;
  border-top-width: 1px;
  @apply border-border;
}

/* The same renderer serves a compact answer without changing release-note typography. */
.md-safe-rich-text-answer {
  min-width: 0;
  -webkit-user-select: text;
  user-select: text;
  cursor: text;
  @apply text-sm text-foreground;
  line-height: 1.85;
}

/* App chrome disables selection on every descendant, not just the root. */
.md-safe-rich-text-answer :deep(*) {
  -webkit-user-select: text;
  user-select: text;
  cursor: text;
}

.md-safe-rich-text-answer :deep(strong) {
  font-weight: 600;
}

.md-safe-rich-text-answer :deep(li + li) {
  margin-top: 0.4em;
}

.md-safe-rich-text-answer :deep(pre),
.md-safe-rich-text-answer :deep(table) {
  max-width: 100%;
  overflow-x: auto;
}

.md-safe-rich-text-answer :deep(table) {
  display: block;
}
</style>
