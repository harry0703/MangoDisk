import { defineStore } from 'pinia';

import type { VolumeInfo } from '@/lib/models/storage/volume';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import {
  STORAGE_SCOPE_IDS,
  type StorageScopeId,
  type StorageScopePreferences,
} from '@/lib/models/storage/storage-scope';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { StandardScanFolderService } from '@/lib/services/storage/standard-scan-folder-service';
import { type StandardScanFolder } from '@/lib/models/storage/standard-scan-folder';
import * as PathUtils from '@/lib/utils/filesystem/path';
import * as StorageScopePreferenceUtils from '@/lib/utils/storage/storage-scope-preference';

interface StorageScopeState extends StorageScopePreferences {
  initialized: boolean;
  standardFolders: StandardScanFolder[];
}

// Initialization promises stay outside Pinia state because Promises are not
// serializable and should not appear in persisted state or developer tools.
// A WeakMap also keeps isolated Pinia instances independent in unit tests.
const initializationByStore = new WeakMap<object, Promise<void>>();

function selectedDuplicateFileProtection(
  protectedPaths: readonly string[],
  selectedPaths: StorageScopePreferences['selectedPaths']
): string[] {
  const selection = selectedPaths[STORAGE_SCOPE_IDS.duplicateFiles] ?? [];
  const selectedKeys = new Set((Array.isArray(selection) ? selection : [selection]).map(PathUtils.comparisonKey));
  return PathUtils.uniquePaths(protectedPaths).filter(path => selectedKeys.has(PathUtils.comparisonKey(path)));
}

