<script setup lang="ts">
import { MemoryReleaseService } from '@/lib/services/memory-release-service';
import { useMemoryReleaseStore } from '@/stores/memory-release-store';
import { METRIC_STATUS_KEYS } from '@/lib/models/system-resources';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import MdIcon from '@/components/icons/md-icon.vue';
import MdUpdateNotice from './components/md-update-notice.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { type MetricId } from '@/lib/models/system-resources';
import MdResourceOverview from './components/md-resource-overview.vue';
import MdMemoryOverview from './components/md-memory-overview.vue';
import MdApplicationMemoryList from './components/md-application-memory-list.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import type { ResidentDestination } from '@/lib/models/resident';
import { ResidentService } from '@/lib/services/resident-service';
import { useTrayPanelStore } from '@/stores/tray-panel-store';
import { useAppStore } from '@/stores/app-store';

const { t } = useI18n({ useScope: 'global' });
const store = useTrayPanelStore();
const appStore = useAppStore();
const memorySettings = useMemoryReleaseStore();
const automaticReleaseRule = computed(() => {
  const preferences = memorySettings.preferences;
  if (!preferences?.automatic) return '';
  const rules = [
    t(preferences.thresholdPercent > 0 ? 'memoryRelease.autoRuleThreshold' : 'memoryRelease.autoRuleAny', {
      minutes: preferences.intervalMinutes,
      percent: preferences.thresholdPercent,
    }),
  ];
  if (OperatingSystemService.isWindows()) {
    if (preferences.skipForeground) rules.push(t('memoryRelease.skipForeground'));
    if (preferences.exclusions.length) {
      rules.push(t('memoryRelease.autoRuleExclusions', { count: preferences.exclusions.length }));
    }
  }
  return rules.join(' · ');
});
const panel = ref<HTMLElement | null>(null);
// Native popup hiding does not consistently update document.hidden in WebView2.
// A prewarmed, unfocused panel must not start chart animation loops.
const panelFocused = ref(false);
// The native metric remains the entry context; only memory opens a dedicated page.
const selectedTab = computed(() => (store.selectedMetric === 'memory' ? 'memory' : 'overview'));
const tabs = ['overview', 'memory'] as const;
// Group activity trends before capacity readings without changing native display order.
const overviewMetrics = ['cpu', 'memory', 'disk', 'network'] as const;
// Feedback belongs to this panel's presentation lifecycle. Start its timeout only
// after loading ends; retrying or unmounting cancels the previous result's timer.
watch(
  [() => store.releasing, () => store.releaseResult],
  ([releasing, result], _previous, onCleanup) => {
    if (releasing || !result) return;
    const timer = setTimeout(() => {
      store.releaseResult = null;
    }, 3000);
    onCleanup(() => clearTimeout(timer));
  },
  { immediate: true }
);
let disposed = false;
const disposers: (() => void)[] = [];
function retain(dispose: () => void) {
  if (disposed) dispose();
  else disposers.push(dispose);
}

