<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import MdDialogFooter from '@/components/custom/md-dialog-footer.vue';
import MdDialogHeader from '@/components/custom/md-dialog-header.vue';
import MdSafeRichText from '@/components/custom/md-safe-rich-text.vue';
import MdIcon from '@/components/icons/md-icon.vue';
import MdIconMangodisk from '@/components/icons/md-icon-mangodisk.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import {
  APP_UPDATE_ACTION_IDS,
  APP_UPDATE_FAILURE_STAGE_IDS,
  APP_UPDATE_STATUS_IDS,
  type AppUpdateAction,
  type AppUpdateFailureStage,
  type AppUpdateStatus,
} from '@/lib/models/app-update';
import { PROJECT_LINKS } from '@/lib/models/application-shell';
import { projectWebsiteUrl } from '@/lib/utils/project-website';
import { ICON_NAMES } from '@/lib/models/ui';
import * as AppUpdateProgressUtils from '@/lib/utils/app-update-progress';
import { ByteSizeService } from '@/lib/services/byte-size-service';

const props = defineProps<{
  open: boolean;
  status: AppUpdateStatus;
  action: AppUpdateAction | null;
  currentVersion: string;
  version: string;
  notes: string;
  checkError: string;
  downloadedBytes: number;
  totalBytes: number | null;
  actionError: string;
  failureStage: AppUpdateFailureStage | null;
}>();
const emit = defineEmits<{
  close: [];
  check: [];
  download: [];
  manualDownload: [];
  install: [];
  restart: [];
  openLink: [url: string];
}>();
const { locale, t } = useI18n({ useScope: 'global' });

const checking = computed(() => props.status === APP_UPDATE_STATUS_IDS.checking);
const downloading = computed(() => props.status === APP_UPDATE_STATUS_IDS.downloading);
const downloaded = computed(() => props.status === APP_UPDATE_STATUS_IDS.downloaded);
const installing = computed(() => props.status === APP_UPDATE_STATUS_IDS.installing);
const restartRequired = computed(() => props.status === APP_UPDATE_STATUS_IDS.restartRequired);
const restarting = computed(() => props.status === APP_UPDATE_STATUS_IDS.restarting);
const closeLocked = computed(() => checking.value || installing.value || restarting.value);
const updateAvailable = computed(() => props.status === APP_UPDATE_STATUS_IDS.available);
const manualDownload = computed(() => props.action === APP_UPDATE_ACTION_IDS.manualDownload);
const updateFocused = computed(
  () =>
    props.status === APP_UPDATE_STATUS_IDS.available ||
    props.status === APP_UPDATE_STATUS_IDS.downloading ||
    props.status === APP_UPDATE_STATUS_IDS.downloaded ||
    props.status === APP_UPDATE_STATUS_IDS.installing ||
    props.status === APP_UPDATE_STATUS_IDS.restartRequired ||
    props.status === APP_UPDATE_STATUS_IDS.restarting
);
const currentVersionLabel = computed(() => props.currentVersion || t('settings.versionUnknown'));
const dialogTitle = computed(() => {
  if (restartRequired.value || restarting.value) return t('updates.installedTitle', { version: props.version });
  if (downloaded.value || installing.value) return t('updates.downloadedTitle', { version: props.version });
  return updateFocused.value ? t('updates.availableTitle', { version: props.version }) : t('settings.aboutTitle');
});
const dialogDescription = computed(() =>
  updateFocused.value
    ? t('updates.currentVersionDescription', { version: currentVersionLabel.value })
    : currentVersionLabel.value
);
const websiteUrl = computed(() => projectWebsiteUrl(locale.value));
const downloadPercent = computed(() => AppUpdateProgressUtils.percent(props.downloadedBytes, props.totalBytes));
const progressLabel = computed(() => {
  if (!props.totalBytes) return t('updates.downloading');
  return t('updates.downloadProgress', {
    downloaded: ByteSizeService.bytes(props.downloadedBytes),
    total: ByteSizeService.bytes(props.totalBytes),
  });
});
const updateStateTitle = computed(() => {
  if (checking.value) return t('settings.updateChecking');
  if (props.status === APP_UPDATE_STATUS_IDS.upToDate) return t('settings.updateUpToDate');
  if (props.status === APP_UPDATE_STATUS_IDS.error) return t('settings.updateCheckFailedTitle');
  return t('settings.softwareUpdateTitle');
});
const updateStateDescription = computed(() => {
  if (checking.value) return t('updates.checkingDescription');
  if (props.status === APP_UPDATE_STATUS_IDS.upToDate) {
    return t('updates.upToDateDescription', { version: currentVersionLabel.value });
  }
  if (props.status === APP_UPDATE_STATUS_IDS.error) {
    return props.checkError || t('settings.updateCheckUnknownError');
  }
  return t('updates.checkDescription');
});
const actionErrorTitle = computed(() =>
  manualDownload.value
    ? t('updates.manualDownloadFailed')
    : props.failureStage === APP_UPDATE_FAILURE_STAGE_IDS.restart
      ? t('updates.restartFailed')
      : props.failureStage === APP_UPDATE_FAILURE_STAGE_IDS.install
        ? t('updates.installFailed')
        : t('updates.downloadFailed')
);