export const useStorageScopeStore = defineStore('storage-scope', {
  state: (): StorageScopeState => ({
    initialized: false,
    schemaVersion: 3,
    selectedPaths: {},
    recentFolders: [],
    duplicateFileProtectedPaths: [],
    standardFolders: [],
  }),
  actions: {
    async initialize() {
      if (this.initialized) return;

      const pendingInitialization = initializationByStore.get(this);
      if (pendingInitialization) {
        await pendingInitialization;
        return;
      }

      // System folders and saved selections are independent inputs. Resolve
      // them together so guarded navigation never mounts a selector with an
      // empty list that is replaced immediately after the first paint.
      const initialization = Promise.all([this.restorePreferences(), this.loadStandardFolders()])
        .then(() => {
          this.initialized = true;
        })
        .finally(() => {
          // Removing the completed task permits a retry if an unexpected error
          // escaped either initialization branch before completion.
          initializationByStore.delete(this);
        });
      initializationByStore.set(this, initialization);
      await initialization;
    },
    async loadStandardFolders() {
      try {
        this.standardFolders = await StandardScanFolderService.list();
      } catch (error) {
        // Standard folders are convenience shortcuts. Disk selection and the
        // native folder dialog remain available when platform discovery fails.
        LoggerService.info(LOG_DOMAINS.storageScope, LOG_EVENTS.standardScanFoldersLoadFailed, { error });
      }
    },
    async restorePreferences() {
      let saved: unknown | null;
      try {
        saved = await PreferenceStorageService.loadStorageScopePreferences();
      } catch (error) {
        LoggerService.warn(LOG_DOMAINS.storageScope, LOG_EVENTS.storageScopePreferencesLoadFailed, { error });
        return;
      }
      try {
        const preferences =
          saved === null ? StorageScopePreferenceUtils.empty() : StorageScopePreferenceUtils.parse(saved);
        this.selectedPaths = preferences.selectedPaths;
        this.recentFolders = preferences.recentFolders;
        this.duplicateFileProtectedPaths = selectedDuplicateFileProtection(
          preferences.duplicateFileProtectedPaths ?? [],
          this.selectedPaths
        );
      } catch (error) {
        LoggerService.warn(LOG_DOMAINS.storageScope, LOG_EVENTS.storageScopePreferencesInvalid, { error });
        this.clearPersistedPreferences();
        return;
      }

      // Saved choices describe user intent. Checking their targets here can prompt
      // for privacy access and must never erase temporarily unavailable selections.
    },
    selectedPath(scopeId: StorageScopeId): string {
      const selection = this.selectedPaths[scopeId];
      return Array.isArray(selection) ? (selection[0] ?? '') : (selection ?? '');
    },
    select(scopeId: StorageScopeId, path: string, disks: readonly VolumeInfo[]) {
      const normalized = PathUtils.display(path);
      if (!normalized) return;

      this.selectedPaths[scopeId] = scopeId === STORAGE_SCOPE_IDS.analysis ? normalized : [normalized];
      if (scopeId === STORAGE_SCOPE_IDS.duplicateFiles) {
        this.duplicateFileProtectedPaths = selectedDuplicateFileProtection(
          this.duplicateFileProtectedPaths ?? [],
          this.selectedPaths
        );
      }
      const diskKeys = new Set(disks.map(disk => PathUtils.comparisonKey(disk.mountPoint)));
      if (!diskKeys.has(PathUtils.comparisonKey(normalized))) {
        this.recentFolders = StorageScopePreferenceUtils.addRecentFolder(this.recentFolders, normalized);
      }
      this.persist();
    },
    selectPaths(
      scopeId: typeof STORAGE_SCOPE_IDS.largeFiles | typeof STORAGE_SCOPE_IDS.duplicateFiles,
      paths: string[],
      disks: readonly VolumeInfo[],
      protectedPaths?: string[]
    ) {
      const keys = new Set<string>();
      const selected = paths.map(PathUtils.display).filter(path => {
        const key = PathUtils.comparisonKey(path);
        if (!key || keys.has(key)) return false;
        keys.add(key);
        return true;
      });
      const previous = this.selectedPaths[scopeId] ?? [];
      const previousKeys = new Set((Array.isArray(previous) ? previous : [previous]).map(PathUtils.comparisonKey));
      this.selectedPaths[scopeId] = selected;
      if (scopeId === STORAGE_SCOPE_IDS.duplicateFiles) {
        this.duplicateFileProtectedPaths = selectedDuplicateFileProtection(
          protectedPaths ?? this.duplicateFileProtectedPaths ?? [],
          this.selectedPaths
        );
      }
      const diskKeys = new Set(disks.map(disk => PathUtils.comparisonKey(disk.mountPoint)));
      const recentKeys = new Set(this.recentFolders.map(PathUtils.comparisonKey));
      for (const path of selected) {
        const key = PathUtils.comparisonKey(path);
        if (!previousKeys.has(key) && !diskKeys.has(key) && !recentKeys.has(key)) {
          this.recentFolders = StorageScopePreferenceUtils.addRecentFolder(this.recentFolders, path);
        }
      }
      this.persist();
    },
    removeFolder(path: string) {
      const removedKey = PathUtils.comparisonKey(path);
      this.recentFolders = StorageScopePreferenceUtils.removePath(this.recentFolders, path);
      for (const [scopeId, selectedPath] of Object.entries(this.selectedPaths)) {
        if (Array.isArray(selectedPath)) {
          this.selectedPaths[scopeId as StorageScopeId] = StorageScopePreferenceUtils.removePath(selectedPath, path);
        } else if (PathUtils.comparisonKey(selectedPath) === removedKey) {
          delete this.selectedPaths[scopeId as StorageScopeId];
        }
      }
      this.duplicateFileProtectedPaths = StorageScopePreferenceUtils.removePath(
        this.duplicateFileProtectedPaths ?? [],
        path
      );
      this.persist();
    },
    setDuplicateFileProtectedPaths(paths: string[]) {
      this.duplicateFileProtectedPaths = selectedDuplicateFileProtection(paths, this.selectedPaths);
      this.persist();
    },
    persist() {
      void PreferenceStorageService.saveStorageScopePreferences({
        schemaVersion: 3,
        selectedPaths: this.selectedPaths,
        recentFolders: this.recentFolders,
        duplicateFileProtectedPaths: this.duplicateFileProtectedPaths,
      }).catch(error => {
        LoggerService.warn(LOG_DOMAINS.storageScope, LOG_EVENTS.storageScopePreferencesSaveFailed, { error });
      });
    },
    clearPersistedPreferences() {
      this.selectedPaths = {};
      this.recentFolders = [];
      this.duplicateFileProtectedPaths = [];
      void PreferenceStorageService.clearStorageScopePreferences().catch(error => {
        LoggerService.warn(LOG_DOMAINS.storageScope, LOG_EVENTS.storageScopePreferencesClearFailed, { error });
      });
    },
  },
});
