<script setup lang="ts">
import { computed, nextTick, onMounted, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import MdIcon from '@/components/icons/md-icon.vue';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import MdSwitch from '@/components/custom/md-switch.vue';
import { Button } from '@/components/ui/button';
import { ICON_NAMES } from '@/lib/models/presentation/ui';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import MdWindowsDisplayFeedback from './md-windows-display-feedback.vue';
import MdResidentDisplayEditor from './md-resident-display-editor.vue';
const props = withDefaults(defineProps<{ isMacOs: boolean; isLinux?: boolean; openRevision?: number }>(), {
  isLinux: false,
  openRevision: 0,
});
const { t } = useI18n({ useScope: 'global' });
const settings = useResidentSettingsStore();
const editor = ref<InstanceType<typeof MdResidentDisplayEditor> | null>(null);
const displayEnabled = computed(() => settings.preferences?.enabled ?? false);
const enabledChangePending = computed(
  () => settings.saving && settings.draft?.enabled !== settings.preferences?.enabled
);
const panelOpening = ref(false);
const panelError = ref(false);
async function openPanel() {
  panelOpening.value = true;
  panelError.value = false;
  try {
    await ResidentService.openPanel();
  } catch {
    panelError.value = true;
    LoggerService.warn('monitoring', 'resident_panel_open_failed');
  } finally {
    panelOpening.value = false;
  }
}
const settingsOpen = ref(false);
const pendingOpenRevision = ref(0);
watch(
  () => props.openRevision,
  revision => {
    if (revision > 0) pendingOpenRevision.value = revision;
  },
  { immediate: true }
);
watch(
  [pendingOpenRevision, displayEnabled, () => settings.loading],
  ([revision, enabled, loading]) => {
    // Navigation can arrive before persisted display preferences finish loading.
    if (revision > 0 && settings.preferences && !loading) {
      pendingOpenRevision.value = 0;
      if (enabled) settingsOpen.value = true;
      else void nextTick(() => document.getElementById('resident-enabled')?.focus());
    }
  },
  { immediate: true }
);

async function setDisplayEnabled(enabled: boolean) {
  editor.value?.resetInteractions();
  await settings.change({ enabled });
}
watch(displayEnabled, enabled => {
  if (!enabled) settingsOpen.value = false;
});
let disposed = false;
let preferencesRefreshPending = false;
let stopPreferences: (() => void) | undefined;
function refreshPreferences() {
  if (disposed || !preferencesRefreshPending || settings.loading || settings.saving) return;
  // Native menu edits can arrive before an older read or save response settles.
  // Consume the pending refresh before loading so a failed read cannot retry indefinitely.
  preferencesRefreshPending = false;
  void settings.load();
}
watch([() => settings.loading, () => settings.saving], refreshPreferences, { flush: 'post' });
onMounted(async () => {
  try {
    const stop = await ResidentService.onPreferencesChanged(() => {
      preferencesRefreshPending = true;
      refreshPreferences();
    });
    if (disposed) stop();
    else stopPreferences = stop;
  } catch (error) {
    LoggerService.warn('resident', 'preferences_subscription_failed', { error });
  }
  if (!disposed && !settings.preferences) void settings.load();
});
onBeforeUnmount(() => {
  disposed = true;
  stopPreferences?.();
});
</script>

<template>
  <div class="status-settings">
    <MdSettingsRow
      :title="t('floatingMonitor.title')"
      :description="t(isLinux ? 'systemStatus.linuxDisplayHint' : 'systemStatus.displayHint')"
      title-id="resident-enabled-label"
      description-id="resident-enabled-hint"
    >
      <template #icon><MdIcon :name="isMacOs ? ICON_NAMES.menuBar : ICON_NAMES.taskbar" /></template>
      <Button
        v-if="displayEnabled"
        id="resident-open-panel"
        variant="ghost"
        size="sm"
        class="text-muted-foreground"
        :disabled="settings.loading || enabledChangePending || panelOpening"
        @click="openPanel"
        >{{ t('systemStatus.details') }}</Button
      >
      <Button
        v-if="displayEnabled"
        id="resident-configure"
        variant="ghost"
        size="sm"
        class="text-muted-foreground"
        :disabled="settings.loading || enabledChangePending"
        aria-haspopup="dialog"
        @click="settingsOpen = true"
        >{{ t('systemStatus.configureAction') }}</Button
      >
      <MdSwitch
        id="resident-enabled"
        :model-value="displayEnabled"
        :disabled="settings.loading || enabledChangePending || !settings.preferences"
        aria-labelledby="resident-enabled-label"
        aria-describedby="resident-enabled-hint"
        @update:model-value="setDisplayEnabled"
      />
    </MdSettingsRow>
    <MdWindowsDisplayFeedback
      v-if="!isMacOs && !isLinux && settings.preferences"
      class="display-feedback"
      :preferences="settings.preferences"
    />
    <p v-if="settings.error && !settingsOpen" class="settings-feedback display-feedback" role="status">
      {{ t('systemStatus.saveFailed') }}
      <button @click="settings.load()">{{ t('monitoring.refresh') }}</button>
    </p>
    <p v-if="panelError" class="settings-feedback display-feedback" role="alert">
      {{ t('monitoring.unavailable') }}
    </p>
    <MdResidentDisplayEditor ref="editor" v-model:open="settingsOpen" :is-mac-os="isMacOs" :is-linux="isLinux" />
  </div>
</template>

<style scoped>
@reference "@assets/main.css";
.display-feedback {
  padding: 10px 14px;
}
.settings-feedback {
  @apply text-destructive-text;
  padding-top: 8px;
  font-size: 11px;
  line-height: 1.5;
}
.settings-feedback button {
  text-decoration: underline;
  cursor: pointer;
}
</style>
