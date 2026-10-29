<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { TabsRoot, TabsList, TabsTrigger, TabsContent } from 'reka-ui';
import type { ResidentPreferences } from '@/lib/models/resident/resident';
import MdResidentDisplayMode from './md-resident-display-mode.vue';

const props = defineProps<{
  open: boolean;
  preferences: ResidentPreferences | null;
  isMacOs: boolean;
  isLinux: boolean;
  loading: boolean;
  saving: boolean;
}>();
const emit = defineEmits<{
  change: [patch: Partial<ResidentPreferences>];
  sectionChange: [];
}>();
const { t } = useI18n();
type SettingsTab = 'content' | 'appearance' | 'interaction';
const tabLabelKeys = {
  content: 'systemStatus.settingsTabs.content',
  appearance: 'systemStatus.settingsTabs.appearance',
  interaction: 'systemStatus.settingsTabs.interaction',
} as const;
const activeTab = ref<SettingsTab>('content');
const windows = computed(() => !props.isMacOs && !props.isLinux);
const tabs = computed<SettingsTab[]>(() =>
  windows.value ? ['content', 'appearance', 'interaction'] : ['content', 'appearance']
);
const displayMode = computed(() =>
  windows.value
    ? (props.preferences?.windowsDisplayMode ?? 'taskbar')
    : props.isMacOs
      ? (props.preferences?.macosDisplayMode ?? 'menuBar')
      : (props.preferences?.linuxDisplayMode ?? 'tray')
);
const scrollPositions = new Map<string, number>();
const editorBody = ref<HTMLElement>();
function rememberScroll(tab: SettingsTab, event: Event) {
  // A queued scroll event can arrive after its panel becomes hidden.
  if (tab !== activeTab.value) return;
  scrollPositions.set(`${displayMode.value}:${tab}`, (event.target as HTMLElement).scrollTop);
}
watch([activeTab, displayMode], async ([tab, mode]) => {
  emit('sectionChange');
  await nextTick();
  const panel = editorBody.value?.querySelector<HTMLElement>('[role="tabpanel"][data-state="active"]');
  if (panel) panel.scrollTop = scrollPositions.get(`${mode}:${tab}`) ?? 0;
});
watch(
  () => props.open,
  open => {
    if (open) {
      activeTab.value = 'content';
      scrollPositions.clear();
    }
  }
);
</script>
<template>
  <div v-if="preferences" class="display-mode-header" :inert="loading" :aria-busy="loading || saving">
    <MdResidentDisplayMode
      :preferences="preferences"
      :platform="windows ? 'windows' : isLinux ? 'linux' : 'macos'"
      @change="emit('change', $event)"
    />
  </div>
  <TabsRoot v-model="activeTab" class="settings-tabs" :unmount-on-hide="false">
    <TabsList class="settings-tab-list" :aria-label="t('systemStatus.settingsCategories')">
      <TabsTrigger v-for="tab in tabs" :key="tab" :value="tab" class="settings-tab">
        {{ t(tabLabelKeys[tab]) }}
      </TabsTrigger>
    </TabsList>
    <div
      id="resident-display-options"
      ref="editorBody"
      class="display-options"
      :inert="loading"
      :aria-busy="loading || saving"
    >
      <TabsContent
        v-for="tab in tabs"
        :key="tab"
        :value="tab"
        class="settings-tab-panel scrollbar-stable-end @container/settings"
        @scroll.passive="rememberScroll(tab, $event)"
      >
        <slot :name="tab" />
      </TabsContent>
    </div>
  </TabsRoot>
</template>
<style scoped>
@reference "@assets/main.css";
.display-mode-header {
  flex: none;
  padding: 4px 20px;
}
.settings-tabs,
.display-options {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
}
.settings-tab-list {
  @apply border-b border-border/70;
  display: flex;
  flex: none;
  padding: 0 20px;
  gap: 8px;
}
.settings-tab {
  @apply text-muted-foreground;
  flex: 1;
  min-width: 0;
  border-bottom: 2px solid transparent;
  padding: 7px 8px;
  font-size: var(--font-content-body);
  line-height: 1.4;
  overflow-wrap: anywhere;
  cursor: pointer;
}
.settings-tab:hover {
  @apply text-foreground;
}
.settings-tab[data-state='active'] {
  @apply border-primary text-primary-text;
  font-weight: 600;
}
.settings-tab:focus-visible,
.settings-tab-panel:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: -2px;
}
.settings-tab-panel {
  flex: 1;
  min-height: 0;
  min-width: 0;
  overflow-y: auto;
  padding: 10px 20px;
}
.settings-tab-panel[hidden] {
  display: none;
}
.settings-tab-panel > :deep(* + *) {
  margin-top: 14px;
}
/* Keep this editor compact without changing settings rows in other dialogs.
   Descriptions and translated labels can still grow their rows naturally. */
.settings-tab-panel :deep(.setting-row--compact) {
  min-height: 36px;
  padding-block: 2px;
}
.settings-tab-panel :deep(.setting-row--compact > .setting-controls [data-slot='select-trigger']) {
  height: 32px;
}
.settings-tab-panel :deep(.settings-section--plain > .settings-list) {
  gap: 4px;
}
.settings-tab-panel :deep(.settings-section--plain > h2) {
  margin-bottom: 6px;
}
</style>
