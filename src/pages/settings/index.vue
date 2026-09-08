<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue';

import MdPageShell from '@/components/custom/md-page-shell.vue';
import MdStatusBadge from '@/components/custom/md-status-badge.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdFeedbackDialog from '@/pages/settings/components/md-feedback-dialog.vue';
import MdAiSettingsDialog from '@/components/custom/md-ai-settings-dialog.vue';
import MdIconMangodisk from '@/components/icons/md-icon-mangodisk.vue';
import { Card } from '@/components/ui/card';
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
import * as AppUpdateProgressUtils from '@/lib/utils/app-update-progress';
import { useAppUpdateStore } from '@/stores/app-update-store';
import { useAiStore } from '@/stores/ai-store';

const { t } = useI18n({ useScope: 'global' });
const appUpdateStore = useAppUpdateStore();
const aiStore = useAiStore();

const props = defineProps<{
  settings: AppSettings;
  focusRevision: number;
}>();
const emit = defineEmits<{
  error: [error: unknown];
  save: [settings: AppSettings];
}>();
const form = reactive<AppSettings>({ ...props.settings });
const aboutRow = ref<HTMLElement | null>(null);
const feedbackOpen = ref(false);
const aiSettingsOpen = ref(false);
const isMacOs = MacOsPermissionService.isMacOs();
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
    <section class="settings-section">
      <h2>{{ t('settings.generalSection') }}</h2>
      <Card class="settings-list">
        <div class="setting-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]">
          <span class="section-icon"><MdIcon :name="ICON_NAMES.languages" /></span>
          <span class="setting-copy"
            ><strong>{{ t('settings.languageTitle') }}</strong
            ><small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{
              t('settings.languageDescription')
            }}</small></span
          >
          <Select :model-value="form.language" @update:model-value="updateLanguage">
            <SelectTrigger class="setting-select col-start-2 w-full @2xl/settings:col-auto @2xl/settings:w-55"
              ><SelectValue>{{ languageLabel }}</SelectValue></SelectTrigger
            >
            <SelectContent>
              <SelectItem v-for="option in LANGUAGE_OPTIONS" :key="option.id" :value="option.id">
                {{ t(option.labelKey) }}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div class="setting-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]">
          <span class="section-icon"><MdIcon :name="ICON_NAMES.theme" /></span>
          <span class="setting-copy"
            ><strong>{{ t('settings.themeTitle') }}</strong
            ><small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{
              t('settings.themeDescription')
            }}</small></span
          >
          <Select :model-value="form.theme" @update:model-value="updateTheme">
            <SelectTrigger class="setting-select col-start-2 w-full @2xl/settings:col-auto @2xl/settings:w-55"
              ><SelectValue>{{ themeLabel }}</SelectValue></SelectTrigger
            >
            <SelectContent>
              <SelectItem :value="THEME_IDS.system">{{ t('settings.themeSystem') }}</SelectItem>
              <SelectItem :value="THEME_IDS.light">{{ t('settings.themeLight') }}</SelectItem>
              <SelectItem :value="THEME_IDS.dark">{{ t('settings.themeDark') }}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </Card>
    </section>

    <section class="settings-section">
      <h2>{{ t('ai.settingsTitle') }}</h2>
      <Card class="settings-list">
        <button
          class="setting-row action-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]"
          type="button"
          @click="aiSettingsOpen = true"
        >
          <span class="section-icon"><MdIcon :name="ICON_NAMES.sparkles" /></span>
          <span class="setting-copy"
            ><strong>{{ t('ai.providerTitle') }}</strong
            ><small class="whitespace-normal">{{ t('ai.settingsDescription') }}</small></span
          >
          <span class="row-action col-start-2 @2xl/settings:col-auto"
            >{{ t('ai.configure') }}<MdIcon :name="ICON_NAMES.chevronRight" :size="16"
          /></span>
        </button>
      </Card>
    </section>

    <section v-if="isMacOs" class="settings-section">
      <h2>{{ t('settings.macosPermissionsSection') }}</h2>
      <Card class="settings-list">
        <button
          class="setting-row action-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]"
          type="button"
          @click="openMacOsPrivacySettings(MACOS_PRIVACY_DESTINATION_IDS.fullDiskAccess)"
        >
          <span class="section-icon"><MdIcon :name="ICON_NAMES.hardDrive" /></span>
          <span class="setting-copy"
            ><strong>{{ t('settings.fullDiskAccessTitle') }}</strong
            ><small class="whitespace-normal">{{ t('settings.fullDiskAccessDescription') }}</small></span
          >
          <span class="permission-actions col-start-2 @2xl/settings:col-auto">
            <MdStatusBadge v-if="hasPermissionObservation" :tone="permissionObservationTone">
              {{ permissionObservationLabel }}
            </MdStatusBadge>
            <span class="row-action">
              {{ t('settings.openPrivacySettings') }}
              <MdIcon :name="ICON_NAMES.external" :size="15" />
            </span>
          </span>
        </button>
      </Card>
    </section>

    <section class="settings-section">
      <h2>{{ t('settings.supportSection') }}</h2>
      <Card class="settings-list">
        <button
          class="setting-row action-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]"
          type="button"
          @click="openApplicationLogs"
        >
          <span class="section-icon"><MdIcon :name="ICON_NAMES.cleanupDiagnosticLogs" /></span>
          <span class="setting-copy"
            ><strong>{{ t('settings.diagnosticLogsTitle') }}</strong
            ><small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{
              t('settings.diagnosticLogsDescription')
            }}</small></span
          >
          <span class="row-action col-start-2 @2xl/settings:col-auto"
            >{{ t('settings.openLogFolderAction') }}<MdIcon :name="ICON_NAMES.external" :size="16"
          /></span>
        </button>
        <button
          class="setting-row action-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]"
          type="button"
          @click="feedbackOpen = true"
        >
          <span class="section-icon"><MdIcon :name="ICON_NAMES.help" /></span>
          <span class="setting-copy"
            ><strong>{{ t('settings.feedbackTitle') }}</strong
            ><small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{
              t('settings.feedbackDescription')
            }}</small></span
          >
          <span class="row-action col-start-2 @2xl/settings:col-auto"
            >{{ t('settings.feedbackAction') }}<MdIcon :name="ICON_NAMES.chevronRight" :size="16"
          /></span>
        </button>
      </Card>
    </section>

    <section class="settings-section">
      <h2>{{ t('settings.aboutSection') }}</h2>
      <Card class="settings-list">
        <button
          ref="aboutRow"
          class="setting-row action-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]"
          type="button"
          @click="appUpdateStore.showAbout()"
        >
          <span class="about-mark"><MdIconMangodisk :size="34" /></span>
          <span class="setting-copy">
            <strong class="update-title">
              {{ t('settings.aboutTitle') }}
              <span
                v-if="appUpdateStore.updateNoticeUnread"
                class="update-notice"
                :aria-label="t('updates.navigationNotice')"
              />
            </strong>
            <small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{
              t('settings.aboutDescription')
            }}</small>
          </span>
          <span
            class="row-action update-action col-start-2 @2xl/settings:col-auto"
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
        </button>
      </Card>
    </section>

    <MdFeedbackDialog v-model:open="feedbackOpen" @error="emit('error', $event)" />
    <MdAiSettingsDialog
      v-model:open="aiSettingsOpen"
      :quota="aiStore.quota"
      @refresh-quota="aiStore.refreshQuota"
      @configured="aiStore.configurationChanged($event)"
    />
  </MdPageShell>
