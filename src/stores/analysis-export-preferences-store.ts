import { defineStore } from 'pinia';

import type { AnalysisExportPreferences } from '@/lib/models/storage/analysis-export';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import * as ExportPreferenceUtils from '@/lib/utils/storage/analysis-export-preference';

type PreferenceKey = Exclude<keyof AnalysisExportPreferences, 'schemaVersion'>;
type Changes = Partial<Pick<AnalysisExportPreferences, PreferenceKey>>;
const loads = new WeakMap<object, { promise: Promise<void>; edited: Changes }>();

export const useAnalysisExportPreferencesStore = defineStore('analysis-export-preferences', {
  state: () => ({ preferences: ExportPreferenceUtils.defaults(), initialized: false }),
  actions: {
    async initialize() {
      if (this.initialized) return;
      const pending = loads.get(this);
      if (pending) return pending.promise;
      const load = { promise: Promise.resolve(), edited: {} as Changes };
      load.promise = (async () => {
        try {
          const saved = ExportPreferenceUtils.parse(await PreferenceStorageService.loadAnalysisExportPreferences());
          // Startup restoration cannot overwrite choices already made in this session.
          this.preferences = { ...saved, ...load.edited };
        } catch (error) {
          LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportPreferencesLoadFailed, {
            error,
            outcome: 'use_session_preferences',
          });
        } finally {
          this.initialized = true;
          loads.delete(this);
        }
        if (Object.keys(load.edited).length) this.persist();
      })();
      loads.set(this, load);
      return load.promise;
    },
    setDepth(value: unknown) {
      if (ExportPreferenceUtils.isDepth(value)) this.update('maxDepth', value);
    },
    setEntryLimit(value: unknown) {
      if (ExportPreferenceUtils.isEntryLimit(value)) this.update('maxEntries', value);
    },
    setFullPaths(value: unknown) {
      if (typeof value === 'boolean') this.update('includeFullPaths', value);
    },
    setFormat(value: unknown) {
      if (ExportPreferenceUtils.isFormat(value)) this.update('format', value);
    },
    update<Key extends PreferenceKey>(key: Key, value: AnalysisExportPreferences[Key]) {
      void this.initialize();
      const pending = loads.get(this);
      if (pending) pending.edited = { ...pending.edited, [key]: value };
      if (this.preferences[key] === value) return;
      this.preferences = { ...this.preferences, [key]: value };
      if (this.initialized) this.persist();
    },
    persist() {
      const preferences = { ...this.preferences };
      void PreferenceStorageService.saveAnalysisExportPreferences(preferences).catch(error => {
        LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportPreferencesSaveFailed, {
          preferences,
          error,
          outcome: 'kept_in_session',
        });
      });
    },
  },
});
