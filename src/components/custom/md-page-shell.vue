<script setup lang="ts">
import { OperatingSystemService } from '@/lib/services/operating-system-service';

withDefaults(
  defineProps<{
    title: string;
    subtitle?: string;
    contentMode?: 'document' | 'workspace';
    contentWidth?: 'fluid' | 'readable' | 'wide';
  }>(),
  {
    subtitle: undefined,
    contentMode: 'document',
    contentWidth: 'fluid',
  }
);

const isWindows = OperatingSystemService.isWindows();
// Tauri drag regions do not inherit through child elements. Page titles and
// their surrounding whitespace are marked explicitly so the integrated
// platform chrome remains draggable while the actions column stays interactive.
const windowDragRegion = OperatingSystemService.isMacOs() || isWindows ? '' : undefined;
</script>

<template>
  <section
    class="md-page-shell"
    :class="[
      `md-page-shell--${contentMode}`,
      `md-page-shell--${contentWidth}`,
      { 'md-page-shell--windows': isWindows },
    ]"
  >
    <header
      :data-tauri-drag-region="windowDragRegion"
      class="md-page-header"
      :class="{ 'md-page-header--draggable': windowDragRegion !== undefined }"
    >
      <div :data-tauri-drag-region="windowDragRegion" class="md-page-heading">
        <h1
          :data-tauri-drag-region="windowDragRegion"
          class="m-0 leading-tight font-normal tracking-tight text-foreground"
        >
          {{ title }}
        </h1>
        <slot name="subtitle"
          ><p
            v-if="subtitle"
            :data-tauri-drag-region="windowDragRegion"
            class="mt-1.5 mb-0 text-sm leading-relaxed text-muted-foreground"
          >
            {{ subtitle }}
          </p></slot
        >
      </div>
      <div v-if="$slots.actions" class="md-page-actions"><slot name="actions" /></div>
    </header>
    <div class="md-page-content-stage" :class="{ 'md-page-content-stage--with-footer': $slots.footer }">
      <div
        class="md-page-content"
        :class="[
          `md-page-content--${contentMode}`,
          {
            'scrollbar-stable-end': contentMode === 'document',
          },
        ]"
      >
        <slot />
      </div>
      <!-- Floating helpers share content bounds, never the page CTA footer. -->
      <slot name="overlay" />
    </div>
    <footer v-if="$slots.footer" class="md-page-footer"><slot name="footer" /></footer>
  </section>
</template>

<style scoped>
@reference "@assets/main.css";
.md-page-shell {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  flex-direction: column;
  overflow: hidden;
  container-type: inline-size;
  padding: 0 var(--layout-page-padding-inline);
}

.md-page-header {
  display: grid;
  width: 100%;
  min-height: var(--layout-page-header-height);
  grid-template-columns: minmax(0, 1fr) minmax(0, auto);
  align-items: center;
  gap: 14px;
  flex: none;
}

/*
 * Document and list pages opt into named content widths through the shell.
 * Keeping the scroll container full width leaves its scrollbar at the window
 * edge, while direct page content and the title remain aligned.
 */
.md-page-shell--readable .md-page-header,
.md-page-shell--readable .md-page-footer,
.md-page-shell--readable .md-page-content :deep(> *) {
  width: 100%;
  max-width: var(--layout-page-readable-width);
  margin-inline: auto;
}

.md-page-shell--wide .md-page-header,
.md-page-shell--wide .md-page-footer,
.md-page-shell--wide .md-page-content :deep(> *) {
  width: 100%;
  max-width: var(--layout-page-wide-width);
  margin-inline: auto;
}

.md-page-header--draggable .md-page-heading {
  user-select: none;
}

/* Windows only reserves the native control area; vertical header geometry is shared across platforms. */
.md-page-shell--windows .md-page-header {
  padding-inline-end: calc(var(--window-controls-width) + 12px);
}

.md-page-shell--windows.md-page-shell--document .md-page-header {
  padding-inline-end: calc(var(--window-controls-width) + var(--layout-scrollbar-width) + 12px);
}

.md-page-shell--document .md-page-header {
  padding-inline-end: var(--layout-scrollbar-width);
}

.md-page-heading {
  display: flex;
  min-width: 0;
  flex-direction: column;
  justify-content: center;
}

.md-page-heading h1 {
  overflow: hidden;
  font-size: 22px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.md-page-actions {
  display: flex;
  width: auto;
  min-width: 0;
  min-height: 36px;
  flex: none;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  white-space: nowrap;
}

/* Toolbar actions stay anchored in the titlebar and share its subtle control border. */
.md-page-actions :deep([data-slot='button']) {
  @apply border-border/70;
  box-shadow: none;
  transform: none;
}

.md-page-actions :deep([data-slot='button']:hover) {
  @apply border-border;
  box-shadow: none;
  transform: none;
}

.md-page-actions :deep([data-slot='button']:active) {
  box-shadow: none;
  transform: none;
}

.md-page-actions :deep([data-slot='button']:focus-visible) {
  @apply border-ring;
}

/*
 * The header stays outside the scroll container. Avoiding nested sticky
 * regions prevents layout movement in lists and treemaps.
 */
.md-page-content {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
  flex-direction: column;
  gap: 8px;
  overflow-x: hidden;
  overscroll-behavior: contain;
}

.md-page-content-stage {
  /* Content and overlays share the same lower edge, including page spacing. */
  --page-content-bottom-inset: 20px;
  position: relative;
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
}

.md-page-shell--workspace .md-page-content-stage {
  --page-content-bottom-inset: 12px;
}

.md-page-shell--workspace .md-page-content-stage--with-footer {
  --page-content-bottom-inset: 0px;
}

.md-page-content--document {
  /*
   * Keep document content aligned with the page header while extending only
   * the scroll container to the pane edge. This places the scrollbar beside
   * the window edge without changing card widths or workspace-page geometry.
   */
  margin-inline-end: calc(-1 * var(--layout-page-padding-inline));
  padding-inline-end: var(--layout-page-padding-inline);
  padding-bottom: var(--page-content-bottom-inset);
}

.md-page-content--workspace {
  /* Reserve bottom spacing unless a page footer provides it. */
  overflow-y: hidden;
  padding-bottom: var(--page-content-bottom-inset);
}

.md-page-footer {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
  margin-top: 6px;
  padding-bottom: 12px;
}
</style>
