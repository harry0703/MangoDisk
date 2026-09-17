<script setup lang="ts">
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import MdSettingsGroup from '@/components/custom/md-settings-group.vue';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import MdResultSearch from '@/components/custom/md-result-search.vue';
import MdSwitch from '@/components/custom/md-switch.vue';
import { Checkbox } from '@/components/ui/checkbox';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import MdIcon from '@/components/icons/md-icon.vue';
import MdNativeFileIcon from '@/components/custom/md-native-file-icon.vue';
import { ICON_NAMES } from '@/lib/models/ui';
import type { ExcludedApplication, MemoryReleasePreferences } from '@/lib/models/memory-release';
import { MemoryReleaseService } from '@/lib/services/memory-release-service';
import { LoggerService } from '@/lib/services/logger-service';
import { useAppStore } from '@/stores/app-store';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { useMemoryReleaseStore } from '@/stores/memory-release-store';

const { t } = useI18n();
const store = useMemoryReleaseStore();
const windows = OperatingSystemService.isWindows();
const appStore = useAppStore();
const draft = ref<MemoryReleasePreferences | null>(null);
const choosing = ref(false);
const candidates = ref<ExcludedApplication[]>([]);
const search = ref('');
const selected = ref<string[]>([]);
const loading = ref(false);
const pickerFailed = ref(false);
const windowFailed = ref(false);
const conflict = computed(() => !!draft.value && draft.value.revision !== store.preferences?.revision);
const filtered = computed(() =>
  candidates.value.filter(app => `${app.name} ${app.path}`.toLowerCase().includes(search.value.toLowerCase()))
);
let disposed = false;
let unsubscribe: (() => void) | undefined;
let stopFocus: (() => void) | undefined;
async function reload() {
  await store.load();
  if (store.preferences) draft.value = JSON.parse(JSON.stringify(store.preferences)) as MemoryReleasePreferences;
}
async function close() {
  try {
    await MemoryReleaseService.closeSettings();
  } catch (error) {
    windowFailed.value = true;
    LoggerService.warn('monitoring', 'memory_settings_close_failed', { error });
  }
}
async function save() {
  if (draft.value && !conflict.value && (await store.save(draft.value))) await close();
}
async function choose() {
  choosing.value = true;
  selected.value = [];
  search.value = '';
  loading.value = true;
  pickerFailed.value = false;
  try {
    candidates.value = await MemoryReleaseService.applications();
  } catch (error) {
    pickerFailed.value = true;
    LoggerService.warn('monitoring', 'memory_exclusion_candidates_failed', { error });
  } finally {
    loading.value = false;
  }
}
function isAdded(path: string) {
  return draft.value?.exclusions.some(app => app.path.toLowerCase() === path.toLowerCase());
}
function toggleCandidate(path: string) {
  if (isAdded(path)) return;
  selected.value = selected.value.includes(path)
    ? selected.value.filter(value => value !== path)
    : [...selected.value, path];
}
function addSelected() {
  if (!draft.value) return;
  draft.value.exclusions.push(
    ...candidates.value.filter(app => selected.value.includes(app.path) && !isAdded(app.path))
  );
  choosing.value = false;
}
function remove(path: string) {
  if (draft.value) draft.value.exclusions = draft.value.exclusions.filter(app => app.path !== path);
}
onMounted(async () => {
  // Reveal the loading view before persistence or event subscriptions can delay it.
  void MemoryReleaseService.showSettings().catch(error => {
    windowFailed.value = true;
    LoggerService.warn('monitoring', 'memory_settings_show_failed', { error });
  });
  try {
    unsubscribe = await MemoryReleaseService.onPreferences(value => store.accept(value));
    if (disposed) {
      unsubscribe();
      return;
    }
    stopFocus = await MemoryReleaseService.onFocus(() => {
      void appStore.loadSettings();
    });
    if (disposed) {
      stopFocus();
      return;
    }
    await reload();
  } catch (error) {
    store.failed = true;
    LoggerService.warn('monitoring', 'memory_settings_initialize_failed', { error });
  }
});

onBeforeUnmount(() => {
  disposed = true;
  unsubscribe?.();
  stopFocus?.();
});
</script>

