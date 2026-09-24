<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue';

import MdStatusDisplaySettings from '@/pages/settings/components/md-status-display-settings.vue';
import MdAutostartSettings from '@/pages/settings/components/md-autostart-settings.vue';
import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdStatusBadge from '@/components/custom/md-status-badge.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdFeedbackDialog from '@/pages/settings/components/md-feedback-dialog.vue';
import MdAiFeatureToggle from '@/pages/settings/components/md-ai-feature-toggle.vue';
import MdAiSettingsDialog from '@/components/custom/md-ai-settings-dialog.vue';
import MdIconMangodisk from '@/components/icons/md-icon-mangodisk.vue';
import MdSettingsGroup from '@/components/custom/md-settings-group.vue';
import MdSettingsRow from '@/components/custom/md-settings-row.vue';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { APP_UPDATE_STATUS_IDS } from '@/lib/models/app-update';
import {
  MACOS_ACCESS_STATUS_IDS,
  MACOS_PRIVACY_DESTINATION_IDS,
  type MacOsPrivacyDestination,
} from '@/lib/models/macos-permissions';
import { ICON_NAMES } from '@/lib/models/ui';
import { isLanguageId, isThemeId, LANGUAGE_OPTIONS, THEME_IDS } from '@/lib/models/settings';
import type { AppSettings } from '@/lib/models/settings';
import { FileManagerService } from '@/lib/services/file-manager-service';
import { MacOsPermissionService } from '@/lib/services/macos-permission-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import * as AppUpdateProgressUtils from '@/lib/utils/app-update-progress';
import { useAppUpdateStore } from '@/stores/app-update-store';
import { useAiStore } from '@/stores/ai-store';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';
import { useCleanupStore } from '@/stores/cleanup-store';
import { useLargeFilesStore } from '@/stores/large-files-store';
import { useDuplicateFilesStore } from '@/stores/duplicate-files-store';
import { useAnalysisStore } from '@/stores/analysis-store';

const { t } = useI18n({ useScope: 'global' });
const appUpdateStore = useAppUpdateStore();
const aiStore = useAiStore();
const scanExclusionStore = useStorageScanPreferencesStore();
const cleanupStore = useCleanupStore();
const largeFilesStore = useLargeFilesStore();
const duplicateFilesStore = useDuplicateFilesStore();
const analysisStore = useAnalysisStore();

const props = defineProps<{
  settings: AppSettings;
  focusRevision: number;
}>();
const emit = defineEmits<{
  error: [error: unknown];
  save: [settings: AppSettings];
  openScanExclusions: [];
}>();
const form = reactive<AppSettings>({ ...props.settings });
const aboutRow = ref<HTMLElement | null>(null);
const feedbackOpen = ref(false);
const aiSettingsOpen = ref(false);
const scanExclusionsBusy = computed(
  () =>
    cleanupStore.loading ||
    cleanupStore.closingApplications ||
    largeFilesStore.loading ||
    largeFilesStore.deleting ||
    duplicateFilesStore.loading ||
    duplicateFilesStore.deleting ||
    analysisStore.pending ||
    analysisStore.deleting
);
watch(
  () => aiStore.enabled,
  enabled => {
    if (!enabled) aiSettingsOpen.value = false;
  }
);
const isMacOs = MacOsPermissionService.isMacOs();
const isLinux = OperatingSystemService.isLinux();
const permissionObservation = ref(MacOsPermissionService.defaultObservation());
const languageLabel = computed(() => {
  const option = LANGUAGE_OPTIONS.find(candidate => candidate.id === form.language) ?? LANGUAGE_OPTIONS[0];
  return t(option.labelKey);
});
const themeLabel = computed(() => {
  if (form.theme === THEME_IDS.light) return t('settings.themeLight');
  if (form.theme === THEME_IDS.dark) return t('settings.themeDark');
  return t('settings.themeSystem');
});
const hasPermissionObservation = computed(
  () => permissionObservation.value.applicationDataStatus !== MACOS_ACCESS_STATUS_IDS.notChecked
);
const permissionObservationLabel = computed(() =>
  t(`settings.permissionStatus.${permissionObservation.value.applicationDataStatus}`)
);
const permissionObservationTone = computed<'success' | 'warning'>(() =>
  permissionObservation.value.applicationDataStatus === MACOS_ACCESS_STATUS_IDS.available ? 'success' : 'warning'
);
const currentVersionLabel = computed(() => appUpdateStore.currentVersion || t('settings.versionUnknown'));
const aboutDownloading = computed(() => appUpdateStore.status === APP_UPDATE_STATUS_IDS.downloading);
const aboutDownloadPercent = computed(() =>
  aboutDownloading.value
    ? AppUpdateProgressUtils.percent(appUpdateStore.downloadedBytes, appUpdateStore.totalBytes)
    : null
);
const aboutStatusLabel = computed(() => {
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.checking) return t('settings.updateChecking');
  if (aboutDownloading.value) {
    return aboutDownloadPercent.value === null
      ? t('settings.updateDownloading')
      : t('settings.updateDownloadingProgress', { percent: Math.round(aboutDownloadPercent.value) });
  }
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.downloaded) return t('settings.updateReadyToInstall');
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.installing) return t('settings.updateInstalling');
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.restartRequired) return t('settings.updateRestartRequired');
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.restarting) return t('settings.updateRestarting');
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.available) {
    return t('settings.updateVersionAvailable', { version: appUpdateStore.update?.version ?? '' });
  }
  if (appUpdateStore.status === APP_UPDATE_STATUS_IDS.upToDate) {
    return t('settings.updateUpToDateWithVersion', { version: currentVersionLabel.value });
  }
  return t('settings.versionValue', { version: currentVersionLabel.value });
});

