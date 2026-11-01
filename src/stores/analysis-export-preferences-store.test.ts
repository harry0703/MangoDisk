import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AnalysisExportPreferences } from '@/lib/models/storage/analysis-export';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { defaults } from '@/lib/utils/storage/analysis-export-preference';
import { useAnalysisExportPreferencesStore } from './analysis-export-preferences-store';

describe('analysis export preference lifecycle', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.spyOn(PreferenceStorageService, 'loadAnalysisExportPreferences').mockResolvedValue(null);
    vi.spyOn(PreferenceStorageService, 'saveAnalysisExportPreferences').mockResolvedValue();
    vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
  });
  afterEach(() => vi.restoreAllMocks());

  it('loads once without persisting defaults or unchanged options', async () => {
    const store = useAnalysisExportPreferencesStore();
    await Promise.all([store.initialize(), store.initialize()]);
    await store.initialize();
    store.setDepth(2);
    store.setEntryLimit(500);
    store.setFullPaths(false);
    store.setFormat('json');
    store.setFormat('csv');
    for (const value of [7, NaN, '6']) store.setDepth(value);
    for (const value of [-1, 7, '500']) store.setEntryLimit(value);
    store.setFullPaths('true');
    expect(store.preferences).toEqual(defaults());
    expect(PreferenceStorageService.loadAnalysisExportPreferences).toHaveBeenCalledOnce();
    expect(PreferenceStorageService.saveAnalysisExportPreferences).not.toHaveBeenCalled();
  });

  it('restores all three options in a new application session', async () => {
    let saved: AnalysisExportPreferences | null = null;
    vi.mocked(PreferenceStorageService.loadAnalysisExportPreferences).mockImplementation(async () => saved);
    vi.mocked(PreferenceStorageService.saveAnalysisExportPreferences).mockImplementation(async value => {
      saved = { ...value };
    });
    const store = useAnalysisExportPreferencesStore();
    await store.initialize();
    store.setDepth(6);
    store.setEntryLimit(0);
    store.setFullPaths(true);
    store.setFormat('markdown');
    setActivePinia(createPinia());
    const restarted = useAnalysisExportPreferencesStore();
    await restarted.initialize();
    expect(restarted.preferences).toEqual({
      schemaVersion: 1,
      maxDepth: 6,
      maxEntries: 0,
      includeFullPaths: true,
      format: 'markdown',
    });
  });

  it('preserves early user choices while restoring untouched fields from a delayed read', async () => {
    let restore!: (value: unknown) => void;
    vi.mocked(PreferenceStorageService.loadAnalysisExportPreferences).mockImplementation(
      () => new Promise(resolve => (restore = resolve))
    );
    const store = useAnalysisExportPreferencesStore();
    const pending = store.initialize();
    store.setDepth(2);
    store.setEntryLimit(1000);
    expect(store.preferences.maxEntries).toBe(1000);
    expect(PreferenceStorageService.saveAnalysisExportPreferences).not.toHaveBeenCalled();
    restore({ schemaVersion: 1, maxDepth: 5, maxEntries: 0, includeFullPaths: true });
    await pending;
    expect(store.preferences).toEqual({
      schemaVersion: 1,
      maxDepth: 2,
      maxEntries: 1000,
      includeFullPaths: true,
      format: 'json',
    });
    expect(PreferenceStorageService.saveAnalysisExportPreferences).toHaveBeenCalledExactlyOnceWith(store.preferences);
  });

  it('keeps selection immediate during blocked writes and recovers after storage failures', async () => {
    vi.mocked(PreferenceStorageService.loadAnalysisExportPreferences).mockRejectedValueOnce(new Error('read failed'));
    const store = useAnalysisExportPreferencesStore();
    await store.initialize();
    let reject!: (error: Error) => void;
    vi.mocked(PreferenceStorageService.saveAnalysisExportPreferences).mockImplementationOnce(
      () => new Promise((_, fail) => (reject = fail))
    );
    store.setDepth(6);
    expect(store.preferences.maxDepth).toBe(6);
    reject(new Error('disk full'));
    await Promise.resolve();
    expect(store.preferences.maxDepth).toBe(6);
    expect(LoggerService.warn).toHaveBeenCalledWith(
      'analysis',
      LOG_EVENTS.analysisExportPreferencesSaveFailed,
      expect.objectContaining({ outcome: 'kept_in_session' })
    );
    store.setEntryLimit(2000);
    expect(PreferenceStorageService.saveAnalysisExportPreferences).toHaveBeenLastCalledWith({
      ...defaults(),
      maxDepth: 6,
      maxEntries: 2000,
    });
  });
});