<template>
  <main class="release-settings">
    <header>
      <h1>{{ t(choosing ? 'memoryRelease.addTitle' : 'memoryRelease.title') }}</h1>
      <p v-if="choosing">{{ t('memoryRelease.addDescription') }}</p>
    </header>
    <div class="settings-body scrollbar-stable">
      <div v-if="store.failed || conflict || windowFailed" class="notice" role="alert">
        <span>{{ t(conflict ? 'memoryRelease.conflict' : 'memoryRelease.failed') }}</span>
        <button class="page-action" type="button" @click="reload">{{ t('memoryRelease.reload') }}</button>
      </div>
      <template v-if="choosing">
        <MdResultSearch
          v-model="search"
          class="candidate-search"
          :aria-label="t('memoryRelease.search')"
          :placeholder="t('memoryRelease.searchPlaceholder')"
        />
        <p v-if="loading" class="empty" role="status">{{ t('memoryRelease.loading') }}</p>
        <div v-else-if="pickerFailed" class="notice" role="alert">
          <span>{{ t('memoryRelease.failed') }}</span
          ><button class="page-action" @click="choose">{{ t('memoryRelease.reload') }}</button>
        </div>
        <p v-else-if="!filtered.length" class="empty">{{ t('memoryRelease.noMatches') }}</p>
        <ul v-else class="application-list">
          <li v-for="app in filtered" :key="app.path">
            <label class="candidate"
              ><Checkbox
                :model-value="!!isAdded(app.path) || selected.includes(app.path)"
                :value="app.path"
                :disabled="isAdded(app.path)"
                :aria-label="app.name"
                @update:model-value="toggleCandidate(app.path)"
              /><MdNativeFileIcon :path="app.path" :name="app.name" compact /><span class="app-copy"
                ><strong>{{ app.name }}</strong
                ><MdTooltip :text="app.path"
                  ><small>{{ app.path }}</small></MdTooltip
                ></span
              ><span v-if="isAdded(app.path)" class="badge">{{ t('memoryRelease.excluded') }}</span></label
            >
          </li>
        </ul>
      </template>
      <template v-else-if="draft">
        <MdSettingsGroup plain>
          <MdSettingsRow
            compact
            :title="t('memoryRelease.automatic')"
            :description="t('memoryRelease.automaticDescription')"
            label-for="automatic-release"
            ><MdSwitch id="automatic-release" v-model="draft.automatic" :disabled="store.saving"
          /></MdSettingsRow>
          <div class="schedule-fields">
            <div class="schedule-field">
              <label id="release-interval-label" for="release-interval">{{ t('memoryRelease.interval') }}</label>
              <Select v-model="draft.intervalMinutes" :disabled="!draft.automatic || store.saving">
                <SelectTrigger id="release-interval" class="w-full" size="sm" aria-labelledby="release-interval-label"
                  ><SelectValue
                /></SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="minutes in [3, 5, 15, 30, 60, 120]" :key="minutes" :value="minutes">{{
                    t('memoryRelease.minutes', { count: minutes })
                  }}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div class="schedule-field">
              <label id="release-threshold-label" for="release-threshold">{{ t('memoryRelease.threshold') }}</label>
              <Select v-model="draft.thresholdPercent" :disabled="!draft.automatic || store.saving">
                <SelectTrigger id="release-threshold" class="w-full" size="sm" aria-labelledby="release-threshold-label"
                  ><SelectValue
                /></SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="percent in [70, 80, 90]" :key="percent" :value="percent"
                    >{{ percent }}%</SelectItem
                  >
                  <SelectItem :value="0">{{ t('memoryRelease.anyUsage') }}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <label v-if="windows" class="foreground"
              ><Checkbox v-model="draft.skipForeground" class="mt-0.5" :disabled="!draft.automatic || store.saving" />{{
                t('memoryRelease.skipForeground')
              }}</label
            >
          </div>
        </MdSettingsGroup>
        <section v-if="windows" class="exclusion-section">
          <div class="section-heading">
            <div class="exclusion-heading">
              <h2>
                {{ t('memoryRelease.exclusions') }} <span>{{ draft.exclusions.length }}</span>
              </h2>
              <MdTooltip :text="t('memoryRelease.exclusionsHint')">
                <button type="button" class="exclusion-help" :aria-label="t('memoryRelease.exclusionsHint')">
                  <MdIcon :name="ICON_NAMES.help" :size="15" />
                </button>
              </MdTooltip>
            </div>
            <button class="page-action" type="button" :disabled="store.saving" @click="choose">
              {{ t('memoryRelease.add') }}
            </button>
          </div>
          <p v-if="!draft.exclusions.length" class="empty">{{ t('memoryRelease.empty') }}</p>
          <ul v-else class="application-list">
            <li v-for="app in draft.exclusions" :key="app.path" class="excluded-row">
              <MdNativeFileIcon :path="app.path" :name="app.name" compact /><span class="app-copy"
                ><strong>{{ app.name }}</strong
                ><MdTooltip :text="app.path"
                  ><small>{{ app.path }}</small></MdTooltip
                ></span
              ><button
                class="page-action"
                type="button"
                :disabled="store.saving"
                :aria-label="t('memoryRelease.removeNamed', { name: app.name })"
                @click="remove(app.path)"
              >
                {{ t('memoryRelease.remove') }}
              </button>
            </li>
          </ul>
        </section>
      </template>
      <p v-else-if="!store.failed" class="empty">{{ t('memoryRelease.loading') }}</p>
    </div>
    <footer v-if="choosing">
      <Button variant="outline" @click="choosing = false">{{ t('memoryRelease.back') }}</Button
      ><Button class="primary" :disabled="!selected.length || loading" @click="addSelected">
        {{ t('memoryRelease.addSelected', { count: selected.length }) }}
      </Button>
    </footer>
    <footer v-else>
      <span>{{ t('memoryRelease.saveHint') }}</span
      ><Button variant="outline" :disabled="store.saving" @click="close">{{ t('memoryRelease.cancel') }}</Button
      ><Button class="primary" :disabled="!draft || store.saving || conflict" @click="save">
        {{ t(store.saving ? 'memoryRelease.saving' : 'memoryRelease.save') }}
      </Button>
    </footer>
  </main>