function updateOpen(value: boolean) {
  if (!value && !closeLocked.value) emit('close');
}

function downloadUpdate() {
  emit('download');
}
</script>

<template>
  <Dialog :open="open" @update:open="updateOpen">
    <MdDialogContent :class="{ 'is-update-focused': updateFocused }" :show-close="!closeLocked" size="standard">
      <MdDialogHeader class="about-dialog-header" :class="{ focused: updateFocused }" variant="brand">
        <span class="about-dialog-mark" aria-hidden="true">
          <MdIconMangodisk :size="updateFocused ? 46 : 58" />
        </span>
        <div class="about-dialog-identity">
          <DialogTitle>{{ dialogTitle }}</DialogTitle>
          <DialogDescription>{{ dialogDescription }}</DialogDescription>
        </div>
      </MdDialogHeader>

      <div class="about-dialog-body">
        <p v-if="!updateFocused" class="product-description">{{ t('settings.aboutDescription') }}</p>
        <nav v-if="!updateFocused" class="project-links" :aria-label="t('settings.projectLinksLabel')">
          <button type="button" @click="emit('openLink', websiteUrl)">
            {{ t('settings.websiteAction') }}
            <MdIcon :name="ICON_NAMES.external" :size="13" />
          </button>
          <button type="button" @click="emit('openLink', PROJECT_LINKS.repository)">
            {{ t('settings.repositoryAction') }}
            <MdIcon :name="ICON_NAMES.external" :size="13" />
          </button>
          <button type="button" @click="emit('openLink', PROJECT_LINKS.license)">
            {{ t('settings.licenseAction') }}
            <MdIcon :name="ICON_NAMES.external" :size="13" />
          </button>
        </nav>

        <section v-if="!updateFocused" class="update-state" aria-live="polite">
          <span class="update-state-icon" aria-hidden="true">
            <MdIcon :class="{ 'icon-spin': checking }" :name="ICON_NAMES.refresh" :size="20" />
          </span>
          <div class="update-state-copy">
            <strong>{{ updateStateTitle }}</strong>
            <p>{{ updateStateDescription }}</p>
          </div>
        </section>

        <section v-if="updateFocused" class="release-notes" :aria-label="t('updates.releaseNotes')">
          <strong>{{ t('updates.releaseNotes') }}</strong>
          <MdSafeRichText :content="notes || t('updates.noReleaseNotes')" @open-link="emit('openLink', $event)" />
        </section>

        <p v-if="updateAvailable && manualDownload" class="portable-update-note">
          {{ t('updates.portableDescription') }}
        </p>

        <div v-if="downloading" class="download-state" aria-live="polite">
          <div class="download-copy">
            <span>{{ progressLabel }}</span>
            <span class="download-percent">{{
              downloadPercent === null ? '—' : `${Math.round(downloadPercent)}%`
            }}</span>
          </div>
          <div
            class="download-track"
            role="progressbar"
            :aria-label="t('updates.downloading')"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="downloadPercent === null ? undefined : Math.round(downloadPercent)"
          >
            <span
              :class="{ indeterminate: downloadPercent === null }"
              :style="downloadPercent === null ? undefined : { width: `${downloadPercent}%` }"
            />
          </div>
        </div>

        <div
          v-else-if="downloaded || installing || restartRequired || restarting"
          class="download-complete"
          aria-live="polite"
        >
          <MdIcon
            :name="installing || restarting ? ICON_NAMES.refresh : ICON_NAMES.check"
            :class="{ 'icon-spin': installing || restarting }"
            :size="19"
          />
          <span>
            {{
              restarting
                ? t('updates.restarting')
                : restartRequired
                  ? t('updates.restartRequired')
                  : installing
                    ? t('updates.installing')
                    : t('updates.downloadComplete')
            }}
          </span>
        </div>

        <div v-if="actionError" class="update-action-error" role="alert">
          <strong>{{ actionErrorTitle }}</strong>
          <span>{{ actionError }}</span>
        </div>
      </div>

      <MdDialogFooter class="about-dialog-footer">
        <template v-if="updateAvailable">
          <Button type="button" variant="outline" @click="emit('close')">{{ t('updates.notNow') }}</Button>
          <Button v-if="manualDownload" type="button" @click="emit('manualDownload')">
            {{ failureStage ? t('updates.retry') : t('updates.downloadPortable') }}
          </Button>
          <Button v-else type="button" @click="downloadUpdate">
            {{ failureStage ? t('updates.retry') : t('updates.downloadUpdate') }}
          </Button>
        </template>
        <template v-else-if="downloading">
          <Button type="button" variant="outline" @click="emit('close')">{{
            t('updates.continueInBackground')
          }}</Button>
        </template>
        <template v-else-if="downloaded">
          <Button type="button" variant="outline" @click="emit('close')">{{ t('updates.later') }}</Button>
          <Button type="button" @click="emit('install')">{{ t('updates.installAndRestart') }}</Button>
        </template>
        <template v-else-if="installing">
          <Button type="button" disabled>{{ t('updates.installing') }}</Button>
        </template>
        <template v-else-if="restartRequired">
          <Button type="button" variant="outline" @click="emit('close')">{{ t('updates.later') }}</Button>
          <Button type="button" @click="emit('restart')">{{ t('updates.restartNow') }}</Button>
        </template>
        <template v-else-if="restarting">
          <Button type="button" disabled>{{ t('updates.restarting') }}</Button>
        </template>
        <template v-else>
          <Button type="button" variant="outline" :disabled="checking" @click="emit('close')">{{
            t('common.close')
          }}</Button>
          <Button type="button" :disabled="checking" @click="emit('check')">
            {{
              checking
                ? t('settings.updateChecking')
                : status === APP_UPDATE_STATUS_IDS.error
                  ? t('settings.updateRetry')
                  : t('settings.updateCheckAction')
            }}
          </Button>
        </template>
      </MdDialogFooter>
    </MdDialogContent>
  </Dialog>