watch(
  () => props.settings,
  value => Object.assign(form, value)
);

watch(
  () => props.focusRevision,
  revision => {
    if (revision <= 0) return;
    void nextTick(() => {
      aboutRow.value?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    });
  },
  { immediate: true, flush: 'post' }
);

function save() {
  emit('save', { ...form });
}

onMounted(() => {
  void scanExclusionStore.initialize().catch(error => emit('error', error));
  if (!isMacOs) return;
  void MacOsPermissionService.loadObservation()
    .then(observation => {
      permissionObservation.value = observation;
    })
    .catch(error => emit('error', error));
});

async function openApplicationLogs() {
  try {
    await FileManagerService.openApplicationLogs();
  } catch (error) {
    emit('error', error);
  }
}

async function openMacOsPrivacySettings(destination: MacOsPrivacyDestination) {
  try {
    await MacOsPermissionService.openPrivacySettings(destination);
  } catch (error) {
    emit('error', error);
  }
}

function updateLanguage(value: unknown) {
  if (!isLanguageId(value)) return;
  form.language = value;
  save();
}

function updateTheme(value: unknown) {
  if (!isThemeId(value)) return;
  form.theme = value;
  save();
}
</script>

<template>
  <MdPageShell class="settings-page @container/settings" content-width="readable" :title="t('settings.title')">
    <MdSettingsGroup :title="t('settings.generalSection')">
      <MdSettingsRow
        :title="t('settings.languageTitle')"
        :description="t('settings.languageDescription')"
        controls="field"
      >
        <template #icon><MdIcon :name="ICON_NAMES.languages" /></template>
        <Select :model-value="form.language" @update:model-value="updateLanguage">
          <SelectTrigger
            ><SelectValue>{{ languageLabel }}</SelectValue></SelectTrigger
          >
          <SelectContent>
            <SelectItem v-for="option in LANGUAGE_OPTIONS" :key="option.id" :value="option.id">
              {{ t(option.labelKey) }}
            </SelectItem>
          </SelectContent>
        </Select>
      </MdSettingsRow>
      <MdSettingsRow :title="t('settings.themeTitle')" :description="t('settings.themeDescription')" controls="field">
        <template #icon><MdIcon :name="ICON_NAMES.theme" /></template>
        <Select :model-value="form.theme" @update:model-value="updateTheme">
          <SelectTrigger
            ><SelectValue>{{ themeLabel }}</SelectValue></SelectTrigger
          >
          <SelectContent>
            <SelectItem :value="THEME_IDS.system">{{ t('settings.themeSystem') }}</SelectItem>
            <SelectItem :value="THEME_IDS.light">{{ t('settings.themeLight') }}</SelectItem>
            <SelectItem :value="THEME_IDS.dark">{{ t('settings.themeDark') }}</SelectItem>
          </SelectContent>
        </Select>
      </MdSettingsRow>
      <MdAutostartSettings />
      <MdStatusDisplaySettings :is-mac-os="isMacOs" :is-linux="isLinux" />
    </MdSettingsGroup>

    <MdSettingsGroup :title="t('settings.scanAnalysisSection')">
      <MdSettingsRow
        as="button"
        :disabled="scanExclusionsBusy"
        controls="responsive"
        :title="t('settings.scanExclusionsTitle')"
        :description="t('settings.scanExclusionsDescription')"
        @click="emit('openScanExclusions')"
      >
        <template #icon><MdIcon :name="ICON_NAMES.folder" /></template>
        <span class="row-action">
          {{ t('settings.scanExclusionsCount', { count: scanExclusionStore.folders.length }) }}
          <MdIcon :name="ICON_NAMES.chevronRight" :size="16" />
        </span>
      </MdSettingsRow>
      <MdAiFeatureToggle @configure="aiSettingsOpen = true" />
      <MdSettingsRow
        v-if="isMacOs"
        as="button"
        controls="responsive"
        :title="t('settings.fullDiskAccessTitle')"
        :description="t('settings.fullDiskAccessDescription')"
        @click="openMacOsPrivacySettings(MACOS_PRIVACY_DESTINATION_IDS.fullDiskAccess)"
      >
        <template #icon><MdIcon :name="ICON_NAMES.hardDrive" /></template>

        <span class="permission-actions">
          <MdStatusBadge v-if="hasPermissionObservation" :tone="permissionObservationTone">
            {{ permissionObservationLabel }}
          </MdStatusBadge>
          <span class="row-action">
            {{ t('settings.openPrivacySettings') }}
            <MdIcon :name="ICON_NAMES.external" :size="15" />
          </span>
        </span>
      </MdSettingsRow>
    </MdSettingsGroup>

    <MdSettingsGroup :title="t('settings.supportSection')">
      <MdSettingsRow
        as="button"
        controls="responsive"
        :title="t('settings.diagnosticLogsTitle')"
        :description="t('settings.diagnosticLogsDescription')"
        @click="openApplicationLogs"
      >
        <template #icon><MdIcon :name="ICON_NAMES.cleanupDiagnosticLogs" /></template>

        <span class="row-action"
          >{{ t('settings.openLogFolderAction') }}<MdIcon :name="ICON_NAMES.external" :size="16"
        /></span>
      </MdSettingsRow>
      <MdSettingsRow
        as="button"
        controls="responsive"
        :title="t('settings.feedbackTitle')"
        :description="t('settings.feedbackDescription')"
        @click="feedbackOpen = true"
      >
        <template #icon><MdIcon :name="ICON_NAMES.help" /></template>

        <span class="row-action"
          >{{ t('settings.feedbackAction') }}<MdIcon :name="ICON_NAMES.chevronRight" :size="16"
        /></span>
      </MdSettingsRow>
    </MdSettingsGroup>

    <MdSettingsGroup :title="t('settings.aboutSection')">
      <div ref="aboutRow">
        <MdSettingsRow
          as="button"
          controls="responsive"
          :title="t('settings.aboutTitle')"
          :description="t('settings.aboutDescription')"
          @click="appUpdateStore.showAbout()"
        >
          <template #icon><MdIconMangodisk :size="34" /></template>
          <template #title
            ><span class="update-title">
              {{ t('settings.aboutTitle') }}
              <span
                v-if="appUpdateStore.updateNoticeUnread"
                class="update-notice"
                :aria-label="t('updates.navigationNotice')"
              /> </span
          ></template>
          <span
            class="row-action update-action"
            :class="{ available: appUpdateStore.status === APP_UPDATE_STATUS_IDS.available }"
          >
            <span class="update-action-content" :class="{ downloading: aboutDownloading }">
              <span class="update-action-label">{{ aboutStatusLabel }}</span>
              <span
                v-if="aboutDownloading"
                class="about-download-track"
                role="progressbar"
                :aria-label="t('updates.downloading')"
                aria-valuemin="0"
                aria-valuemax="100"
                :aria-valuenow="aboutDownloadPercent === null ? undefined : Math.round(aboutDownloadPercent)"
              >
                <span
                  :class="{ indeterminate: aboutDownloadPercent === null }"
                  :style="aboutDownloadPercent === null ? undefined : { width: `${aboutDownloadPercent}%` }"
                />
              </span>
            </span>
            <MdIcon :name="ICON_NAMES.chevronRight" :size="17" />
          </span>
        </MdSettingsRow>
      </div>
    </MdSettingsGroup>

    <MdFeedbackDialog v-model:open="feedbackOpen" @error="emit('error', $event)" />
    <MdAiSettingsDialog
      v-if="aiStore.enabled"
      v-model:open="aiSettingsOpen"
      :quota="aiStore.quota"
      @refresh-quota="aiStore.refreshQuota"
      @configured="aiStore.configurationChanged($event)"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.update-title {
  display: inline-flex;
  align-items: center;
  gap: 7px;
}
.update-notice {
  width: 8px;
  height: 8px;
  flex: none;
  border-radius: 50%;
  @apply bg-primary;
}

.row-action {
  display: flex;
  align-items: center;
  gap: 6px;
}

.update-action.available {
  @apply text-primary;
}

.update-action-content {
  display: flex;
  min-width: 0;
}

.update-action-content.downloading {
  width: 138px;
  flex-direction: column;
  align-items: stretch;
  gap: 5px;
}

.update-action-label {
  overflow: hidden;
  text-align: right;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

.about-download-track {
  height: 3px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--surface-primary-subtle);
}

.about-download-track > span {
  display: block;
  height: 100%;
  border-radius: inherit;
  @apply bg-primary;
}

.about-download-track > span.indeterminate {
  width: 34%;
  animation: settings-update-download 1.2s ease-in-out infinite;
}

.permission-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}
@keyframes settings-update-download {
  from {
    transform: translateX(-110%);
  }
  to {
    transform: translateX(310%);
  }
}
</style>
