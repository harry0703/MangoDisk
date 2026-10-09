import { PreferenceStorageService } from '@/lib/services/preference-storage-service';
import { defaultResourceSort, parseResourceSort } from '@/lib/utils/application-resource-list';
import type { ResourceMetric, ResourceSort } from '@/lib/models/application-resource-list';
import { emptyReadings, retainSampleValue } from '@/lib/utils/system-resources';
import type { MetricId } from '@/lib/models/system-resources';
import { defineStore } from 'pinia';

import type { MemoryReleaseResult, ResidentReading } from '@/lib/models/resident';
import { ResidentService } from '@/lib/services/resident-service';
import { LoggerService } from '@/lib/services/logger-service';

export const useTrayPanelStore = defineStore('tray-panel', {
  state: () => ({
    reading: { revision: 0, ...emptyReadings() } as ResidentReading,
    selectedMetric: 'cpu' as MetricId,
    sortPreferences: defaultResourceSort(),
    sortRevision: { cpu: 0, memory: 0 },
    sortLoading: null as Promise<void> | null,
    sortLoaded: false,
    error: false,
    refreshing: false,
    releasing: false,
    releaseResult: null as MemoryReleaseResult | null,
  }),
  actions: {
    loadSort(): Promise<void> {
      if (this.sortLoaded) return Promise.resolve();
      if (this.sortLoading) return this.sortLoading;
      const revision = { ...this.sortRevision };
      this.sortLoading = (async () => {
        try {
          const stored = parseResourceSort(await PreferenceStorageService.loadResourceSort());
          // A click only supersedes that metric's saved preference, not the other tab's.
          this.sortPreferences = {
            ...stored,
            cpu: revision.cpu === this.sortRevision.cpu ? stored.cpu : this.sortPreferences.cpu,
            memory: revision.memory === this.sortRevision.memory ? stored.memory : this.sortPreferences.memory,
          };
        } catch {
          LoggerService.warn('monitoring', 'resource_sort_load_failed');
        } finally {
          this.sortLoaded = true;
          this.sortLoading = null;
        }
      })();
      return this.sortLoading;
    },
    async sortResources(metric: ResourceMetric, column: ResourceSort['column']) {
      const loading = this.loadSort();
      const previous = this.sortPreferences[metric];
      this.sortRevision[metric] += 1;
      this.sortPreferences = {
        ...this.sortPreferences,
        [metric]: {
          column,
          direction:
            previous.column === column
              ? previous.direction === 'ascending'
                ? 'descending'
                : 'ascending'
              : column === 'name'
                ? 'ascending'
                : 'descending',
        },
      };
      try {
        // Render immediately, but preserve the untouched tab before persisting both preferences.
        await loading;
        await PreferenceStorageService.saveResourceSort(this.sortPreferences);
      } catch {
        LoggerService.warn('monitoring', 'resource_sort_save_failed');
      }
    },
    accept(reading: ResidentReading) {
      // A cached IPC response can arrive after a newer native event. Never move
      // backwards or show an incompatible protocol as a plausible measurement.
      if (reading.schemaVersion !== 16 || (reading.memory.value && reading.memory.value.schemaVersion !== 4)) {
        this.error = true;
        return;
      }
      if (reading.revision < this.reading.revision) return;
      this.reading = {
        ...reading,
        cpuProcesses: retainSampleValue(this.reading.cpuProcesses, reading.cpuProcesses),
        memoryProcesses: retainSampleValue(this.reading.memoryProcesses, reading.memoryProcesses),
      };
      this.error = false;
      this.refreshing = false;
    },
    async load() {
      try {
        this.accept(await ResidentService.reading());
      } catch {
        this.fail('monitoring_read_failed');
      }
    },
    async refresh() {
      if (this.refreshing) return;
      this.refreshing = true;
      try {
        await ResidentService.refresh();
        await this.load();
      } catch {
        this.refreshing = false;
        this.fail('monitoring_refresh_failed');
      } finally {
        this.refreshing = false;
      }
    },
    async releaseMemory() {
      if (this.releasing) return;
      this.releasing = true;
      this.releaseResult = null;
      try {
        const result = await ResidentService.releaseMemory();
        if (result.schemaVersion !== 1) throw new Error('unsupported memory release response');
        this.releaseResult = result;
        await this.refresh();
      } catch {
        this.releaseResult = { schemaVersion: 1, status: 'failed', observedReductionBytes: null };
        LoggerService.warn('monitoring', 'memory_release_request_failed');
      } finally {
        this.releasing = false;
      }
    },
    fail(event: string) {
      this.error = true;
      LoggerService.warn('monitoring', event);
    },
  },
});
