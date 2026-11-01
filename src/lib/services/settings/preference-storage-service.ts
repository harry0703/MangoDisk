import type { ResourceSortPreferences } from '@/lib/models/system-resources/application-resource-list';
import { load, type Store } from '@tauri-apps/plugin-store';

import type { AppSettings } from '@/lib/models/settings/settings';
import type { StorageScopePreferences } from '@/lib/models/storage/storage-scope';
import type { CustomCleanupPreferences } from '@/lib/models/cleanup/custom-cleanup';
import type { ScanExclusionPreferences } from '@/lib/models/storage/storage-scan';
import type { AnalysisScanMode, AnalysisViewPreferences } from '@/lib/models/storage/analysis';
import type { AnalysisExportPreferences } from '@/lib/models/storage/analysis-export';

const SETTINGS_FILE_NAME = 'settings.json';
const SETTINGS_KEYS = {
  settings: 'settings',
  analysisViewPreferences: 'analysisViewPreferences',
  analysisExportPreferences: 'analysisExportPreferences',
  analysisScanMode: 'analysisScanMode',
  resourceSortPreferences: 'resourceSortPreferences',
  storageScopePreferences: 'storageScopePreferences',
  customCleanupPreferences: 'customCleanupPreferences',
  scanExclusionPreferences: 'scanExclusionPreferences',
  /** Read only during one-way migration to scanExclusionPreferences. */
  storageScanPreferences: 'storageScanPreferences',
  largeFilePreferences: 'largeFilePreferences',
} as const;

type SettingsKey = (typeof SETTINGS_KEYS)[keyof typeof SETTINGS_KEYS];

/**
 * Owns the single frontend settings document.
 *
 * Values remain unknown on reads so their owning domain parser validates every
 * persisted value before it enters application state.
 */
export class PreferenceStorageService {
  private static storePromise: Promise<Store> | null = null;
  private static mutationQueue: Promise<void> = Promise.resolve();
  private static analysisViewToSave: AnalysisViewPreferences | null = null;
  private static analysisViewSave: Promise<void> | null = null;

  static loadAnalysisExportPreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.analysisExportPreferences);
  }

  static saveAnalysisExportPreferences(value: AnalysisExportPreferences): Promise<void> {
    return this.write(SETTINGS_KEYS.analysisExportPreferences, { ...value });
  }

  static loadAnalysisScanMode(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.analysisScanMode);
  }

  static saveAnalysisScanMode(mode: AnalysisScanMode): Promise<void> {
    return this.write(SETTINGS_KEYS.analysisScanMode, mode);
  }

  static loadAnalysisViewPreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.analysisViewPreferences);
  }

  static saveAnalysisViewPreferences(value: AnalysisViewPreferences): Promise<void> {
    this.analysisViewToSave = { ...value };
    if (this.analysisViewSave) return this.analysisViewSave;
    const write = this.enqueueMutation(async store => {
      const latest = this.analysisViewToSave!;
      // Consume only queued updates. Changes during this write form one follow-up
      // behind the shared queue, so unrelated settings are never starved.
      this.analysisViewToSave = null;
      this.analysisViewSave = null;
      await store.set(SETTINGS_KEYS.analysisViewPreferences, latest);
      await store.save();
    }).catch(error => {
      // A store-load failure occurs before the callback consumes the request.
      if (this.analysisViewSave === write) {
        this.analysisViewSave = null;
        this.analysisViewToSave = null;
      }
      throw error;
    });
    this.analysisViewSave = write;
    return write;
  }

  static loadResourceSort(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.resourceSortPreferences);
  }

  static saveResourceSort(value: ResourceSortPreferences): Promise<void> {
    return this.write(SETTINGS_KEYS.resourceSortPreferences, value);
  }

  static loadSettings(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.settings);
  }

  static saveSettings(settings: AppSettings): Promise<void> {
    return this.write(SETTINGS_KEYS.settings, settings);
  }

  static saveSidebarExpanded(expanded: boolean, fallbackSettings: AppSettings): Promise<void> {
    return this.enqueueMutation(async store => {
      const saved = await store.get<unknown>(SETTINGS_KEYS.settings);
      if (saved != null && (typeof saved !== 'object' || Array.isArray(saved))) {
        throw new Error('Invalid app settings document');
      }
      // Startup can fall back to defaults after a transient read failure. Patch
      // the stored document so a navigation toggle cannot replace other choices.
      await store.set(SETTINGS_KEYS.settings, { ...(saved ?? fallbackSettings), sidebarExpanded: expanded });
      await store.save();
    });
  }

  static clearSettings(): Promise<void> {
    return this.remove(SETTINGS_KEYS.settings);
  }

  static loadStorageScopePreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.storageScopePreferences);
  }

  static saveStorageScopePreferences(preferences: StorageScopePreferences): Promise<void> {
    return this.write(SETTINGS_KEYS.storageScopePreferences, preferences);
  }

  static clearStorageScopePreferences(): Promise<void> {
    return this.remove(SETTINGS_KEYS.storageScopePreferences);
  }

  static loadCustomCleanupPreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.customCleanupPreferences);
  }

  static saveCustomCleanupPreferences(preferences: CustomCleanupPreferences): Promise<void> {
    return this.write(SETTINGS_KEYS.customCleanupPreferences, preferences);
  }

  static clearCustomCleanupPreferences(): Promise<void> {
    return this.remove(SETTINGS_KEYS.customCleanupPreferences);
  }

  static loadScanExclusionPreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.scanExclusionPreferences);
  }

  static saveScanExclusionPreferences(preferences: ScanExclusionPreferences): Promise<void> {
    return this.write(SETTINGS_KEYS.scanExclusionPreferences, preferences);
  }

  static loadLegacyStorageScanPreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.storageScanPreferences);
  }

  static loadLegacyLargeFilePreferences(): Promise<unknown | null> {
    return this.read(SETTINGS_KEYS.largeFilePreferences);
  }

  /** Publishes the scoped document and removes older keys in one serialized save. */
  static migrateScanExclusionPreferences(preferences: ScanExclusionPreferences): Promise<void> {
    return this.enqueueMutation(async store => {
      await store.set(SETTINGS_KEYS.scanExclusionPreferences, preferences);
      await store.delete(SETTINGS_KEYS.storageScanPreferences);
      await store.delete(SETTINGS_KEYS.largeFilePreferences);
      await store.save();
    });
  }

  static clearScanExclusionPreferences(): Promise<void> {
    return this.remove(SETTINGS_KEYS.scanExclusionPreferences);
  }

  private static store(): Promise<Store> {
    this.storePromise ??= load(SETTINGS_FILE_NAME, { autoSave: false }).catch(error => {
      this.storePromise = null;
      throw error;
    });
    return this.storePromise;
  }

  private static async read(key: SettingsKey): Promise<unknown | null> {
    await this.mutationQueue.catch(() => undefined);
    return (await this.store()).get<unknown>(key).then(value => value ?? null);
  }

  private static write(key: SettingsKey, value: unknown): Promise<void> {
    return this.enqueueMutation(async store => {
      await store.set(key, value);
      await store.save();
    });
  }

  private static remove(key: SettingsKey): Promise<void> {
    return this.enqueueMutation(async store => {
      await store.delete(key);
      await store.save();
    });
  }

  private static enqueueMutation(operation: (store: Store) => Promise<void>): Promise<void> {
    const mutation = this.mutationQueue.catch(() => undefined).then(async () => operation(await this.store()));
    this.mutationQueue = mutation;
    return mutation;
  }
}
