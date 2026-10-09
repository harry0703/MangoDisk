import type { ScanNameExclusion } from '@/lib/models/storage-scan';
import { markRaw } from 'vue';
import { defineStore } from 'pinia';

import { ANALYSIS_RESULT_CACHE_LIMIT, ANALYSIS_SCAN_MODES, ANALYSIS_VIEW_IDS } from '@/lib/models/analysis';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/telemetry';
import type {
  AnalysisResult,
  AnalysisScanMode,
  AnalysisViewId,
  AnalysisViewPreferences,
  DirectoryEntryInfo,
} from '@/lib/models/analysis';
import type { TraversalProgress } from '@/lib/models/progress';
import { AnalysisService } from '@/lib/services/analysis-service';
import { LoggerService } from '@/lib/services/logger-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { PreferenceStorageService } from '@/lib/services/preference-storage-service';
import * as AnalysisViewPreferenceUtils from '@/lib/utils/analysis-view-preference';
import * as AnalysisCacheUtils from '@/lib/utils/analysis-cache';
import * as PathUtils from '@/lib/utils/path';
import { parseCommandError } from '@/lib/utils/error';
import * as StorageScanPreferenceUtils from '@/lib/utils/storage-scan-preference';

import { useAppStore } from './app-store';
import { useStorageScanPreferencesStore } from './storage-scan-preferences-store';

interface AnalysisState {
  scanMode: AnalysisScanMode;
  scanModeInitialized: boolean;
  viewPreferences: AnalysisViewPreferences;
  viewPreferencesInitialized: boolean;
  result: AnalysisResult | null;
  cache: Record<string, AnalysisResult>;
  cacheOrder: string[];
  scanExcludedFolders: string[];
  scanExcludedNames: ScanNameExclusion[];
  homePath: string;
  progress: TraversalProgress | null;
  pending: boolean;
  cancelling: boolean;
  scanStarted: boolean;
  deleting: boolean;
  deletingPath: string | null;
  recovering: boolean;
  recoveryRequired: boolean;
}

type ViewPreferenceKey = 'viewMode' | 'treemapDepth' | 'sunburstDepth';
const viewPreferenceLoads = new WeakMap<object, { promise: Promise<void>; edited: Set<ViewPreferenceKey> }>();
const scanModeLoads = new WeakMap<object, { promise: Promise<void>; selected: boolean }>();

