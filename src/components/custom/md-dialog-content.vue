<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { reactiveOmit } from '@vueuse/core';
import type { DialogContentEmits, DialogContentProps } from 'reka-ui';
import { DialogClose, DialogContent, DialogOverlay, DialogPortal, useForwardPropsEmits } from 'reka-ui';
import { useAttrs, type HTMLAttributes } from 'vue';

import MdIcon from '@/components/icons/md-icon.vue';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

defineOptions({ inheritAttrs: false });

const { t } = useI18n({ useScope: 'global' });
const props = withDefaults(
  defineProps<
    DialogContentProps & {
      class?: HTMLAttributes['class'];
      showClose?: boolean;
      blurBackground?: boolean;
      scaleOnOpen?: boolean;
      size?: 'compact' | 'standard' | 'large' | 'wide';
      height?: 'auto' | 'tall';
    }
  >(),
  {
    class: undefined,
    showClose: true,
    blurBackground: true,
    scaleOnOpen: true,
    size: 'standard',
    height: 'auto',
  }
);
const emits = defineEmits<DialogContentEmits>();
const attrs = useAttrs();
const delegatedProps = reactiveOmit(props, 'class', 'showClose', 'blurBackground', 'scaleOnOpen', 'size', 'height');
const forwarded = useForwardPropsEmits(delegatedProps, emits);
// Keep defaults and caller overrides in one merger so Tailwind utility ordering
// cannot override the selected dialog size or caller's layout classes.
const sizeClass = {
  compact: 'max-w-[var(--layout-dialog-compact-width)]',
  standard: 'max-w-[var(--layout-dialog-standard-width)]',
  large: 'max-w-[var(--layout-dialog-large-width)]',
  wide: 'max-w-[var(--layout-dialog-wide-width)]',
} as const;
</script>

<template>
  <DialogPortal>
    <DialogOverlay
      data-slot="dialog-overlay"
      :class="{ 'backdrop-blur-sm': props.blurBackground, 'md-dialog-overlay--plain': !props.blurBackground }"
      class="fixed inset-0 z-50 bg-background/65 data-[state=closed]:animate-out data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
    />
    <DialogContent
      data-slot="dialog-content"
      v-bind="{ ...forwarded, ...attrs }"
      :class="
        twMerge(
          clsx(
            'md-dialog-content fixed top-1/2 left-1/2 z-50 grid w-full max-w-[680px] -translate-x-1/2 -translate-y-1/2 gap-0 rounded-2xl border border-border bg-card text-card-foreground shadow-2xl shadow-foreground/15 duration-200 data-[state=closed]:animate-out data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95',
            sizeClass[props.size],
            { 'md-dialog-content--tall': props.height === 'tall', 'data-[state=open]:zoom-in-95': props.scaleOnOpen },
            props.class
          )
        )
      "
    >
      <slot />
      <DialogClose
        v-if="props.showClose"
        class="absolute top-4 right-4 z-20 grid size-9 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <MdIcon :name="ICON_NAMES.close" :size="18" />
        <span class="sr-only">{{ t('common.close') }}</span>
      </DialogClose>
    </DialogContent>
  </DialogPortal>
</template>

<style>
/*
 * DialogPortal teleports content to document.body.
 * Scoped selectors stay on this wrapper's local subtree and therefore cannot
 * constrain the teleported node. These project-prefixed classes intentionally
 * remain global so every real dialog receives the viewport safety boundary.
 */
/* A zero blur radius still retains a backdrop-filter surface in WebKit.
   Plain overlays explicitly opt out of that compositing work over large charts. */
[data-slot='dialog-overlay'].md-dialog-overlay--plain {
  -webkit-backdrop-filter: none !important;
  backdrop-filter: none !important;
}

.md-dialog-content {
  width: calc(100% - var(--layout-dialog-viewport-inset) - var(--layout-dialog-viewport-inset));
  max-height: calc(100vh - var(--layout-dialog-viewport-inset) - var(--layout-dialog-viewport-inset));
  gap: 0;
  overflow: hidden;
  padding: 0;
}

/*
 * WebKit can leave an exit animation running after the dialog is closed,
 * retaining its modal overlay and blocking the page. Release both surfaces
 * immediately on close instead of making dismissal depend on animationend.
 * Opening animations remain unchanged.
 */
.md-dialog-content[data-state='closed'],
[data-slot='dialog-overlay'][data-state='closed'] {
  animation: none;
}

.md-dialog-content--tall {
  height: min(
    var(--layout-dialog-tall-height),
    calc(100vh - var(--layout-dialog-viewport-inset) - var(--layout-dialog-viewport-inset))
  );
}
</style>