async function act(action: () => Promise<void>) {
  try {
    await action();
  } catch {
    store.fail('monitoring_action_failed');
  }
}
let metricRevision = 0;
function selectMetric(metric: MetricId) {
  metricRevision += 1;
  store.selectedMetric = metric;
  void act(() => ResidentService.selectMetric(metric));
}
function selectTab(tab: 'overview' | 'memory') {
  selectMetric(tab === 'memory' ? 'memory' : 'cpu');
}
function moveTab() {
  const next = selectedTab.value === 'memory' ? 'overview' : 'memory';
  selectTab(next);
  void nextTick(() => document.getElementById(`metric-tab-${next}`)?.focus());
}
function navigate(destination: ResidentDestination) {
  void act(() => ResidentService.openMain(destination));
}
function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault();
    void act(() => ResidentService.hidePanel());
  }
}
let subscribed = false;
let connecting: Promise<void> | null = null;
async function connect() {
  if (subscribed || disposed) return;
  if (connecting) return connecting;
  connecting = (async () => {
    // Retrying reconnects the event stream; a one-off cached read cannot restore live updates.
    const pending: (() => void)[] = [];
    try {
      pending.push(
        await ResidentService.onReading(reading => {
          if (!disposed) store.accept(reading);
        })
      );
      pending.push(
        await ResidentService.onPanelMetric(metric => {
          if (!disposed) {
            metricRevision += 1;
            store.selectedMetric = metric;
          }
        })
      );
      if (!disposed)
        pending.push(
          await ResidentService.onFocusChanged(focused => {
            if (disposed) return;
            panelFocused.value = focused;
            if (!focused) return;
            // A prewarmed WebView survives closing. Do not present an old result
            // as a new action's state when the user returns to the panel.
            if (!store.releasing) store.releaseResult = null;
            void appStore.loadSettings();
            void memorySettings.load();
            // Background sampling no longer wakes the hidden WebView. Rehydrate
            // from the native cache without waiting for the next sampling tick.
            void store.load();
            panel.value?.focus({ preventScroll: true });
          })
        );
      pending.forEach(retain);
      subscribed = !disposed;
    } catch (error) {
      pending.forEach(dispose => dispose());
      throw error;
    }
  })();
  try {
    await connecting;
  } finally {
    connecting = null;
  }
}
async function refresh() {
  try {
    await connect();
    if (!disposed) await store.refresh();
  } catch {
    store.fail('monitoring_subscription_failed');
  }
}
onMounted(() => {
  void MemoryReleaseService.onPreferences(value => memorySettings.accept(value))
    .then(retain)
    .catch(() => {
      memorySettings.failed = true;
    });
  void memorySettings.load();
  window.addEventListener('keydown', onKey);
  // Reveal the first rendered frame independently of IPC, samples, and icons.
  // Native icon components progressively fill their placeholders using the shared cache.
  void act(() => ResidentService.panelReady()).then(() => {
    if (!disposed) panel.value?.focus({ preventScroll: true });
  });
  void connect()
    .then(() => {
      const initialRevision = metricRevision;
      if (!disposed)
        return Promise.all([
          store.load(),
          ResidentService.panelMetric().then(metric => {
            if (!disposed && initialRevision === metricRevision) store.selectedMetric = metric;
          }),
        ]);
    })
    .catch(() => {
      if (!disposed) store.fail('monitoring_subscription_failed');
    });
});
onBeforeUnmount(() => {
  disposed = true;
  window.removeEventListener('keydown', onKey);
  disposers.forEach(dispose => dispose());
});
</script>

<template>
  <main ref="panel" class="monitor-panel" tabindex="-1" :aria-label="t('monitoring.title')">
    <div class="monitor-body">
      <div class="resource-header">
        <div class="resource-tabs" role="tablist" :aria-label="t('systemStatus.details')">
          <button
            v-for="tab in tabs"
            :id="`metric-tab-${tab}`"
            :key="tab"
            role="tab"
            :aria-selected="selectedTab === tab"
            aria-controls="metric-details"
            :tabindex="selectedTab === tab ? 0 : -1"
            @click="selectTab(tab)"
            @keydown.right.prevent="moveTab()"
            @keydown.left.prevent="moveTab()"
          >
            {{ t(tab === 'overview' ? 'systemStatus.overview' : 'systemStatus.memoryManagement') }}
          </button>
        </div>
        <MdUpdateNotice />
      </div>
      <section
        v-if="selectedTab === 'overview'"
        id="metric-details"
        class="resource-cards"
        role="tabpanel"
        aria-labelledby="metric-tab-overview"
      >
        <MdResourceOverview
          v-for="metric in overviewMetrics"
          :key="metric"
          :active="panelFocused"
          :metric="metric"
          :reading="store.reading"
          @cleanup="navigate('cleanup')"
          @memory="selectTab('memory')"
        />
      </section>
      <div v-if="store.error" class="monitor-notice" role="alert">
        {{ t('monitoring.unavailable') }} <button @click="refresh()">{{ t('monitoring.refresh') }}</button>
      </div>
      <section
        v-if="store.selectedMetric === 'memory'"
        id="metric-details"
        class="memory-details"
        role="tabpanel"
        aria-labelledby="metric-tab-memory"
      >
        <template v-if="store.reading.memory.value">
          <span v-if="store.reading.memory.status !== 'ready'" class="metric-stale" role="status">{{
            t(METRIC_STATUS_KEYS[store.reading.memory.status])
          }}</span>
          <MdMemoryOverview
            :memory="store.reading.memory.value.memory"
            :releasing="store.releasing"
            :release-result="store.releaseResult"
            @release="store.releaseMemory()"
          >
            <template #settings>
              <div class="release-settings-entry">
                <span v-if="memorySettings.failed" role="alert"
                  >{{ t('memoryRelease.failed') }}
                  <button @click="memorySettings.load()">{{ t('memoryRelease.reload') }}</button></span
                >
                <MdTooltip v-else :text="automaticReleaseRule">
                  <span
                    :tabindex="automaticReleaseRule ? 0 : undefined"
                    :class="{ 'cursor-help': automaticReleaseRule }"
                    >{{
                      t(memorySettings.preferences?.automatic ? 'memoryRelease.autoOn' : 'memoryRelease.autoOff')
                    }}</span
                  >
                </MdTooltip>
                <button @click="act(() => MemoryReleaseService.openSettings())">{{ t('memoryRelease.entry') }}</button>
              </div>
            </template>
          </MdMemoryOverview>
          <MdApplicationMemoryList class="monitor-processes" :summary="store.reading.memory.value.processes" />
        </template>
        <div v-else class="monitor-loading" role="status">
          {{ t(METRIC_STATUS_KEYS[store.reading.memory.status]) }}
        </div>
      </section>
    </div>
    <footer>
      <button class="panel-icon-button" :aria-label="t('monitoring.settings')" @click="navigate('settings')">
        <MdIcon :name="ICON_NAMES.settings" :size="16" />
      </button>
      <button class="open-main-shortcut" @click="navigate('main')">
        {{ t('monitoring.openMain') }}
      </button>
      <button class="quit-shortcut" @click="act(() => ResidentService.quit())">
        {{ t('monitoring.quit') }}
      </button>
    </footer>
  </main>
