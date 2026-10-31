import type { ScanNameExclusion } from '@/lib/models/storage/storage-scan';
import { defineStore } from 'pinia';

import { DUPLICATE_RESULT_PAGE_SIZE, DUPLICATE_SCAN_LOCATION_MODES } from '@/lib/models/storage/duplicate-file';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import type {
  DuplicateFileEntry,
  DuplicateFilesResult,
  DuplicateGroup,
  DuplicateGroupBatch,
  DuplicateScanLocation,
} from '@/lib/models/storage/duplicate-file';
import { FILE_CATEGORY_IDS, type FileCategoryId } from '@/lib/models/filesystem/file-category';
import type { TraversalProgress } from '@/lib/models/presentation/progress';
import { DuplicateFileService } from '@/lib/services/storage/duplicate-file-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import * as DuplicateFileResultUtils from '@/lib/utils/storage/duplicate-file-result';
import * as DuplicateFileGroupUtils from '@/lib/utils/storage/duplicate-file-group';
import * as PathUtils from '@/lib/utils/filesystem/path';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage/storage-scan-preference';

import { useAppStore } from './app-store';
import { useHistoryStore } from './history-store';
import { useStorageScanPreferencesStore } from './storage-scan-preferences-store';

interface DuplicateFilesState {
  result: DuplicateFilesResult | null;
  progress: TraversalProgress | null;
  resultComplete: boolean;
  loading: boolean;
  loadingMore: boolean;
  nextPageOffset: number | null;
  cancelling: boolean;
  deleting: boolean;
  activeOperationId: number | null;
  lastGroupSequence: number;
  resultExcludedFolders: string[];
  resultExcludedNames: ScanNameExclusion[];
}

function countGroupsInCategory(groups: readonly DuplicateGroup[], category: FileCategoryId): number {
  if (category === FILE_CATEGORY_IDS.all) return groups.length;
  return groups.reduce((count, group) => count + Number(DuplicateFileGroupUtils.category(group) === category), 0);
}

