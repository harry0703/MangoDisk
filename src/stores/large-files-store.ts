import type { ScanNameExclusion } from '@/lib/models/storage/storage-scan';
import { defineStore } from 'pinia';

import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { type LargeFileEntry, type LargeFileScanMode, type LargeFilesResult } from '@/lib/models/storage/large-file';
import type { TraversalProgress } from '@/lib/models/presentation/progress';
import { LargeFileService } from '@/lib/services/storage/large-file-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { PermanentDeleteService } from '@/lib/services/filesystem/permanent-delete-service';
import * as PathUtils from '@/lib/utils/filesystem/path';
import * as LargeFileResultUtils from '@/lib/utils/storage/large-file-result';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage/storage-scan-preference';

import { useAppStore } from './app-store';
import { useHistoryStore } from './history-store';
import { useStorageScanPreferencesStore } from './storage-scan-preferences-store';

interface LargeFilesState {
  result: LargeFilesResult | null;
  progress: TraversalProgress | null;
  loading: boolean;
  cancelling: boolean;
  deleting: boolean;
  resultExcludedFolders: string[];
  resultExcludedNames: ScanNameExclusion[];
}

export const useLargeFilesStore = defineStore('large-files', {
  state: (): LargeFilesState => ({
    result: null,
    progress: null,
    loading: false,
    cancelling: false,
    deleting: false,
    resultExcludedFolders: [],
    resultExcludedNames: [],
  }),
  actions: {
    invalidateResultForExclusionChange() {
      if (!this.result) return;
      const currentNames = useStorageScanPreferencesStore().namesForScope('largeFiles');
      const currentExclusions = useStorageScanPreferencesStore().pathsForScope('largeFiles');
      if (
        StorageScanPreferenceUtils.sameExcludedFolders(this.resultExcludedFolders, currentExclusions) &&
        StorageScanPreferenceUtils.sameExcludedNames(this.resultExcludedNames, currentNames)
      )
        return;
      LoggerService.info(LOG_DOMAINS.largeFiles, LOG_EVENTS.staleScanResultIgnored, {
        operation: 'exclusion_preferences_changed',
        scanId: this.result.scanId,
        previousExcludedFolderCount: this.resultExcludedFolders.length,
        currentExcludedFolderCount: currentExclusions.length,
        previousExcludedNameCount: this.resultExcludedNames.length,
        currentExcludedNameCount: currentNames.length,
      });
      this.result = null;
      this.resultExcludedFolders = [];
      this.resultExcludedNames = [];
    },
    async find(roots: string[], minimumBytes: number, scanMode: LargeFileScanMode) {
      if (this.loading || this.deleting || !roots.length) return;
      roots = PathUtils.uniquePaths(roots);
      if (!roots.length) return;
      const appStore = useAppStore();
      this.loading = true;
      this.cancelling = false;
      this.progress = null;
      appStore.clearError();
      let unlisten: (() => void) | undefined;
      let requestedExclusions: string[] = [];
      let requestedNames: ScanNameExclusion[] = [];
      try {
        const preferencesStore = useStorageScanPreferencesStore();
        await preferencesStore.initialize();
        unlisten = await LargeFileService.listenProgress(progress => {
          this.progress = progress;
        });
        requestedExclusions = preferencesStore.pathsForScope('largeFiles');
        requestedNames = preferencesStore.namesForScope('largeFiles');
        LoggerService.info(LOG_DOMAINS.largeFiles, LOG_EVENTS.scanRequested, {
          rootCount: roots.length,
          roots: roots.slice(0, 8),
          minimumBytes,
          scanMode,
          excludedFolderCount: requestedExclusions.length,
          excludedNameCount: requestedNames.length,
        });
        const result = await LargeFileService.find(roots, minimumBytes, scanMode, requestedExclusions, requestedNames);
        if (
          !StorageScanPreferenceUtils.sameExcludedFolders(
            requestedExclusions,
            preferencesStore.pathsForScope('largeFiles')
          ) ||
          !StorageScanPreferenceUtils.sameExcludedNames(requestedNames, preferencesStore.namesForScope('largeFiles'))
        ) {
          LoggerService.info(LOG_DOMAINS.largeFiles, LOG_EVENTS.staleScanResultIgnored, {
            operation: 'exclusion_preferences_changed',
            scanId: result.scanId,
          });
          this.invalidateResultForExclusionChange();
          return;
        }
        this.result = result;
        this.resultExcludedFolders = requestedExclusions;
        this.resultExcludedNames = requestedNames;
      } catch (error) {
        if (!this.cancelling) {
          LoggerService.warn(LOG_DOMAINS.largeFiles, LOG_EVENTS.operationFailed, {
            operation: 'find_large_files',
            rootCount: roots.length,
            roots: roots.slice(0, 8),
            minimumBytes,
            scanMode,
            excludedFolderCount: requestedExclusions.length,
            excludedNameCount: requestedNames.length,
            error,
          });
          appStore.reportError(error);
        }
      } finally {
        unlisten?.();
        this.progress = null;
        this.loading = false;
        this.cancelling = false;
      }
    },
    async filter(minimumBytes: number) {
      const source = this.result;
      if (!source || this.loading || this.deleting) return;
      const appStore = useAppStore();
      try {
        const result = await LargeFileService.filter(source.scanId, minimumBytes);
        // Threshold queries are intentionally concurrent and filesystem-free. Only the response
        // matching the current preference may replace the visible result when users click quickly.
        if (appStore.settings.largeFileMinimumBytes === minimumBytes && this.result === source) {
          this.result = result;
        } else {
          LoggerService.info(LOG_DOMAINS.largeFiles, LOG_EVENTS.staleScanResultIgnored, {
            requestedMinimumBytes: minimumBytes,
            currentMinimumBytes: appStore.settings.largeFileMinimumBytes,
          });
        }
      } catch (error) {
        appStore.reportError(error);
      }
    },
    async cancel() {
      if (!this.loading || this.cancelling) return;
      this.cancelling = true;
      try {
        await LargeFileService.cancel();
      } catch (error) {
        useAppStore().reportError(error);
        this.cancelling = false;
      }
    },
    async deletePermanently(path: string) {
      const entry = this.result?.entries.find(item => item.path === path);
      if (!entry) return;
      await this.deleteManyPermanently([entry]);
    },
    async deleteManyPermanently(entries: LargeFileEntry[]) {
      this.invalidateResultForExclusionChange();
      if (this.loading || this.deleting || !entries.length) return;
      const appStore = useAppStore();
      const sourceResult = this.result;
      this.deleting = true;
      appStore.clearError();
      try {
        if (!sourceResult) return;
        const result = await PermanentDeleteService.deleteFiles(
          sourceResult.scanId,
          entries.map(entry => entry.path)
        );
        if (sourceResult && this.result === sourceResult) {
          const removedPaths = new Set([...result.removedPaths, ...(result.alreadyAbsentPaths ?? [])]);
          this.result = LargeFileResultUtils.removePaths(sourceResult, removedPaths);
        }
        await useHistoryStore().load({ reportError: false });
        await appStore.refreshSystemDisk();
        if (result.failed.length) {
          // Item-level failures are part of a completed batch and are shown by
          // the page as one warning summary. Logging only aggregate evidence
          // avoids a contradictory global error and keeps private paths out.
          LoggerService.warn(LOG_DOMAINS.largeFiles, LOG_EVENTS.deleteCompletedWithFailures, {
            removedCount: result.removedPaths.length,
            failedCount: result.failed.length,
            releasedBytes: result.releasedBytes,
          });
        }
        return result;
      } catch (error) {
        appStore.reportError(error);
      } finally {
        this.deleting = false;
      }
    },
  },
});
