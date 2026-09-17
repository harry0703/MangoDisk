import { defineStore } from 'pinia';

import {
  APP_UPDATE_ACTION_IDS,
  APP_UPDATE_FAILURE_STAGE_IDS,
  APP_UPDATE_STATUS_IDS,
  type AppUpdateFailureStage,
  type AppUpdateInfo,
  type AppUpdateStatus,
  type AppDistribution,
} from '@/lib/models/app-update';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import { AppDistributionService } from '@/lib/services/app-distribution-service';
import { AppUpdateService } from '@/lib/services/app-update-service';
import { InstallationIdentityService } from '@/lib/services/installation-identity-service';
import { LinkService } from '@/lib/services/link-service';
import { LoggerService } from '@/lib/services/logger-service';
import { normalizeError } from '@/lib/utils/error';

interface AppUpdateState {
  status: AppUpdateStatus;
  distribution: AppDistribution | null;
  currentVersion: string;
  update: AppUpdateInfo | null;
  checkError: string;
  dialogOpen: boolean;
  updateNoticeUnread: boolean;
  downloadedBytes: number;
  totalBytes: number | null;
  actionError: string;
  failureStage: AppUpdateFailureStage | null;
}

export const useAppUpdateStore = defineStore('app-update', {
  state: (): AppUpdateState => ({
    status: APP_UPDATE_STATUS_IDS.idle,
    distribution: null,
    currentVersion: '',
    update: null,
    checkError: '',
    dialogOpen: false,
    updateNoticeUnread: false,
    downloadedBytes: 0,
    totalBytes: null,
    actionError: '',
    failureStage: null,
  }),
  getters: {
    busy: state =>
      state.status === APP_UPDATE_STATUS_IDS.checking ||
      state.status === APP_UPDATE_STATUS_IDS.downloading ||
      state.status === APP_UPDATE_STATUS_IDS.installing ||
      state.status === APP_UPDATE_STATUS_IDS.restarting,
  },
  actions: {
    async initialize() {
      if (!this.currentVersion) {
        // Keep installation identity creation in its existing owner. Native
        // discovery reads the shared store and does not race a second writer.
        try {
          await InstallationIdentityService.getOrCreateInstallId();
        } catch (error) {
          LoggerService.warn(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateMetadataUnavailable, {
            source: 'installation_identity',
            diagnostic: normalizeError(error),
          });
        }
        try {
          this.currentVersion = await AppUpdateService.currentVersion();
        } catch (error) {
          LoggerService.warn(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateVersionReadFailed, {
            diagnostic: normalizeError(error),
          });
        }
      }
      if (!this.distribution) {
        try {
          this.distribution = await AppDistributionService.current();
        } catch (error) {
          const diagnostic = normalizeError(error);
          LoggerService.warn(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateDistributionReadFailed, { diagnostic });
        }
      }
    },
    async check(manual: boolean, refresh = manual) {
      if (this.status === APP_UPDATE_STATUS_IDS.downloaded || this.status === APP_UPDATE_STATUS_IDS.restartRequired) {
        if (manual) this.showAbout();
        return;
      }
      if (this.busy) return;
      this.status = APP_UPDATE_STATUS_IDS.checking;
      this.checkError = '';
      this.actionError = '';
      this.failureStage = null;
      LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateCheckStarted, { manual, refresh });

      try {
        await this.initialize();
        if (!this.distribution) throw new Error('The application distribution is unavailable.');
        const update = await AppUpdateService.check(this.distribution, refresh);
        if (!update) {
          this.update = null;
          this.dialogOpen = manual || this.dialogOpen;
          this.updateNoticeUnread = false;
          this.status = APP_UPDATE_STATUS_IDS.upToDate;
          LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateNotAvailable, {
            currentVersion: this.currentVersion,
            manual,
            refresh,
          });
          return;
        }

        this.currentVersion = update.currentVersion;
        this.update = update;
        this.dialogOpen = manual || this.dialogOpen;
        this.updateNoticeUnread = !manual && !this.dialogOpen;
        this.status = APP_UPDATE_STATUS_IDS.available;
        LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateAvailable, {
          currentVersion: update.currentVersion,
          releaseVersion: update.version,
          action: update.action,
          manual,
          refresh,
        });
      } catch (error) {
        const diagnostic = normalizeError(error).trim();
        this.status = this.update
          ? APP_UPDATE_STATUS_IDS.available
          : manual
            ? APP_UPDATE_STATUS_IDS.error
            : APP_UPDATE_STATUS_IDS.idle;
        this.checkError = manual ? diagnostic : '';
        this.dialogOpen = manual || this.dialogOpen;
        LoggerService.warn(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateCheckFailed, {
          manual,
          refresh,
          diagnostic,
        });
      }
    },
    async download() {
      if (
        !this.update ||
        this.update.action !== APP_UPDATE_ACTION_IDS.automaticInstall ||
        this.status !== APP_UPDATE_STATUS_IDS.available
      )
        return;
      this.status = APP_UPDATE_STATUS_IDS.downloading;
      this.downloadedBytes = 0;
      this.totalBytes = null;
      this.actionError = '';
      this.failureStage = null;
      LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateDownloadStarted, {
        currentVersion: this.currentVersion,
        releaseVersion: this.update.version,
      });

      try {
        await AppUpdateService.download(progress => {
          this.downloadedBytes = progress.downloadedBytes;
          this.totalBytes = progress.totalBytes;
        });
        this.status = APP_UPDATE_STATUS_IDS.downloaded;
        if (!this.dialogOpen) this.updateNoticeUnread = true;
        LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateDownloadCompleted, {
          currentVersion: this.currentVersion,
          releaseVersion: this.update.version,
          downloadedBytes: this.downloadedBytes,
        });
      } catch (error) {
        const diagnostic = normalizeError(error).trim();
        this.status = APP_UPDATE_STATUS_IDS.available;
        this.actionError = diagnostic;
        this.failureStage = APP_UPDATE_FAILURE_STAGE_IDS.download;
        if (!this.dialogOpen) this.updateNoticeUnread = true;
        LoggerService.error(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateDownloadFailed, {
          currentVersion: this.currentVersion,
          releaseVersion: this.update.version,
          diagnostic,
        });
      }
    },
    async installDownloaded() {
      if (
        !this.update ||
        this.update.action !== APP_UPDATE_ACTION_IDS.automaticInstall ||
        this.status !== APP_UPDATE_STATUS_IDS.downloaded
      )
        return;
      this.status = APP_UPDATE_STATUS_IDS.installing;
      this.actionError = '';
      this.failureStage = null;
      LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateInstallStarted, {
        currentVersion: this.currentVersion,
        releaseVersion: this.update.version,
      });

      try {
        await AppUpdateService.installDownloaded();
      } catch (error) {
        const diagnostic = normalizeError(error).trim();
        this.status = APP_UPDATE_STATUS_IDS.downloaded;
        this.actionError = diagnostic;
        this.failureStage = APP_UPDATE_FAILURE_STAGE_IDS.install;
        this.dialogOpen = true;
        LoggerService.error(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateInstallFailed, {
          currentVersion: this.currentVersion,
          releaseVersion: this.update.version,
          diagnostic,
        });
        return;
      }

      this.status = APP_UPDATE_STATUS_IDS.restartRequired;
      LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateInstallCompleted, {
        currentVersion: this.currentVersion,
        releaseVersion: this.update.version,
      });
      await this.restartApplication();
    },
    async restartApplication() {
      if (
        !this.update ||
        this.update.action !== APP_UPDATE_ACTION_IDS.automaticInstall ||
        this.status !== APP_UPDATE_STATUS_IDS.restartRequired
      )
        return;
      this.status = APP_UPDATE_STATUS_IDS.restarting;
      this.actionError = '';
      this.failureStage = null;

      try {
        await AppUpdateService.restartApplication();
      } catch (error) {
        const diagnostic = normalizeError(error).trim();
        this.status = APP_UPDATE_STATUS_IDS.restartRequired;
        this.actionError = diagnostic;
        this.failureStage = APP_UPDATE_FAILURE_STAGE_IDS.restart;
        this.dialogOpen = true;
        LoggerService.error(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateRestartFailed, {
          currentVersion: this.currentVersion,
          releaseVersion: this.update.version,
          diagnostic,
        });
      }
    },
    async openManualDownload() {
      const update = this.update;
      if (
        !update ||
        update.action !== APP_UPDATE_ACTION_IDS.manualDownload ||
        !update.manualDownloadUrl ||
        this.status !== APP_UPDATE_STATUS_IDS.available
      )
        return;

      this.actionError = '';
      this.failureStage = null;
      try {
        await LinkService.open(update.manualDownloadUrl);
        LoggerService.info(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateManualDownloadOpened, {
          currentVersion: this.currentVersion,
          releaseVersion: update.version,
        });
      } catch (error) {
        const diagnostic = normalizeError(error).trim();
        this.actionError = diagnostic;
        this.failureStage = APP_UPDATE_FAILURE_STAGE_IDS.download;
        this.dialogOpen = true;
        LoggerService.error(LOG_DOMAINS.appUpdate, LOG_EVENTS.updateManualDownloadFailed, {
          currentVersion: this.currentVersion,
          releaseVersion: update.version,
          diagnostic,
        });
      }
    },
    dismiss() {
      if (
        this.status === APP_UPDATE_STATUS_IDS.checking ||
        this.status === APP_UPDATE_STATUS_IDS.installing ||
        this.status === APP_UPDATE_STATUS_IDS.restarting
      )
        return;
      this.dialogOpen = false;
      this.actionError = '';
      this.failureStage = null;
    },
    showAbout() {
      this.updateNoticeUnread = false;
      this.dialogOpen = true;
    },
  },
});