</template>

<style scoped>
@reference "@assets/main.css";
.release-settings {
  @apply bg-card text-card-foreground;
  display: flex;
  flex-direction: column;
  /* Follow the WebView content bounds. Older WKWebView versions can size
     dynamic viewport units beyond the visible native window content area. */
  height: 100%;
  min-height: 0;
  overflow: hidden;
}
header {
  flex: none;
  padding: 16px 18px;
  border-bottom: 1px solid var(--border);
}
h1 {
  font-size: 18px;
  font-weight: 650;
}
header p {
  @apply text-muted-foreground;
  font-size: 12px;
  line-height: 1.6;
  margin-top: 7px;
}
.settings-body {
  padding: 12px 18px;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
.schedule-fields {
  padding-top: 4px;
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}
.schedule-field,
.schedule-fields > label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12px;
  min-width: 0;
}
.schedule-fields > .foreground {
  grid-column: 1 / -1;
  flex-direction: row;
  align-items: flex-start;
}
.candidate-search.result-search {
  width: 100%;
  min-width: 0;
  max-width: none;
}
.exclusion-section {
  margin-top: 16px;
}
.section-heading {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}
.exclusion-heading {
  display: flex;
  align-items: center;
  gap: 6px;
}
.exclusion-help {
  @apply text-muted-foreground rounded-sm;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  padding: 3px;
  background-color: transparent;
  cursor: help;
}
.exclusion-help:hover {
  @apply text-foreground bg-accent;
}
h2 {
  font-size: 14px;
  font-weight: 600;
}
h2 span {
  @apply text-muted-foreground;
  font-weight: 400;
  margin-left: 6px;
}
.page-action {
  @apply text-primary rounded-md;
  padding: 7px 10px;
  cursor: pointer;
  font-size: 12px;
  flex: none;
  background-color: transparent;
}
.page-action:hover:not(:disabled) {
  background-color: var(--accent);
}
.page-action:disabled {
  opacity: 0.5;
  cursor: default;
}
.page-action:focus-visible,
.exclusion-help:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}
.application-list {
  padding: 0;
  margin: 8px 0 0;
  list-style: none;
}
.application-list li {
  border-bottom: 1px solid var(--border);
}
.excluded-row,
.candidate {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 0;
}
.candidate {
  cursor: pointer;
}
.app-copy {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1;
}
.app-copy strong {
  font-size: 13px;
  font-weight: 550;
  overflow-wrap: anywhere;
}
.app-copy small {
  @apply text-muted-foreground;
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge {
  @apply text-muted-foreground;
  font-size: 11px;
  flex: none;
}
.empty {
  @apply text-muted-foreground;
  padding: 16px 0;
  font-size: 12px;
  text-align: center;
}
.notice {
  @apply text-destructive bg-muted rounded-md;
  padding: 12px;
  margin-bottom: 14px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}
footer {
  flex: none;
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  min-height: var(--layout-dialog-footer-height);
  padding: var(--layout-dialog-footer-padding);
  @apply border-t border-border/70 bg-muted/20;
  align-items: center;
}
footer span {
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  margin-right: auto;
}
</style>
