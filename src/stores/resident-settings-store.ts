import { defineStore } from 'pinia';
import type { ResidentPreferences } from '@/lib/models/resident';
import { ResidentService } from '@/lib/services/resident-service';
import { LoggerService } from '@/lib/services/logger-service';

function copy(value: ResidentPreferences): ResidentPreferences {
  return { ...value, metrics: value.metrics.map(metric => ({ ...metric })) };
}

export const useResidentSettingsStore = defineStore('resident-settings', {
  state: () => ({
    preferences: null as ResidentPreferences | null,
    draft: null as ResidentPreferences | null,
    loading: false,
    saving: false,
    error: false,
    editRevision: 0,
  }),
  actions: {
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
        if (value.schemaVersion !== 8) throw new Error('unsupported resident preferences');
        this.preferences = value;
        this.draft = copy(value);
      } catch {
        if (revision !== this.editRevision) return;
        this.error = true;
        LoggerService.warn('resident', 'preferences_load_failed');
      } finally {
        this.loading = false;
      }
    },
    async change(patch: Partial<ResidentPreferences>) {
      if (!this.draft || !this.preferences) return;
      this.draft = copy({ ...this.draft, ...patch });
      this.editRevision += 1;
      if (this.saving) return;
      this.saving = true;
      this.error = false;
      try {
        while (this.draft && this.preferences) {
          const revision = this.editRevision;
          const requested = copy({ ...this.draft, revision: this.preferences.revision });
          try {
            const committed = await ResidentService.savePreferences(requested);
            if (committed.schemaVersion !== 8) throw new Error('unsupported resident preferences');
            this.preferences = committed;
            if (revision === this.editRevision) {
              this.draft = copy(committed);
              break;
            }
            // Merge rapid edits into the next write using the returned native
            // revision. No older request is allowed to race the latest choice.
          } catch {
            this.error = true;
            try {
              this.preferences = await ResidentService.preferences();
            } catch {
              /* Keep last committed state. */
            }
            this.draft = this.preferences ? copy(this.preferences) : null;
            LoggerService.warn('resident', 'preferences_save_rejected');
            break;
          }
        }
      } finally {
        this.saving = false;
      }
    },
  },
});
