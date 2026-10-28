<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type {
  FloatingEntry,
  FloatingItemMeasurement,
  FloatingMetricId,
  FloatingSnapshot,
} from '@/lib/models/resident/floating-monitor';
import { FloatingMonitorService } from '@/lib/services/resident/floating-monitor-service';
import { useAppStore } from '@/stores/app-store';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { FLOATING_LABEL_KEYS, detailPage, floatingValue } from '@/lib/utils/resident/floating-metric';
import '@/assets/themes/floating.css';
import { FLOATING_FONT_SCALES } from '@/lib/models/resident/floating-appearance';
import { FloatingLayoutService } from '@/lib/services/resident/floating-layout-service';

const { t, locale } = useI18n();
const appStore = useAppStore();
const snapshot = ref<FloatingSnapshot | null>(null);
const columnCount = computed(() => snapshot.value?.columns?.length || 1);
const trailingCellCount = computed(() => {
  if (snapshot.value?.preferences.layout !== 'horizontal' || !snapshot.value.entries.length) return 0;
  return (columnCount.value - (snapshot.value.entries.length % columnCount.value)) % columnCount.value;
});
const failed = ref(false);
const fontScale = computed(() => FLOATING_FONT_SCALES[snapshot.value?.preferences.fontSize ?? 'standard']);
const surface = ref<HTMLElement>();
const keyboardFocus = ref(false);
function trackKeyboardFocus(event: KeyboardEvent) {
  if (event.key === 'Tab') keyboardFocus.value = true;
}
function clearKeyboardFocus() {
  keyboardFocus.value = false;
}
function openPointerMenu() {
  clearKeyboardFocus();
  openMenu();
}
const measurements = ref<FloatingItemMeasurement[]>([]);
const slots = computed(() => new Map(measurements.value.map(item => [`${item.id}:${item.gpuDeviceId ?? ''}`, item])));
const entryCounts = computed(() => {
  const counts: Partial<Record<FloatingMetricId, number>> = {};
  for (const entry of snapshot.value?.entries ?? []) counts[entry.id] = (counts[entry.id] ?? 0) + 1;
  return counts;
});
let disposed = false;
const unlisteners: (() => void)[] = [];
async function perform(action: () => Promise<void>) {
  try {
    await action();
    failed.value = false;
  } catch (error) {
    failed.value = true;
    LoggerService.warn('resident', 'floating_interaction_failed', { error });
  }
}
function accept(value: FloatingSnapshot | null) {
  if (!disposed && value?.schemaVersion === 12) {
    snapshot.value = value;
    if (sizingFailed || sizedKey !== sizingKey.value) queueSizing();
  }
}
const sizingKey = computed(() =>
  JSON.stringify([
    locale.value,
    snapshot.value?.preferences.fontSize,
    snapshot.value?.preferences.bold,
    snapshot.value?.entries.map(entry => [
      entry.id,
      entry.gpuDevice?.id,
      metricLabel(entry),
      !!entry.fanRows?.length,
      entry.frequency?.idleLabel,
    ]),
  ])
);
let pendingSizing = false;
let sizing = false;
let sizingFailed = false;
let fallbackFailed = false;
let sizedKey = '';
let lastMeasurement = '';
function queueSizing() {
  pendingSizing = true;
  void sizeSurface();
}
watch(sizingKey, queueSizing, { flush: 'post' });
async function sizeSurface() {
  if (sizing || !snapshot.value || disposed) return;
  sizing = true;
  try {
    while (pendingSizing && !disposed) {
      pendingSizing = false;
      try {
        await nextTick();
        await document.fonts?.ready;
        if (disposed || !surface.value || !snapshot.value) return;
        const key = sizingKey.value;
        const items = FloatingLayoutService.measure(
          surface.value,
          snapshot.value.entries.map(entry => ({ entry, label: metricLabel(entry) }))
        );
        const typography = { fontSize: snapshot.value.preferences.fontSize, bold: snapshot.value.preferences.bold };
        const serialized = JSON.stringify([typography, items]);
        if (serialized !== lastMeasurement || sizingFailed) {
          const accepted = await FloatingMonitorService.layout(items, typography);
          if (disposed) return;
          if (!accepted) continue;
          measurements.value = items;
          lastMeasurement = serialized;
        }
        sizedKey = key;
        if (sizingFailed) {
          failed.value = false;
          LoggerService.info('resident', 'floating_layout_measurement_recovered', { entries: items.length });
        }
        sizingFailed = false;
        fallbackFailed = false;
      } catch (error) {
        if (disposed) return;
        if (!sizingFailed)
          LoggerService.warn('resident', 'floating_layout_measurement_failed', { error, outcome: 'fixed_cells' });
        sizingFailed = true;
        failed.value = true;
        await FloatingMonitorService.layout(null, {
          fontSize: snapshot.value!.preferences.fontSize,
          bold: snapshot.value!.preferences.bold,
        }).catch(fallbackError => {
          if (!fallbackFailed)
            LoggerService.warn('resident', 'floating_layout_fallback_failed', { error: fallbackError });
          fallbackFailed = true;
        });
      }
    }
  } finally {
    sizing = false;
  }
}
function refreshFontSizing() {
  // Native resizing may change DPI; identical measurements never trigger another resize.
  queueSizing();
}
function metricStyle(entry: FloatingEntry) {
  const item = slots.value.get(`${entry.id}:${entry.gpuDevice?.id ?? ''}`);
  return {
    '--floating-value-width': item ? `${item.valueWidth}px` : undefined,
  };
}
onMounted(async () => {
  window.addEventListener('resize', refreshFontSizing);
  window.addEventListener('keydown', trackKeyboardFocus, true);
  document.fonts?.addEventListener('loadingdone', refreshFontSizing);
  try {
    let received = false;
    const stop = await FloatingMonitorService.onSnapshot(value => {
      received = true;
      accept(value);
    });
    if (disposed) {
      stop();
      return;
    }
    unlisteners.push(stop);
    const appearanceStop = await FloatingMonitorService.onAppearanceChanged(() => {
      if (!disposed) void appStore.loadSettings();
    });
    if (disposed) {
      appearanceStop();
      return;
    }
    unlisteners.push(appearanceStop);
    const initial = await FloatingMonitorService.ready();
    if (!received) accept(initial);
  } catch (error) {
    failed.value = true;
    LoggerService.warn('resident', 'floating_initialization_failed', { error });
  }
});
onBeforeUnmount(() => {
  disposed = true;
  window.removeEventListener('resize', refreshFontSizing);
  window.removeEventListener('keydown', trackKeyboardFocus, true);
  document.fonts?.removeEventListener('loadingdone', refreshFontSizing);
  unlisteners.forEach(stop => stop());
});
function openMenu() {
  if (!snapshot.value?.contextMenu) return;
  void perform(() => FloatingMonitorService.menu());
}
function metricLabel(entry: FloatingEntry) {
  if (entry.gpuDevice && (entryCounts.value[entry.id] ?? 0) > 1) return entry.gpuDevice.label;
  if (entry.id === 'fanSpeed' && !snapshot.value?.interactive) return t('floatingMonitor.labels.fan');
  const label = t(FLOATING_LABEL_KEYS[entry.id]);
  return entry.id === 'gpuPower' && entry.gpuDevice && entry.gpuDevice.label !== 'GPU'
    ? `${label} · ${entry.gpuDevice.label}`
    : label;
}
function toggleDetail(id: FloatingMetricId) {
  if (!snapshot.value?.interactive) return;
  void perform(() => FloatingMonitorService.togglePage(detailPage(id)));
}
let dragOrigin: { x: number; y: number; clientX: number; clientY: number } | null = null;
let dragged = false;
function prepareMove(event: PointerEvent) {
  clearKeyboardFocus();
  dragged = false;
  dragOrigin =
    event.button === 0 && !snapshot.value?.preferences.locked
      ? { x: event.screenX, y: event.screenY, clientX: event.clientX, clientY: event.clientY }
      : null;
  if (dragOrigin) {
    const target = (event.target as Element).closest('button') ?? (event.currentTarget as Element);
    target.setPointerCapture(event.pointerId);
  }
}
function move(event: PointerEvent) {
  if (!dragOrigin || snapshot.value?.preferences.locked) return;
  if (!(event.buttons & 1)) {
    finishMove();
    return;
  }
  if (Math.hypot(event.screenX - dragOrigin.x, event.screenY - dragOrigin.y) < 4) return;
  LoggerService.info('resident', 'floating_drag_requested', {
    origin: dragOrigin,
    screenX: event.screenX,
    screenY: event.screenY,
    clientX: event.clientX,
    clientY: event.clientY,
    pointerType: event.pointerType,
  });
  finishMove();
  dragged = true;
  event.preventDefault();
  void perform(() => FloatingMonitorService.drag());
}
function finishMove() {
  dragOrigin = null;
}
function suppressDraggedClick(event: MouseEvent) {
  if (!dragged) return;
  dragged = false;
  if (event.detail === 0) return;
  event.preventDefault();
  event.stopPropagation();
}
</script>

