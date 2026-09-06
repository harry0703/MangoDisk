<script setup lang="ts">
import { nextTick, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdSpinner from '@/components/custom/md-spinner.vue';
import { ICON_NAMES } from '@/lib/models/ui';

const props = defineProps<{
  open: boolean;
  minimized: boolean;
  title: string;
  subtitle?: string;
  busy?: boolean;
  statusLabel?: string;
}>();
const emit = defineEmits<{ minimize: []; restore: []; close: [] }>();
const { t } = useI18n();
const panel = ref<HTMLElement | null>(null);
const launcher = ref<HTMLElement | null>(null);
const panelId = useId();
watch(
  () => props.open && !props.minimized,
  async expanded => {
    await nextTick();
    if (expanded !== (props.open && !props.minimized)) return;
    if (expanded) panel.value?.focus({ preventScroll: true });
    else if (props.open) launcher.value?.focus({ preventScroll: true });
  }
);
</script>

<template>
  <div class="floating-panel-anchor">
    <Transition name="floating-launcher-motion" appear>
      <button
        v-show="open && minimized"
        ref="launcher"
        :inert="!open || !minimized || undefined"
        :aria-hidden="!open || !minimized"
        class="floating-panel-launcher"
        :aria-label="t('ai.restore')"
        :aria-controls="panelId"
        :aria-expanded="false"
        @click="emit('restore')"
      >
        <MdSpinner v-if="busy" /><MdIcon v-else :name="ICON_NAMES.sparkles" :size="19" />
        <span class="min-w-0 text-left"
          ><strong class="block text-sm">{{ title }}</strong
          ><small class="block max-w-48 truncate text-xs text-muted-foreground">{{ subtitle }}</small></span
        >
        <span v-if="statusLabel" role="status" class="sr-only">{{ statusLabel }}</span>
        <MdIcon :name="ICON_NAMES.chevronUp" :size="16" />
      </button>
    </Transition>
    <!-- This non-modal region never locks scrolling or traps focus. -->
    <Transition name="floating-panel-motion" appear>
      <section
        v-show="open && !minimized"
        :id="panelId"
        ref="panel"
        :inert="!open || minimized || undefined"
        :aria-hidden="!open || minimized"
        class="floating-panel"
        role="region"
        :aria-label="title"
        tabindex="-1"
        @keydown.esc.stop="emit('minimize')"
      >
        <header class="flex flex-none items-center gap-2 border-b border-border/70 px-4 py-3">
          <MdIcon :name="ICON_NAMES.sparkles" :size="19" />
          <div class="min-w-0 flex-1">
            <h2 class="text-base font-semibold">{{ title }}</h2>
            <p v-if="subtitle" class="truncate text-xs text-muted-foreground" :title="subtitle">{{ subtitle }}</p>
          </div>
          <slot name="actions" />
          <MdIconAction variant="ghost" :label="t('ai.minimize')" @click="emit('minimize')"
            ><MdIcon :name="ICON_NAMES.minus" :size="17"
          /></MdIconAction>
          <MdIconAction variant="ghost" :label="t('common.close')" @click="emit('close')"
            ><MdIcon :name="ICON_NAMES.close" :size="17"
          /></MdIconAction>
        </header>
        <slot />
        <footer
          v-if="$slots.footer"
          class="flex flex-none items-center justify-between gap-3 border-t border-border/70 px-4 py-3"
        >
          <slot name="footer" />
        </footer>
      </section>
    </Transition>
  </div>
</template>

<style scoped>
@reference "@assets/main.css";
.floating-panel-anchor {
  position: absolute;
  top: 12px;
  right: 0;
  bottom: var(--page-content-bottom-inset, 12px);
  z-index: 40;
  width: 100%;
  max-width: var(--layout-dialog-compact-width);
  pointer-events: none;
}
.floating-panel,
.floating-panel-launcher {
  position: absolute;
  right: 0;
  bottom: 0;
  transform-origin: bottom right;
  pointer-events: auto;
  @apply border border-border bg-background shadow-xl;
}
.floating-panel {
  width: 100%;
  display: flex;
  min-height: 0;
  height: min(480px, 100%);
  flex-direction: column;
  overflow: hidden;
  border-radius: var(--radius-lg);
  outline: none;
}
.floating-panel-launcher {
  display: flex;
  max-width: 100%;
  align-items: center;
  gap: 12px;
  border-radius: var(--radius-lg);
  padding: 12px 16px;
  @apply transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
}
/* Unfold vertically from the bottom edge instead of fading a full-size window.
   Perspective narrows the base during travel, suggesting a dock-style funnel.
   Keep the response's layout size unchanged and the bottom edge fixed so the
   motion cannot reflow streamed text or travel across the page action bar.
   Transitions (rather than keyframes) also reverse from their current position. */
.floating-panel-motion-enter-active {
  transition:
    opacity 90ms ease-out,
    transform 440ms cubic-bezier(0.2, 0.7, 0.2, 1);
}
.floating-panel-motion-leave-active {
  transition:
    opacity 100ms ease-in 260ms,
    transform 360ms cubic-bezier(0.4, 0, 0.6, 1);
}
.floating-panel-motion-enter-from,
.floating-panel-motion-leave-to {
  opacity: 0;
  transform: perspective(800px) rotateX(-24deg) scale(0.58, 0.02);
}
.floating-launcher-motion-enter-active {
  transition:
    opacity 120ms ease-out 220ms,
    transform 160ms ease-out 180ms;
}
.floating-launcher-motion-leave-active {
  transition:
    opacity 90ms ease-out,
    transform 160ms ease-out;
}
.floating-launcher-motion-enter-from,
.floating-launcher-motion-leave-to {
  opacity: 0;
  transform: scale(0.96, 0.8);
}
.floating-panel[inert],
.floating-panel-launcher[inert] {
  pointer-events: none;
}
@media (prefers-reduced-motion: reduce) {
  .floating-panel-motion-enter-active,
  .floating-panel-motion-leave-active,
  .floating-launcher-motion-enter-active,
  .floating-launcher-motion-leave-active {
    transition: none;
  }
  .floating-panel-motion-enter-from,
  .floating-panel-motion-leave-to,
  .floating-launcher-motion-enter-from,
  .floating-launcher-motion-leave-to {
    transform: none;
  }
}
</style>
