import { defineStore } from 'pinia';
import type { ResidentPreferences } from '@/lib/models/resident/resident';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { parseCommandError, parseCommandErrorReason } from '@/lib/utils/diagnostics/error';

type WriteIntent = 'edit' | 'restoreDefaults';

function retainDisplayState(value: ResidentPreferences, current: ResidentPreferences): ResidentPreferences {
  return copy({
    ...value,
    revision: current.revision,
    enabled: current.enabled,
    windowsDisplayMode: current.windowsDisplayMode,
    taskbarMonitor: current.taskbarMonitor,
    macosDisplayMode: current.macosDisplayMode,
    linuxDisplayMode: current.linuxDisplayMode,
  });
}

/** Normal edits rebase only disjoint fields; restoring defaults replaces all options except display state. */
function rebase(
  base: ResidentPreferences,
  draft: ResidentPreferences,
  latest: ResidentPreferences,
  intent: WriteIntent
): ResidentPreferences | null {
  if (latest.schemaVersion !== base.schemaVersion || latest.revision <= base.revision) return null;
  if (intent === 'restoreDefaults') return retainDisplayState(draft, latest);
  const patch: Partial<ResidentPreferences> = {};
  for (const key of Object.keys(draft) as (keyof ResidentPreferences)[]) {
    if (key === 'revision' || key === 'schemaVersion') continue;
    const before = JSON.stringify(base[key]);
    const desired = JSON.stringify(draft[key]);
    if (before === desired) continue;
    const current = JSON.stringify(latest[key]);
    if (current !== before && current !== desired) return null;
    Object.assign(patch, { [key]: draft[key] });
  }
  return copy({ ...latest, ...patch });
}

function copy(value: ResidentPreferences): ResidentPreferences {
  return {
    ...value,
    floating: { ...value.floating },
    publicNetwork: { ...value.publicNetwork },
    fanIds: [...value.fanIds],
    metrics: value.metrics.map(metric => ({ ...metric })),
    colorRules: value.colorRules.map(rule => ({ ...rule, rules: rule.rules.map(condition => ({ ...condition })) })),
  };
}

export const useResidentSettingsStore = defineStore('resident-settings', {
  state: () => ({
    preferences: null as ResidentPreferences | null,
    draft: null as ResidentPreferences | null,
    loading: false,
    saving: false,
    resetting: false,
    error: false,
    editRevision: 0,
  }),
  actions: {
    async restoreDefaults(): Promise<boolean> {
      if (!this.draft || this.loading || this.saving || this.resetting) return false;
      this.resetting = true;
      this.error = false;
      try {
        const defaults = await ResidentService.defaultPreferences();
        if (defaults.schemaVersion !== 42) throw new Error('unsupported resident defaults');
        if (!this.draft) return false;
        if (this.saving) throw new Error('resident preference write in progress');
        // Native defaults own platform-specific order and options. Keep the
        // current display destinations; floating placement is stored separately.
        return await this.change(retainDisplayState(defaults, this.draft), 'restoreDefaults');
      } catch (error) {
        this.error = true;
        LoggerService.warn('resident', 'preferences_defaults_failed', { error });
        return false;
      } finally {
        this.resetting = false;
      }
    },
    async load() {
      if (this.loading || this.saving) return;
      this.loading = true;
      this.error = false;
      const revision = this.editRevision;
      try {
        const value = await ResidentService.preferences();
        // A retry can overlap an edit made from the retained draft. Its older
        // response must not replace the newer optimistic or committed state.
        if (revision !== this.editRevision) return;
        if (value.schemaVersion !== 42) throw new Error('unsupported resident preferences');
        this.preferences = value;
        this.draft = copy(value);
      } catch (error) {
        if (revision !== this.editRevision) return;
        this.error = true;
        LoggerService.warn('resident', 'preferences_load_failed', { error });
      } finally {
        this.loading = false;
      }
    },
    async change(patch: Partial<ResidentPreferences>, intent: WriteIntent = 'edit'): Promise<boolean> {
      if (!this.draft || !this.preferences) return false;
      this.draft = copy({ ...this.draft, ...patch });
      this.editRevision += 1;
      if (this.saving) return false;
      this.saving = true;
      this.error = false;
      let recoveredConflict = false;
      let completed = false;
      try {
        while (this.draft && this.preferences) {
          const revision = this.editRevision;
          const requested = copy({ ...this.draft, revision: this.preferences.revision });
          try {
            const committed = await ResidentService.savePreferences(requested);
            if (committed.schemaVersion !== 42) throw new Error('unsupported resident preferences');
            this.preferences = committed;
            if (revision === this.editRevision) {
              this.draft = copy(committed);
              completed = true;
              break;
            }
            // Merge rapid edits into the next write using the returned native
            // revision. No older request is allowed to race the latest choice.
          } catch (error) {
            try {
              const latest = await ResidentService.preferences();
              const commandError = parseCommandError(error);
              const merged: ResidentPreferences | null =
                !recoveredConflict &&
                commandError?.details.operation === 'resident_save_preferences' &&
                parseCommandErrorReason(commandError) === 'preferencesConflict' &&
                this.draft
                  ? rebase(this.preferences, this.draft, latest, intent)
                  : null;
              if (merged) {
                LoggerService.info('resident', 'preferences_conflict_retry', {
                  requestedRevision: requested.revision,
                  currentRevision: latest.revision,
                  outcome: intent === 'restoreDefaults' ? 'retry_full_defaults_once' : 'retry_disjoint_edits_once',
                });
                this.preferences = latest;
                this.draft = merged;
                recoveredConflict = true;
                continue;
              }
              this.preferences = latest;
            } catch {
              /* Keep last committed state. */
            }
            this.error = true;
            this.draft = this.preferences ? copy(this.preferences) : null;
            LoggerService.warn('resident', 'preferences_save_rejected', { revision: requested.revision, error });
            break;
          }
        }
      } finally {
        this.saving = false;
      }
      return completed;
    },
  },
});
