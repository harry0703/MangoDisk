import { defineStore } from 'pinia';
import type {
  WindowsMonitoringObservation,
  WindowsMonitoringOutcome,
} from '@/lib/models/system-resources/windows-monitoring';
import { WindowsMonitoringService } from '@/lib/services/system-resources/windows-monitoring-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

export const useWindowsMonitoringStore = defineStore('windows-monitoring', {
  state: () => ({
    observation: null as WindowsMonitoringObservation | null,
    outcome: null as WindowsMonitoringOutcome | null,
    nativeCode: null as string | null,
    loading: false,
    busy: false,
    revision: 0,
  }),
  getters: {
    statusLabel:
      state =>
      (enabled: boolean): string | null => {
        if (!state.observation) return null;
        const labels = {
          notInstalled: 'windowsMonitoring.states.notInstalled',
          ready: enabled ? 'windowsMonitoring.states.ready' : 'windowsMonitoring.states.notEnabled',
          unavailable: 'windowsMonitoring.states.unavailable',
          incompatible: 'windowsMonitoring.states.incompatible',
          unsupportedHardware: 'windowsMonitoring.states.unsupportedHardware',
        };
        return labels[state.observation.state];
      },
    needsAttention: state => ['unavailable', 'incompatible'].includes(state.observation?.state ?? ''),
  },
  actions: {
    async load() {
      if (this.loading || this.busy) return;
      const revision = ++this.revision;
      this.loading = true;
      this.outcome = null;
      this.nativeCode = null;
      try {
        const observation = await WindowsMonitoringService.observe();
        if (this.revision === revision) this.observation = observation;
      } catch (error) {
        if (this.revision === revision)
          this.observation = { state: 'unavailable', driverAvailable: false, serviceInstalled: false };
        LoggerService.warn('monitoring', 'windows_capability_observation_failed', { error });
      } finally {
        this.loading = false;
      }
    },
    async setup(remove = false, removeDriver = false): Promise<WindowsMonitoringOutcome> {
      if (this.busy) return 'busy';
      this.revision++;
      this.busy = true;
      this.outcome = null;
      this.nativeCode = null;
      try {
        const result = await (remove
          ? WindowsMonitoringService.uninstall(removeDriver)
          : WindowsMonitoringService.install());
        this.observation = result.observation;
        this.nativeCode = result.nativeCode ?? null;
        this.outcome = result.outcome;
      } catch (error) {
        this.outcome = remove ? 'removalFailed' : 'installationFailed';
        LoggerService.warn('monitoring', 'windows_capability_setup_failed', { error });
      } finally {
        this.busy = false;
      }
      return this.outcome;
    },
  },
});