export const useDuplicateFilesStore = defineStore('duplicate-files', {
  state: (): DuplicateFilesState => ({
    result: null,
    progress: null,
    resultComplete: false,
    loading: false,
    loadingMore: false,
    nextPageOffset: null,
    cancelling: false,
    deleting: false,
    activeOperationId: null,
    lastGroupSequence: 0,
    resultExcludedFolders: [],
    resultExcludedNames: [],
  }),
  getters: {
    hasMore: state => state.nextPageOffset !== null,
  },
  actions: {
    invalidateResultForExclusionChange() {
      if (!this.result) return;
      const currentNames = useStorageScanPreferencesStore().namesForScope('duplicateFiles');
      const currentExclusions = useStorageScanPreferencesStore().pathsForScope('duplicateFiles');
      if (
        StorageScanPreferenceUtils.sameExcludedFolders(this.resultExcludedFolders, currentExclusions) &&
        StorageScanPreferenceUtils.sameExcludedNames(this.resultExcludedNames, currentNames)
      )
        return;
      LoggerService.info(LOG_DOMAINS.duplicateFiles, LOG_EVENTS.staleScanResultIgnored, {
        operation: 'exclusion_preferences_changed',
        scanId: this.result.scanId,
        previousExcludedFolderCount: this.resultExcludedFolders.length,
        currentExcludedFolderCount: currentExclusions.length,
        previousExcludedNameCount: this.resultExcludedNames.length,
        currentExcludedNameCount: currentNames.length,
      });
      this.clearResult();
    },
    async find(locations: DuplicateScanLocation[], minimumBytes: number) {
      if (this.loading || this.deleting || !locations.length) return;
      const roots = PathUtils.collapseOverlappingRoots(locations.map(location => location.path));
      const protectedRoots = PathUtils.uniquePaths(
        locations
          .filter(location => location.mode === DUPLICATE_SCAN_LOCATION_MODES.protected)
          .map(location => location.path)
      );
      const appStore = useAppStore();
      const preferencesStore = useStorageScanPreferencesStore();
      try {
        await preferencesStore.initialize();
      } catch (error) {
        appStore.reportError(error);
        return;
      }
      if (this.loading || this.deleting) return;
      const requestedNames = preferencesStore.namesForScope('duplicateFiles');
      const requestedExclusions = preferencesStore.pathsForScope('duplicateFiles');
      const retainCurrentResult = Boolean(
        this.result &&
        PathUtils.sameRootScope(this.result.roots, roots) &&
        PathUtils.sameRootScope(this.result.protectedRoots, protectedRoots) &&
        StorageScanPreferenceUtils.sameExcludedFolders(this.resultExcludedFolders, requestedExclusions) &&
        StorageScanPreferenceUtils.sameExcludedNames(this.resultExcludedNames, requestedNames)
      );
      this.loading = true;
      this.cancelling = false;
      this.progress = null;
      this.loadingMore = false;
      this.activeOperationId = null;
      this.lastGroupSequence = 0;
      if (!retainCurrentResult) {
        this.result = null;
        this.resultComplete = false;
        this.nextPageOffset = null;
      }
      appStore.clearError();
      LoggerService.info(LOG_DOMAINS.duplicateFiles, LOG_EVENTS.scanRequested, {
        rootCount: roots.length,
        roots: roots.slice(0, 8),
        protectedRootCount: protectedRoots.length,
        excludedFolderCount: requestedExclusions.length,
        excludedNameCount: requestedNames.length,
        minimumBytes,
      });
      let unlistenProgress: (() => void) | undefined;
      let unlistenGroups: (() => void) | undefined;
      let scanInvoked = false;
      try {
        [unlistenProgress, unlistenGroups] = await Promise.all([
          DuplicateFileService.listenProgress(progress => {
            // Ignore progress from a request whose threshold is no longer
            // current, preventing stale events from repopulating cleared state.
            if (appStore.settings.duplicateFileMinimumBytes !== minimumBytes) return;
            if (this.activeOperationId === null) this.activeOperationId = progress.operationId;
            if (progress.operationId !== this.activeOperationId) return;
            this.progress = progress;
            if (!retainCurrentResult && this.result && !this.resultComplete) {
              this.result = {
                ...this.result,
                scannedFileCount: progress.itemsScanned,
              };
            }
          }),
          DuplicateFileService.listenGroups(batch => {
            if (appStore.settings.duplicateFileMinimumBytes !== minimumBytes) return;
            // A same-scope refresh keeps the published result stable until the
            // replacement is complete. Initial scans still stream groups.
            if (retainCurrentResult) return;
            this.applyGroupBatch(batch, roots, protectedRoots);
          }),
        ]);
        scanInvoked = true;
        const result = await DuplicateFileService.find(locations, minimumBytes, requestedExclusions, requestedNames);
        // Settings can change during hashing. Discard results produced with a
        // threshold that no longer matches the active workflow.
        if (
          appStore.settings.duplicateFileMinimumBytes === minimumBytes &&
          StorageScanPreferenceUtils.sameExcludedFolders(
            requestedExclusions,
            preferencesStore.pathsForScope('duplicateFiles')
          ) &&
          StorageScanPreferenceUtils.sameExcludedNames(requestedNames, preferencesStore.namesForScope('duplicateFiles'))
        ) {
          this.result = result;
          this.resultExcludedFolders = requestedExclusions;
          this.resultExcludedNames = requestedNames;
          this.resultComplete = true;
          this.nextPageOffset = result.groups.length < result.returnedGroupCount ? result.groups.length : null;
        } else {
          LoggerService.info(LOG_DOMAINS.duplicateFiles, LOG_EVENTS.staleScanResultIgnored, {
            requestedMinimumBytes: minimumBytes,
            currentMinimumBytes: appStore.settings.duplicateFileMinimumBytes,
          });
          this.clearResult();
        }
      } catch (error) {
        // Core discards its previous session when a new scan starts. Retained
        // rows remain useful for viewing, but cannot authorize another delete.
        if (scanInvoked) {
          this.resultComplete = false;
          this.nextPageOffset = null;
        }
        if (!this.cancelling) {
          LoggerService.warn(LOG_DOMAINS.duplicateFiles, LOG_EVENTS.operationFailed, {
            operation: 'find_duplicate_files',
            rootCount: roots.length,
            roots: roots.slice(0, 8),
            protectedRootCount: protectedRoots.length,
            excludedFolderCount: requestedExclusions.length,
            excludedNameCount: requestedNames.length,
            minimumBytes,
            operationId: this.activeOperationId,
            error,
          });
          appStore.reportError(error);
        }
      } finally {
        unlistenProgress?.();
        unlistenGroups?.();
        this.progress = null;
        this.loading = false;
        this.cancelling = false;
        this.activeOperationId = null;
      }
    },
    applyGroupBatch(batch: DuplicateGroupBatch, roots: string[], protectedRoots: string[]) {
      if (this.activeOperationId === null) this.activeOperationId = batch.operationId;
      if (
        batch.operationId !== this.activeOperationId ||
        batch.sequence <= this.lastGroupSequence ||
        this.resultComplete
      ) {
        return;
      }
      this.lastGroupSequence = batch.sequence;
      const groups = [...(this.result?.groups ?? [])];
      const knownHashes = new Set(groups.map(group => group.hash));
      for (const group of batch.groups) {
        if (knownHashes.has(group.hash)) continue;
        knownHashes.add(group.hash);
        groups.push(group);
      }
      this.result = {
        scanId: batch.operationId,
        roots: [...roots],
        protectedRoots: [...protectedRoots],
        scannedAtMs: 0,
        scannedFileCount: this.progress?.itemsScanned ?? 0,
        skippedCount: 0,
        duplicateFileCount: batch.foundFileCount,
        totalDuplicateBytes: batch.foundTotalBytes,
        reclaimableBytes: batch.foundReclaimableBytes,
        totalGroupCount: batch.foundGroupCount,
        returnedGroupCount: batch.foundGroupCount,
        truncated: false,
        groups,
      };
    },
    async loadMore(category: FileCategoryId = FILE_CATEGORY_IDS.all, loadAll = false) {
      if (!this.result || !this.resultComplete || !this.hasMore || this.loading || this.loadingMore || this.deleting) {
        return;
      }
      const sourceScanId = this.result.scanId;
      const initialCategoryCount = countGroupsInCategory(this.result.groups, category);
      let requestedResult: DuplicateFilesResult = this.result;
      this.loadingMore = true;
      try {
        while (this.nextPageOffset !== null) {
          const sourceOffset = this.nextPageOffset;
          const currentResult: DuplicateFilesResult | null = this.result;
          if (!currentResult || currentResult.scanId !== sourceScanId) return;

          requestedResult = currentResult;
          const page = await DuplicateFileService.page(sourceScanId, sourceOffset, DUPLICATE_RESULT_PAGE_SIZE);
          // A refresh or deletion may overlap a read-only page request. Neither
          // its old contents nor its cursor may replace the active result.
          if (
            this.loading ||
            this.deleting ||
            !this.resultComplete ||
            !this.result ||
            this.result !== currentResult ||
            page.scanId !== sourceScanId
          )
            return;

          const knownHashes = new Set(this.result.groups.map(group => group.hash));
          const groups: DuplicateGroup[] = [
            ...this.result.groups,
            ...page.groups.filter(group => {
              if (knownHashes.has(group.hash)) return false;
              knownHashes.add(group.hash);
              return true;
            }),
          ];
          this.result = { ...this.result, groups };
          // The native session owns ordering and the next cursor. Local
          // deduplication cannot safely derive the following offset.
          this.nextPageOffset = page.nextOffset;

          // The page cursor spans every category. While a category filter is
          // active, continue across unrelated pages so one click always reveals
          // a matching group or reaches the end of the native result set.
          if (
            !loadAll &&
            (category === FILE_CATEGORY_IDS.all || countGroupsInCategory(groups, category) > initialCategoryCount)
          ) {
            break;
          }
        }
      } catch (error) {
        if (this.loading || this.deleting || !this.resultComplete || this.result !== requestedResult) return;
        if (this.result?.scanId === sourceScanId) {
          // Preserve loaded rows as read-only when the native session expires.
          this.resultComplete = false;
          this.nextPageOffset = null;
        }
        useAppStore().reportError(error);
      } finally {
        // A late page request must not unlock pagination for a newer scan.
        if (!this.result || this.result.scanId === sourceScanId) {
          this.loadingMore = false;
        }
      }
    },
    async cancel() {
      if (!this.loading || this.cancelling) return;
      this.cancelling = true;
      try {
        await DuplicateFileService.cancel();
      } catch (error) {
        useAppStore().reportError(error);
        this.cancelling = false;
      }
    },
    clearResult() {
      this.result = null;
      this.resultExcludedFolders = [];
      this.resultExcludedNames = [];
      this.resultComplete = false;
      this.loadingMore = false;
      this.nextPageOffset = null;
      this.activeOperationId = null;
      this.lastGroupSequence = 0;
    },
    async deletePermanently(entries: DuplicateFileEntry[]) {
      this.invalidateResultForExclusionChange();
      if (!this.result || !this.resultComplete || this.loading || this.deleting || !entries.length) return;
      const appStore = useAppStore();
      const sourceResult = this.result;
      this.deleting = true;
      appStore.clearError();
      try {
        const operation = await DuplicateFileService.deletePermanently(
          sourceResult.scanId,
          entries.map(entry => ({
            path: entry.path,
            expectedBytes: entry.bytes,
            expectedModifiedAtMs: entry.modifiedAtMs,
          }))
        );
        const removedPaths = new Set([...operation.removedPaths, ...(operation.alreadyAbsentPaths ?? [])]);
        // Apply the update only while the initiating result remains active.
        if (this.result === sourceResult) {
          this.result = DuplicateFileResultUtils.removePaths(sourceResult, removedPaths);
          if (this.nextPageOffset !== null) {
            this.nextPageOffset = Math.min(this.nextPageOffset, this.result.returnedGroupCount);
          }
        }
        await useHistoryStore().load({ reportError: false });
        await appStore.refreshSystemDisk();
        if (operation.failed.length) {
          // The caller owns the single user-facing partial-success warning.
          // Keep diagnostics aggregate-only because failure entries contain
          // private paths that must not cross the frontend logging boundary.
          LoggerService.warn(LOG_DOMAINS.duplicateFiles, LOG_EVENTS.deleteCompletedWithFailures, {
            removedCount: operation.removedPaths.length,
            failedCount: operation.failed.length,
            releasedBytes: operation.releasedBytes,
          });
        }
        return operation;
      } catch (error) {
        appStore.reportError(error);
      } finally {
        this.deleting = false;
      }
    },
  },
});