<template>
  <main
    ref="surface"
    class="floating-monitor"
    :class="{ vertical: snapshot?.preferences.layout === 'vertical', locked: snapshot?.preferences.locked }"
    :data-interactive="snapshot?.interactive ?? false"
    :data-keyboard-focus="keyboardFocus"
    :data-background="snapshot?.preferences.background ?? 'default'"
    :data-bold="snapshot?.preferences.bold ?? false"
    :data-dividers="snapshot?.preferences.dividers ?? true"
    :style="{
      '--floating-font-scale': fontScale,
      '--floating-label-weight': snapshot?.preferences.bold ? 600 : 400,
      '--floating-value-weight': snapshot?.preferences.bold ? 700 : 500,
      '--floating-background-opacity': `${snapshot?.preferences.backgroundOpacity ?? 100}%`,
      '--floating-columns': snapshot?.columns?.map(width => `${width}px`).join(' '),
    }"
    :aria-label="t('floatingMonitor.title')"
    :tabindex="snapshot?.contextMenu ? 0 : undefined"
    @pointerdown="prepareMove"
    @pointermove="move"
    @pointerup="finishMove"
    @pointercancel="finishMove"
    @pointerleave="finishMove"
    @click.capture="suppressDraggedClick"
    @dragstart.prevent
    @contextmenu.prevent="openPointerMenu()"
    @keydown.shift.f10.prevent="openMenu()"
  >
    <div v-if="snapshot?.entries.length" class="metrics">
      <component
        :is="snapshot.interactive ? 'button' : 'div'"
        v-for="(entry, index) in snapshot.entries"
        :key="`${entry.id}:${entry.gpuDevice?.id ?? ''}`"
        class="metric"
        :class="{
          'column-divider': index % columnCount > 0,
          'row-divider': index >= (snapshot.preferences.layout === 'vertical' ? 1 : columnCount),
        }"
        :data-metric="entry.id"
        :style="metricStyle(entry)"
        :aria-label="entry.tooltip"
        @click="toggleDetail(entry.id)"
      >
        <span class="metric-label">{{ metricLabel(entry) }}</span>
        <span v-if="entry.fanRows" class="fan-values" :data-tone="entry.tone">
          <span v-for="fan in entry.fanRows" :key="fan.id" :data-tone="fan.tone">{{ fan.text }}</span>
        </span>
        <span v-else class="metric-value" :data-tone="entry.tone">
          {{ floatingValue(entry) }}
        </span>
      </component>
      <div
        v-for="index in trailingCellCount"
        :key="`empty-${index}`"
        class="metric empty-cell column-divider"
        :class="{ 'row-divider': snapshot.entries.length + index - 1 >= columnCount }"
        aria-hidden="true"
      />
    </div>
    <component
      :is="snapshot?.interactive ? 'button' : 'div'"
      v-else
      class="empty-state"
      @click="snapshot?.interactive && openMenu()"
    >
      {{ t('floatingMonitor.chooseMetrics') }}
    </component>
    <span v-if="failed" role="alert" class="error-indicator" :aria-label="t('monitoring.unavailable')">!</span>
  </main>
</template>

<style scoped>
@reference "@assets/main.css";
:global(html.floating-monitor-document),
:global(html.floating-monitor-document body),
:global(html.floating-monitor-document #app) {
  min-width: 0;
  overflow: hidden;
  background: transparent;
}
.floating-monitor {
  height: 100dvh;
}
.floating-monitor:not(.locked),
.floating-monitor:not(.locked) .metric {
  touch-action: none;
}
.floating-monitor:focus-visible,
button:focus-visible {
  outline: none;
}
/* WebKit may retain focus-visible when a native menu returns pointer focus. */
.floating-monitor[data-keyboard-focus='true']:focus-visible,
.floating-monitor[data-keyboard-focus='true'] button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: -2px;
}
.error-indicator {
  position: absolute;
  top: 0;
  right: 1px;
  color: var(--destructive-text);
  font-size: calc(10px * var(--floating-font-scale));
  pointer-events: none;
}
.empty-state {
  @apply text-muted-foreground hover:bg-accent;
  flex: 1;
  min-width: 0;
  padding: 0 2px;
  font-size: calc(10px * var(--floating-font-scale));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: inherit;
}
</style>
