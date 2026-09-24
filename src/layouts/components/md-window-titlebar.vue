<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { onBeforeUnmount, onMounted, ref } from 'vue';

import MdIconAction from '@/components/custom/md-icon-action.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdIconMangodisk from '@/components/icons/md-icon-mangodisk.vue';
import { APP_NAME } from '@/lib/models/application-shell';
import { ICON_NAMES } from '@/lib/models/ui';
import { ApplicationWindowService } from '@/lib/services/application-window-service';

const props = defineProps<{
  platform: 'linux' | 'macos' | 'windows';
  sidebarExpanded?: boolean;
}>();

const { t } = useI18n({ useScope: 'global' });
const maximized = ref(false);
let stopObservingMaximized: (() => void) | undefined;
let disposed = false;

onMounted(async () => {
  if (props.platform === 'macos') return;
  const stop = await ApplicationWindowService.observeMaximized(value => {
    maximized.value = value;
  });
  if (disposed) stop();
  else stopObservingMaximized = stop;
});

onBeforeUnmount(() => {
  disposed = true;
  stopObservingMaximized?.();
});

function toggleMaximize() {
  if (props.platform !== 'macos') void ApplicationWindowService.toggleMaximize();
}

function minimize() {
  void ApplicationWindowService.minimize();
}

function close() {
  void ApplicationWindowService.close();
}
</script>

<template>
  <header data-tauri-drag-region class="window-titlebar" :class="`window-titlebar--${platform}`">
    <!--
      Tauri owns double-click handling for drag regions on Windows and Linux.
      Adding a Vue dblclick handler here would toggle the native window twice,
      leaving maximize and restore behavior out of sync.
    -->
    <div
      v-if="platform !== 'macos'"
      data-tauri-drag-region
      class="window-title"
      :class="{ 'window-title--expanded': sidebarExpanded }"
    >
      <span data-tauri-drag-region class="window-title-icon">
        <MdIconMangodisk :size="28" />
      </span>
      <strong data-tauri-drag-region :aria-hidden="!sidebarExpanded">{{ APP_NAME }}</strong>
    </div>

    <div v-if="platform !== 'macos'" class="window-controls" @dblclick.stop>
      <MdIconAction
        appearance="unstyled"
        class="window-control"
        :label="t('common.minimize')"
        :show-tooltip="false"
        @click="minimize"
      >
        <MdIcon :name="ICON_NAMES.minus" :size="18" :stroke-width="1.7" />
      </MdIconAction>
      <MdIconAction
        appearance="unstyled"
        class="window-control"
        :label="t(maximized ? 'common.restore' : 'common.maximize')"
        :show-tooltip="false"
        @click="toggleMaximize"
      >
        <MdIcon
          :name="maximized ? ICON_NAMES.windowRestore : ICON_NAMES.windowMaximize"
          :size="15"
          :stroke-width="1.6"
        />
      </MdIconAction>
      <MdIconAction
        appearance="unstyled"
        class="window-control window-control--close"
        :label="t('common.close')"
        :show-tooltip="false"
        @click="close"
      >
        <MdIcon :name="ICON_NAMES.close" :size="18" :stroke-width="1.7" />
      </MdIconAction>
    </div>

    <!--
      The content pane starts at the top edge on macOS. Its top padding is an
      intentionally empty strip, so it can restore window dragging without
      covering page-header actions.
    -->
    <div v-if="platform === 'macos'" data-tauri-drag-region class="window-content-drag-region" />
  </header>
</template>

<style scoped>
@reference "@assets/main.css";

.window-titlebar {
  position: fixed;
  z-index: 30;
  top: 0;
  right: 0;
  left: 0;
  height: var(--titlebar-height);
  @apply bg-sidebar;
  user-select: none;
}

.window-titlebar--linux,
.window-titlebar--windows {
  --window-control-visual-bottom-gap: 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: transparent;
  pointer-events: none;
}

/*
 * The macOS overlay only keeps the traffic-light area draggable. Restricting
 * it to the sidebar prevents the transparent layer from covering page-header
 * controls after the content pane extends to the top edge.
 */
.window-titlebar--macos {
  right: auto;
  width: var(--sidebar-width);
  transition: width var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}

.window-content-drag-region {
  position: fixed;
  top: 0;
  right: 0;
  left: var(--sidebar-width);
  height: var(--layout-page-padding-top);
  transition: left var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}

.window-title {
  display: flex;
  width: var(--sidebar-width);
  height: 100%;
  min-width: 0;
  align-items: center;
  justify-content: flex-start;
  gap: 0;
  padding: 0 20px;
  color: var(--sidebar-foreground);
  pointer-events: auto;
  transition:
    width var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    gap var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease);
}

.window-title--expanded {
  gap: 10px;
}

.window-title-icon {
  display: grid;
  width: 28px;
  height: 28px;
  flex: none;
  overflow: hidden;
  place-items: center;
  border-radius: 7px;
}

.window-title strong {
  max-width: 0;
  overflow: hidden;
  opacity: 0;
  font-size: 14px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
  visibility: hidden;
  transform: translateX(-4px);
  transition:
    max-width var(--sidebar-transition-duration, 240ms) var(--sidebar-transition-easing, ease),
    opacity 100ms ease,
    transform 180ms ease,
    visibility 0s linear var(--sidebar-transition-duration, 240ms);
}

.window-title--expanded strong {
  max-width: 150px;
  opacity: 1;
  visibility: visible;
  transform: translateX(0);
  transition-delay: 0s, 60ms, 60ms, 0s;
}

.window-controls {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  display: flex;
  flex: none;
  pointer-events: auto;
}

.window-controls :deep(.window-control) {
  position: relative;
  display: grid;
  width: 48px;
  height: 100%;
  place-items: center;
  border: 0;
  background: transparent;
  color: var(--sidebar-foreground);
  cursor: default;
  transition: color 0.15s ease;
}

/* Keep the full Windows hit target while separating its visual states from page content. */
.window-controls :deep(.window-control::before) {
  position: absolute;
  inset: 0 0 var(--window-control-visual-bottom-gap);
  background: transparent;
  pointer-events: none;
  content: '';
  transition: background-color 0.15s ease;
}

.window-controls :deep(.window-control > svg) {
  position: relative;
  z-index: 1;
}

.window-controls :deep(.window-control:hover) {
  color: var(--foreground);
}

.window-controls :deep(.window-control:hover::before) {
  background: var(--surface-muted-subtle);
  background: color-mix(in oklab, var(--foreground) 9%, transparent);
}

.window-controls :deep(.window-control:focus-visible) {
  outline: none;
}

.window-controls :deep(.window-control:focus-visible::before) {
  outline: 2px solid var(--ring);
  outline-offset: -3px;
}

.window-controls :deep(.window-control--close:hover) {
  @apply text-destructive-foreground;
}

.window-controls :deep(.window-control--close:hover::before) {
  @apply bg-destructive;
}
</style>