export const useAnalysisStore = defineStore('analysis', {
  state: (): AnalysisState => ({
    scanMode: ANALYSIS_SCAN_MODES.standard,
    scanModeInitialized: false,
    viewPreferences: AnalysisViewPreferenceUtils.defaults(),
    viewPreferencesInitialized: false,
    result: null,
    cache: {},
    cacheOrder: [],
    scanExcludedFolders: [],
    scanExcludedNames: [],
    homePath: '',
    progress: null,
    pending: false,
    cancelling: false,
    scanStarted: false,
    deleting: false,
    deletingPath: null,
    recovering: false,
    recoveryRequired: false,
  }),
  actions: {
    async initializeScanMode() {
      if (this.scanModeInitialized) return;
      if (!OperatingSystemService.isWindows()) {
        this.scanMode = ANALYSIS_SCAN_MODES.standard;
        this.scanModeInitialized = true;
        return;
      }
      const pending = scanModeLoads.get(this);
      if (pending) return pending.promise;
      const load = { promise: Promise.resolve(), selected: false };
      load.promise = (async () => {
        try {
          const saved = await PreferenceStorageService.loadAnalysisScanMode();
          if (saved !== null && saved !== ANALYSIS_SCAN_MODES.standard && saved !== ANALYSIS_SCAN_MODES.fast) {
            LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisScanModeInvalid, {
              outcome: 'ignore_saved_mode',
            });
            return;
          }
          // A scan started during restoration owns its explicit selection.
          if (!load.selected && saved !== null) {
            this.scanMode = saved;
            LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisScanModeRestored, { scanMode: saved });
          }
        } catch (error) {
          LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisScanModeLoadFailed, { error });
        } finally {
          this.scanModeInitialized = true;
          scanModeLoads.delete(this);
        }
      })();
      scanModeLoads.set(this, load);
      return load.promise;
    },
    persistScanMode(scanMode: AnalysisScanMode) {
      const pending = scanModeLoads.get(this);
      if (pending) pending.selected = true;
      void PreferenceStorageService.saveAnalysisScanMode(scanMode)
        .then(() => {
          LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisScanModeSaved, { scanMode });
        })
        .catch(error => {
          LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisScanModeSaveFailed, { scanMode, error });
        });
    },
    async initializeViewPreferences() {
      if (this.viewPreferencesInitialized) return;
      const pending = viewPreferenceLoads.get(this);
      if (pending) return pending.promise;
      const edited = new Set<ViewPreferenceKey>();
      const promise = (async () => {
        try {
          const saved = AnalysisViewPreferenceUtils.parse(await PreferenceStorageService.loadAnalysisViewPreferences());
          // A delayed read must preserve choices made while it was in flight.
          for (const key of edited) {
            if (key === 'viewMode') saved.viewMode = this.viewPreferences.viewMode;
            else saved[key] = this.viewPreferences[key];
          }
          this.viewPreferences = saved;
        } catch (error) {
          LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisViewPreferencesLoadFailed, { error });
        } finally {
          this.viewPreferencesInitialized = true;
          viewPreferenceLoads.delete(this);
        }
        if (edited.size) this.persistViewPreferences();
      })();
      viewPreferenceLoads.set(this, { promise, edited });
      return promise;
    },
    setViewMode(viewMode: AnalysisViewId) {
      if (!AnalysisViewPreferenceUtils.isViewMode(viewMode)) return;
      this.updateViewPreferences('viewMode', viewMode);
    },
    setChartDepth(viewMode: AnalysisViewId, depth: number) {
      if (!AnalysisViewPreferenceUtils.isViewMode(viewMode) || !AnalysisViewPreferenceUtils.isDepth(viewMode, depth))
        return;
      this.updateViewPreferences(viewMode === ANALYSIS_VIEW_IDS.treemap ? 'treemapDepth' : 'sunburstDepth', depth);
    },
    updateViewPreferences<Key extends ViewPreferenceKey>(key: Key, value: AnalysisViewPreferences[Key]) {
      void this.initializeViewPreferences();
      viewPreferenceLoads.get(this)?.edited.add(key);
      if (this.viewPreferences[key] === value) return;
      this.viewPreferences = AnalysisViewPreferenceUtils.parse({ ...this.viewPreferences, [key]: value });
      if (this.viewPreferencesInitialized) this.persistViewPreferences();
    },
    persistViewPreferences() {
      void PreferenceStorageService.saveAnalysisViewPreferences(this.viewPreferences).catch(error => {
        LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisViewPreferencesSaveFailed, { error });
      });
    },
    invalidateResultForExclusionChange() {
      const currentNames = useStorageScanPreferencesStore().namesForScope('analysis');
      const currentExclusions = useStorageScanPreferencesStore().pathsForScope('analysis');
      if (
        StorageScanPreferenceUtils.sameExcludedFolders(this.scanExcludedFolders, currentExclusions) &&
        StorageScanPreferenceUtils.sameExcludedNames(this.scanExcludedNames, currentNames)
      )
        return;
      const cachedResultCount = Object.keys(this.cache).length;
      if (this.result || cachedResultCount) {
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisCacheConfigurationChanged, {
          root: this.result?.root ?? null,
          scanId: this.result?.scanId ?? null,
          cachedResultCount,
          previousExcludedFolderCount: this.scanExcludedFolders.length,
          currentExcludedFolderCount: currentExclusions.length,
          previousExcludedNameCount: this.scanExcludedNames.length,
          currentExcludedNameCount: currentNames.length,
        });
      }
      // Every cached folder result belongs to one exclusion configuration.
      // Discard all of them so navigation cannot revive an unfiltered snapshot.
      this.result = null;
      this.cache = {};
      this.cacheOrder = [];
      this.scanExcludedFolders = currentExclusions;
      this.scanExcludedNames = currentNames;
    },
    async analyze(path?: string, refresh = false, setHome = false, requestedMode?: AnalysisScanMode) {
      if (this.pending || this.deleting) return;
      const scanMode = OperatingSystemService.isWindows()
        ? (requestedMode ?? this.scanMode)
        : ANALYSIS_SCAN_MODES.standard;
      if (scanMode !== this.scanMode) {
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisCacheConfigurationChanged, {
          previousMode: this.scanMode,
          scanMode,
          cachedResultCount: Object.keys(this.cache).length,
        });
        this.scanMode = scanMode;
        this.result = null;
        this.cache = {};
        this.cacheOrder = [];
        refresh = true;
      }
      if (OperatingSystemService.isWindows() && requestedMode !== undefined) this.persistScanMode(scanMode);
      const appStore = useAppStore();
      const target = path?.trim() || appStore.disk?.mountPoint;
      const preferences = useStorageScanPreferencesStore();
      // Mark the request pending before loading preferences so the page can
      // replace stale results as soon as the user starts a new analysis.
      this.pending = true;
      this.cancelling = false;
      this.scanStarted = false;
      this.progress = null;
      let unlisten: (() => void) | undefined;
      let requestedExclusions: string[] = [];
      let requestedNames: ScanNameExclusion[] = [];
      try {
        try {
          await preferences.initialize();
        } catch (error) {
          LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, {
            operation: 'load_scan_exclusions',
            root: target,
            error,
          });
          appStore.reportError(error);
          return;
        }
        if (this.cancelling) return;
        this.invalidateResultForExclusionChange();
        requestedExclusions = preferences.pathsForScope('analysis');
        requestedNames = preferences.namesForScope('analysis');
        const targetKey = target ? AnalysisCacheUtils.key(target) : '';
        if (!refresh && targetKey && this.cache[targetKey]) {
          this.result = this.cache[targetKey];
          this.recoveryRequired = false;
          this.cacheOrder = AnalysisCacheUtils.touch(this.cacheOrder, targetKey);
          if (setHome) this.homePath = PathUtils.display(this.result.root);
          return;
        }
        if (refresh && targetKey) {
          this.cache = Object.fromEntries(
            Object.entries(this.cache).filter(([key]) => !PathUtils.isSameOrChildKey(key, targetKey))
          );
          this.cacheOrder = AnalysisCacheUtils.retainExisting(this.cacheOrder, this.cache);
        }
        appStore.clearError();
        unlisten = await AnalysisService.listenProgress(progress => {
          this.progress = progress;
        });
        if (this.cancelling) return;
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.scanRequested, {
          root: target,
          scanMode,
          refresh,
          excludedFolderCount: requestedExclusions.length,
          excludedNameCount: requestedNames.length,
        });
        this.scanStarted = true;
        // Published snapshots are replaced rather than edited in place. Keep large
        // entry arrays out of deep reactivity; workflow state remains reactive.
        const result = markRaw(
          await AnalysisService.analyze(target, refresh, requestedExclusions, requestedNames, scanMode)
        );
        if (
          !StorageScanPreferenceUtils.sameExcludedFolders(requestedExclusions, preferences.pathsForScope('analysis')) ||
          !StorageScanPreferenceUtils.sameExcludedNames(requestedNames, preferences.namesForScope('analysis'))
        ) {
          LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.staleScanResultIgnored, {
            operation: 'exclusion_preferences_changed',
            root: target,
            scanId: result.scanId,
            previousExcludedFolderCount: requestedExclusions.length,
            currentExcludedFolderCount: preferences.pathsForScope('analysis').length,
          });
          this.invalidateResultForExclusionChange();
          return;
        }
        this.result = result;
        this.recoveryRequired = false;
        const cached = AnalysisCacheUtils.store(this.cache, this.cacheOrder, result, ANALYSIS_RESULT_CACHE_LIMIT);
        this.cache = cached.cache;
        this.cacheOrder = cached.order;
        if (setHome || !this.homePath) this.homePath = PathUtils.display(result.root);
      } catch (error) {
        if (!this.cancelling) {
          LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, {
            operation: 'analyze_path',
            scanMode,
            root: target,
            refresh,
            excludedFolderCount: requestedExclusions.length,
            excludedNameCount: requestedNames.length,
            error,
          });
          appStore.reportError(error);
        }
      } finally {
        unlisten?.();
        this.progress = null;
        this.pending = false;
        this.cancelling = false;
        this.scanStarted = false;
      }
    },
    async cancel() {
      if (!this.pending || this.cancelling) return;
      this.cancelling = true;
      // A request cancelled during preference loading or listener setup has
      // not reached Core yet. The pending action will stop before starting it.
      if (!this.scanStarted) return;
      try {
        await AnalysisService.cancel();
      } catch (error) {
        useAppStore().reportError(error);
        this.cancelling = false;
      }
    },
    async refreshAfterDelete(root: string, path: string, deleteFailed: boolean) {
      if (!OperatingSystemService.isWindows() && this.scanMode !== ANALYSIS_SCAN_MODES.standard) {
        this.scanMode = ANALYSIS_SCAN_MODES.standard;
        this.cache = {};
        this.cacheOrder = [];
      }
      const visibleResult = this.result;
      // Deletion or concurrent writes can invalidate every overlapping snapshot.
      // Expire overlapping snapshots before starting a cancellable recovery scan.
      this.cache = AnalysisCacheUtils.invalidateChangedPath(this.cache, path);
      this.cacheOrder = AnalysisCacheUtils.retainExisting(this.cacheOrder, this.cache);
      // Keep the mounted browser and scroll position while the snapshot is
      // explicitly read-only. Failed or cancelled recovery must not revive it
      // as an authoritative cache entry.
      this.recovering = true;
      this.recoveryRequired = true;
      this.pending = true;
      this.cancelling = false;
      this.scanStarted = false;
      this.progress = null;
      let unlisten: (() => void) | undefined;
      const preferences = useStorageScanPreferencesStore();
      const paths = preferences.pathsForScope('analysis');
      const names = preferences.namesForScope('analysis');
      try {
        unlisten = await AnalysisService.listenProgress(progress => {
          this.progress = progress;
        });
        if (this.cancelling) return;
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.scanRequested, {
          operation: 'refresh_analysis_after_delete',
          presentation: 'background',
          scanMode: this.scanMode,
          root,
          excludedFolderCount: paths.length,
          excludedNameCount: names.length,
        });
        this.scanStarted = true;
        const refreshed = markRaw(await AnalysisService.analyze(root, true, paths, names, this.scanMode));
        if (this.cancelling) return;
        if (
          StorageScanPreferenceUtils.sameExcludedFolders(paths, preferences.pathsForScope('analysis')) &&
          StorageScanPreferenceUtils.sameExcludedNames(names, preferences.namesForScope('analysis'))
        ) {
          this.invalidateResultForExclusionChange();
          const cached = AnalysisCacheUtils.store(this.cache, this.cacheOrder, refreshed, ANALYSIS_RESULT_CACHE_LIMIT);
          this.cache = cached.cache;
          this.cacheOrder = cached.order;
          if (this.result === visibleResult) {
            this.result = refreshed;
            this.recoveryRequired = false;
          }
        } else {
          this.invalidateResultForExclusionChange();
          this.result = null;
        }
      } catch (refreshError) {
        if (!this.cancelling) {
          if (deleteFailed) {
            LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, {
              operation: 'refresh_analysis_after_delete',
              root,
              path,
              error: refreshError,
            });
          } else {
            useAppStore().reportAnalysisRefreshFailure(refreshError, root, path);
          }
        }
      } finally {
        unlisten?.();
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisRecoveryFinished, {
          root,
          path,
          outcome: this.recoveryRequired ? (this.cancelling ? 'cancelled' : 'unverified') : 'verified',
        });
        this.recovering = false;
        this.progress = null;
        this.pending = false;
        this.cancelling = false;
        this.scanStarted = false;
      }
    },
    async deletePermanently(entry: DirectoryEntryInfo) {
      if (this.pending || this.deleting || this.recoveryRequired) return;
      this.invalidateResultForExclusionChange();
      if (!this.result) return;
      const sourceResult = this.result;
      const appStore = useAppStore();
      this.deleting = true;
      this.deletingPath = entry.path;
      appStore.clearError();
      try {
        const removed = await AnalysisService.deletePermanently(this.result.scanId, entry.path);

        if (removed.requiresRescan) {
          // A recreated original path cannot be represented by the deleted snapshot.
          this.cache = {};
          this.cacheOrder = [];
          this.deleting = false;
          this.deletingPath = null;
          await this.refreshAfterDelete(sourceResult.root, entry.path, false);
        } else {
          // Core owns allocation transfers and the validity of every scan ID.
          const synchronized = AnalysisCacheUtils.syncAfterDelete(
            this.cache,
            this.cacheOrder,
            removed,
            ANALYSIS_RESULT_CACHE_LIMIT
          );
          for (const result of Object.values(synchronized.cache)) markRaw(result);
          this.cache = synchronized.cache;
          this.cacheOrder = synchronized.order;
          // Refresh the currently visible result rather than the path where the
          // operation started, preserving correctness if a future UI can navigate.
          const visibleResultKey = this.result ? AnalysisCacheUtils.key(this.result.root) : null;
          this.result = visibleResultKey ? (this.cache[visibleResultKey] ?? null) : null;
          this.invalidateResultForExclusionChange();
          LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisCacheSyncedAfterDelete, {
            path: removed.removedPath,
            releasedBytes: removed.releasedBytes,
          });
        }
        await appStore.refreshSystemDisk();
      } catch (error) {
        const failure = parseCommandError(error);
        if (failure?.details.mutationState === 'mayHaveChanged' || failure?.code === 'taskJoinFailed') {
          appStore.reportError(error);
          this.deleting = false;
          this.deletingPath = null;
          // Partial deletion may move shared allocation into a sibling root.
          this.cache = {};
          this.cacheOrder = [];
          await this.refreshAfterDelete(sourceResult.root, entry.path, true);
          await appStore.refreshSystemDisk();
        } else {
          appStore.reportError(error);
        }
      } finally {
        this.deleting = false;
        this.deletingPath = null;
      }
    },
  },
});