</template>

<style scoped>
@reference "@assets/main.css";

.about-dialog-header {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 28px 56px 12px;
  text-align: center;
}

.about-dialog-header.focused {
  display: grid;
  grid-template-columns: 52px minmax(0, 1fr);
  align-items: center;
  gap: 14px;
  padding: 24px 28px 16px;
  text-align: left;
}

.portable-update-note {
  color: var(--muted-foreground);
  font-size: 13px;
  line-height: 1.6;
}

.about-dialog-mark {
  display: grid;
  width: 64px;
  height: 64px;
  place-items: center;
  filter: drop-shadow(0 3px 4px var(--shadow-subtle));
  filter: drop-shadow(0 3px 4px color-mix(in oklab, var(--brand-stem, var(--foreground)) 14%, transparent));
}

.about-dialog-header.focused .about-dialog-mark {
  width: 52px;
  height: 52px;
}

.about-dialog-identity {
  display: flex;
  min-width: 0;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.about-dialog-header.focused .about-dialog-identity {
  align-items: flex-start;
  gap: 4px;
}

.about-dialog-identity :deep([data-slot='dialog-title']) {
  font-size: 21px;
  line-height: 1.25;
}

.about-dialog-identity :deep([data-slot='dialog-description']) {
  font-size: var(--font-content-secondary);
}

.about-dialog-body {
  display: flex;
  flex-direction: column;
  gap: 15px;
  padding: 0 26px 8px;
}

.is-update-focused .about-dialog-body {
  gap: 12px;
  padding: 0 28px 8px;
}

.product-description {
  align-self: center;
  max-width: 420px;
  margin: 0;
  text-align: center;
  @apply text-muted-foreground;
  font-size: var(--font-content-body);
  line-height: 1.55;
}

.project-links {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  align-items: center;
  gap: 4px;
  font-size: var(--font-content-secondary);
}

.project-links button {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 0;
  padding: 2px 4px;
  background: transparent;
  cursor: pointer;
  text-decoration-color: transparent;
  text-underline-offset: 4px;
  @apply text-primary transition-colors duration-200 hover:text-primary/75 hover:underline;
}

.update-state {
  display: grid;
  grid-template-columns: 28px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  border-radius: 12px;
  padding: 12px 14px;
  @apply bg-muted/45;
}

.update-state-icon {
  display: grid;
  width: 28px;
  height: 28px;
  place-items: center;
  @apply text-muted-foreground;
}

.update-state-copy {
  min-width: 0;
}

.update-state-copy strong {
  display: block;
  margin-bottom: 2px;
  font-size: var(--font-content-primary);
}

.update-state-copy p {
  margin: 0;
  overflow-wrap: anywhere;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
}

.release-notes {
  min-height: 96px;
  max-height: 280px;
  overflow: auto;
  border-top: 1px solid;
  padding: 16px 2px 8px;
  @apply border-border/70;
}

.release-notes strong {
  display: block;
  margin-bottom: 7px;
  font-size: var(--font-content-body);
}

.download-state {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.download-copy {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 4ch;
  align-items: center;
  gap: 16px;
  @apply text-muted-foreground;
  font-size: var(--font-content-secondary);
  font-variant-numeric: tabular-nums;
}

.download-percent {
  text-align: right;
}

.download-track {
  height: 5px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--surface-primary-subtle);
}

.download-track > span {
  display: block;
  height: 100%;
  border-radius: inherit;
  @apply bg-primary;
}

.download-track > span.indeterminate {
  width: 34%;
  animation: update-download 1.2s ease-in-out infinite;
}

.download-complete,
.update-action-error {
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 10px;
  padding: 10px 12px;
  @apply bg-muted/45 text-muted-foreground;
  font-size: var(--font-content-secondary);
}

.download-complete {
  @apply text-primary;
}

.update-action-error {
  align-items: flex-start;
  flex-direction: column;
  gap: 3px;
  @apply text-destructive;
  background: var(--surface-destructive-subtle);
}

.update-action-error span {
  overflow-wrap: anywhere;
}

.about-dialog-footer {
  align-items: center;
}

.about-dialog-footer :deep(button) {
  min-width: 104px;
}

@keyframes update-download {
  from {
    transform: translateX(-110%);
  }
  to {
    transform: translateX(310%);
  }
}
</style>