</template>

<style scoped>
@reference "@assets/main.css";
.resource-cards {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
  overflow-y: auto;
}
.resource-header {
  @apply border-b border-border;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 12px;
}
.resource-tabs {
  display: flex;
  justify-content: flex-start;
  gap: 24px;
  flex: none;
  height: 28px;
}
.resource-tabs button {
  @apply text-muted-foreground;
  position: relative;
  padding: 0 0 4px;
  font-size: 12px;
  font-weight: 500;
  background: transparent;
}
.resource-tabs button:hover {
  @apply text-foreground;
  background: transparent;
}
.resource-tabs button[aria-selected='true'] {
  @apply text-primary;
}
.resource-tabs button[aria-selected='true']::after {
  /* Overlay the divider so switching tabs never changes the content height. */
  content: '';
  position: absolute;
  bottom: -1px;
  left: 0;
  right: 0;
  height: 2px;
  border-radius: 1px;
  background: var(--primary);
}
.metric-stale {
  @apply text-muted-foreground;
  font-size: 11px;
}
.monitor-panel {
  @apply bg-background text-foreground;
  display: flex;
  flex-direction: column;
  height: 100dvh;
  border: 1px solid var(--border);
  overflow: hidden;
}
.monitor-panel:focus {
  outline: none;
}
.panel-icon-button {
  @apply text-muted-foreground;
  display: inline-grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 7px;
  flex: none;
}
button {
  cursor: pointer;
}
button:hover {
  @apply bg-accent text-accent-foreground;
}
button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
button:disabled {
  cursor: default;
  opacity: 0.45;
}
.monitor-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  overflow: hidden;
  min-height: 0;
  flex: 1;
  padding: 12px 12px 14px;
}
.release-settings-entry {
  @apply text-muted-foreground;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 11px;
}
.release-settings-entry button {
  @apply text-primary rounded;
  padding: 4px;
  flex: none;
}
.monitor-processes {
  /* Extend the scroll viewport through the body's right inset to the window edge. */
  margin-right: -12px;
}
.memory-details {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 0;
  flex: 1;
}
.monitor-loading {
  @apply text-muted-foreground;
  display: grid;
  place-items: center;
  min-height: 200px;
  font-size: 12px;
}
.monitor-notice {
  @apply border border-border rounded-lg text-muted-foreground;
  flex: none;
  font-size: 11px;
  line-height: 1.5;
  padding: 10px;
}
.monitor-notice button {
  @apply text-primary;
  text-decoration: underline;
}
footer {
  @apply border-t border-border text-muted-foreground;
  display: grid;
  grid-template-columns: minmax(40px, 1fr) auto minmax(40px, 1fr);
  align-items: center;
  flex: none;
  padding: 6px 14px;
}
.quit-shortcut {
  justify-self: end;
}
.open-main-shortcut,
.quit-shortcut {
  /* Equal side columns keep the main action centered in every locale. */
  justify-content: center;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border-radius: 7px;
  min-height: 30px;
  padding: 0 8px;
  font-size: 11px;
}
</style>
