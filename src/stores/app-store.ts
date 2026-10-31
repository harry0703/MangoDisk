import type { CommandErrorReason } from '@/lib/models/diagnostics/command-error';
import { defineStore } from 'pinia';

import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { PAGE_IDS } from '@/lib/models/shell/application-shell';
import type { AppSettings } from '@/lib/models/settings/settings';
import type { VolumeInfo } from '@/lib/models/storage/volume';
import type { PageId } from '@/lib/models/shell/application-shell';
import { VolumeService } from '@/lib/services/storage/volume-service';
import { LanguageService } from '@/lib/services/settings/language-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { ThemeService } from '@/lib/services/settings/theme-service';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import * as AppSettingsUtils from '@/lib/utils/settings/app-settings';
import {
  normalizeError,
  parseCommandError,
  parseCommandErrorReason,
  type CommandErrorCode,
} from '@/lib/utils/diagnostics/error';

interface AppState {
  currentPage: PageId;
  disk: VolumeInfo | null;
  disks: VolumeInfo[];
  diskRevision: number;
  pendingSystemDiskRefreshes: number;
  settings: AppSettings;
  errorCode: CommandErrorCode | null;
  errorReason: CommandErrorReason | 'analysisRefreshFailedAfterDelete' | null;
}

export const useAppStore = defineStore('app', {
  state: (): AppState => ({
    currentPage: PAGE_IDS.cleanup,
    disk: null,
    disks: [],
    diskRevision: 0,
    pendingSystemDiskRefreshes: 0,
    // Application startup loads platform-aware settings before Vue mounts. The
    // deterministic binary placeholder keeps isolated Store tests independent
    // from Tauri's window-scoped OS plugin.
    settings: AppSettingsUtils.defaults(LanguageService.detectSystemLanguage()),
    errorCode: null,
    errorReason: null,
  }),
  actions: {
    async initialize() {
      const revision = this.diskRevision;
      try {
        const [disk, disks] = await Promise.all([VolumeService.getSystemVolume(), VolumeService.listVolumes()]);
        if (revision !== this.diskRevision || this.pendingSystemDiskRefreshes) return;
        this.disk = disk;
        this.disks = disks;
        this.diskRevision += 1;
      } catch (error) {
        if (revision === this.diskRevision && !this.pendingSystemDiskRefreshes) this.reportError(error);
      }
    },
    navigate(page: PageId) {
      this.currentPage = page;
      this.errorCode = null;
      this.errorReason = null;
    },
    reportOperationBusy(reason: CommandErrorReason | null = null) {
      this.errorCode = 'operationBusy';
      this.errorReason = reason;
      LoggerService.info(LOG_DOMAINS.applicationShell, LOG_EVENTS.operationDeferred, {
        code: this.errorCode,
        reason,
      });
    },
    updateSystemDisk(disk: VolumeInfo) {
      this.diskRevision += 1;
      this.disk = disk;
      const index = this.disks.findIndex(item => item.mountPoint === disk.mountPoint);
      if (index >= 0) this.disks[index] = disk;
    },
    async refreshSystemDisk(): Promise<boolean> {
      const revision = ++this.diskRevision;
      this.pendingSystemDiskRefreshes += 1;
      try {
        const disk = await VolumeService.getSystemVolume(true);
        if (revision !== this.diskRevision) return false;
        this.updateSystemDisk(disk);
        return true;
      } catch (error) {
        /*
         * Capacity is a secondary view after a completed filesystem mutation.
         * A refresh failure must not turn that completed operation into a user-
         * visible failure, but the typed error code is retained for diagnosis.
         */
        LoggerService.warn(LOG_DOMAINS.applicationShell, LOG_EVENTS.diskRefreshFailed, {
          code: parseCommandError(error)?.code ?? 'operationFailed',
        });
        return false;
      } finally {
        this.pendingSystemDiskRefreshes -= 1;
      }
    },
    async refreshDisks(): Promise<boolean> {
      if (this.pendingSystemDiskRefreshes) return false;
      const revision = this.diskRevision;
      try {
        // Retry the authoritative system lookup only when startup left it unavailable.
        // Volume ordering is not a system-disk contract (for example on Windows).
        const [disks, systemDisk] = await Promise.all([
          VolumeService.listVolumes(),
          this.disk ? Promise.resolve(null) : VolumeService.getSystemVolume(),
        ]);
        // A periodic response must not overwrite a post-cleanup forced refresh.
        if (revision !== this.diskRevision || this.pendingSystemDiskRefreshes) return false;
        this.disks = disks;
        const disk = this.disk ?? systemDisk;
        this.disk = disks.find(item => item.mountPoint === disk?.mountPoint) ?? disk;
        this.diskRevision += 1;
        return true;
      } catch (error) {
        LoggerService.warn(LOG_DOMAINS.applicationShell, LOG_EVENTS.diskRefreshFailed, {
          code: parseCommandError(error)?.code ?? 'operationFailed',
        });
        return false;
      }
    },
    reportError(error: unknown) {
      const commandError = parseCommandError(error);
      if (commandError?.code === 'operationBusy') {
        this.reportOperationBusy(parseCommandErrorReason(commandError));
        return;
      }
      this.errorCode = commandError?.code ?? 'operationFailed';
      this.errorReason = parseCommandErrorReason(commandError);
      LoggerService.error(LOG_DOMAINS.applicationShell, LOG_EVENTS.operationFailed, {
        code: this.errorCode,
        diagnostic: normalizeError(error),
      });
    },
    reportAnalysisRefreshFailure(error: unknown, root: string, path: string) {
      // The delete already completed; a scan error must identify the failed
      // follow-up rather than imply that the destructive operation failed.
      this.errorCode = 'operationFailed';
      this.errorReason = 'analysisRefreshFailedAfterDelete';
      LoggerService.error(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, {
        operation: 'refresh_analysis_after_delete',
        root,
        path,
        diagnostic: normalizeError(error),
      });
    },
    setSidebarExpanded(expanded: boolean) {
      this.settings = { ...this.settings, sidebarExpanded: expanded };
      void PreferenceStorageService.saveSidebarExpanded(expanded, this.settings).catch(error => {
        LoggerService.warn(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsSaveFailed, {
          key: 'sidebarExpanded',
          error,
        });
        this.reportError(error);
      });
    },
    saveSettings(settings: AppSettings) {
      this.settings = AppSettingsUtils.parse(settings, ByteSizeService.currentUnitBase());
      void PreferenceStorageService.saveSettings(this.settings).catch(error => {
        LoggerService.warn(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsSaveFailed, {
          error,
        });
        this.reportError(error);
      });
      LanguageService.apply(this.settings.language);
      ThemeService.apply(this.settings.theme);
      ThemeService.applyColorTheme(this.settings.colorTheme);
    },
    async loadSettings() {
      const unitBase = ByteSizeService.currentUnitBase();
      const defaults = AppSettingsUtils.defaults(LanguageService.detectSystemLanguage(), unitBase);
      let value: unknown | null;
      try {
        value = await PreferenceStorageService.loadSettings();
      } catch (error) {
        LoggerService.warn(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsLoadFailed, { error });
        this.settings = defaults;
        LanguageService.apply(this.settings.language);
        ThemeService.apply(this.settings.theme);
        ThemeService.applyColorTheme(this.settings.colorTheme);
        return;
      }
      try {
        this.settings = value === null ? defaults : AppSettingsUtils.parse(value, unitBase);
      } catch (error) {
        LoggerService.warn(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsInvalid, {
          error,
        });
        try {
          await PreferenceStorageService.clearSettings();
        } catch (clearError) {
          LoggerService.warn(LOG_DOMAINS.settings, LOG_EVENTS.savedSettingsClearFailed, { error: clearError });
        }
        this.settings = defaults;
      }
      LanguageService.apply(this.settings.language);
      ThemeService.apply(this.settings.theme);
      ThemeService.applyColorTheme(this.settings.colorTheme);
    },
    clearError() {
      this.errorCode = null;
      this.errorReason = null;
    },
  },
});