</template>

<style scoped>
@reference "@assets/main.css";

.settings-section > h2 {
  margin: 1px 0 6px 2px;
  @apply text-muted-foreground;
  font-size: var(--font-content-body);
  font-weight: 600;
}

.settings-list {
  gap: 0;
  overflow: hidden;
  border-radius: 10px;
  @apply border-border/70 bg-card shadow-none;
}

.setting-row {
  display: grid;
  width: 100%;
  min-height: 60px;
  align-items: center;
  gap: 10px;
  border: 0;
  border-top-width: 1px;
  padding: 7px 14px;
  background: transparent;
  text-align: left;
  @apply border-border/60 text-card-foreground transition-colors duration-200 hover:bg-muted/50;
}

.setting-row:first-child {
  border-top: 0;
}

.setting-copy {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}
.setting-copy strong {
  font-size: var(--font-content-primary);
  font-weight: 650;
}
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

.setting-copy small {
  overflow: hidden;
  font-size: var(--font-content-secondary);
  text-overflow: ellipsis;
  white-space: nowrap;
  @apply text-muted-foreground;
}
.setting-select {
  height: 36px;
}

.action-row {
  font: inherit;
  cursor: pointer;
}

.action-row:focus-visible {
  position: relative;
  z-index: 1;
  @apply outline-none ring-2 ring-inset ring-ring/50;
}

.action-row:disabled {
  cursor: default;
  opacity: 0.6;
}

.row-action {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--font-content-body);
  @apply text-muted-foreground transition-colors duration-200;
}

.action-row:hover .row-action,
.action-row:focus-visible .row-action,
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
.section-icon {
  display: grid;
  width: 34px;
  height: 34px;
  flex: none;
  place-items: center;
  @apply text-muted-foreground;
}

.about-mark {
  display: grid;
  width: 34px;
  height: 34px;
  flex: none;
  place-items: center;
  overflow: hidden;
  border-radius: 10px;
}

@keyframes settings-update-download {
  from {
    transform: translateX(-110%);
  }
  to {
    transform: translateX(310%);
  }
}

/* Safari 15.6 predates container queries, so mirror the wide settings layout
 * with a viewport query. MangoDisk's desktop window has a 1000px minimum
 * width, which makes this fallback equivalent to the intended container rule.
 */
@supports not (container-type: inline-size) {
  @media (min-width: 900px) {
    .setting-row {
      grid-template-columns: 42px minmax(0, 1fr) auto;
    }

    .setting-copy small {
      white-space: nowrap;
    }

    .setting-select {
      grid-column: auto;
      width: 13.75rem;
    }

    .permission-actions,
    .row-action {
      grid-column: auto;
    }
  }
}
</style>
