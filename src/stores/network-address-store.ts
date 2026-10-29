import { defineStore } from 'pinia';
import { NetworkAddressService } from '@/lib/services/system-resources/network-address-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import type {
  AddressFamily,
  AddressProbe,
  NetworkAddress,
  NetworkAddressSnapshot,
  NetworkAddressRequest,
} from '@/lib/models/system-resources/network-addresses';

function emptyProbe(): AddressProbe {
  return { address: null, loading: false, failed: false };
}
export const useNetworkAddressStore = defineStore('network-addresses', {
  state: () => ({
    probes: { ipv4: emptyProbe(), ipv6: emptyProbe() },
    revision: -1,
    generation: 0,
    active: false,
    contextPending: null as Promise<void> | null,
    refreshQueued: false,
    unsubscribe: null as (() => void) | null,
    contextFailed: false,
    validating: false,
    updatedAt: 0,
  }),
  getters: {
    busy: state => state.probes.ipv4.loading || state.probes.ipv6.loading,
    results: state => {
      // Returned address families remain authoritative when a proxy changes the transport family.
      const found = (family: 4 | 6, preferred: AddressFamily) => {
        const candidates = [state.probes[preferred], state.probes[preferred === 'ipv4' ? 'ipv6' : 'ipv4']];
        const matching = candidates.filter(probe => probe.address?.family === family);
        const selected = matching.find(probe => !probe.failed) ?? matching[0];
        return { address: selected?.address ?? null, stale: selected?.failed ?? false };
      };
      return { ipv4: found(4, 'ipv4'), ipv6: found(6, 'ipv6') };
    },
    addresses(): Record<AddressFamily, NetworkAddress | null> {
      return { ipv4: this.results.ipv4.address, ipv6: this.results.ipv6.address };
    },
  },
  actions: {
    clear() {
      void this.setActive(false);
      this.probes = { ipv4: emptyProbe(), ipv6: emptyProbe() };
      this.contextFailed = false;
      this.validating = false;
      this.updatedAt = 0;
    },
    accept(snapshot: NetworkAddressSnapshot) {
      if (!this.active) return;
      if (snapshot.schemaVersion !== 1 || !Number.isSafeInteger(snapshot.revision))
        throw new Error('unsupported public address snapshot');
      if (snapshot.revision < this.revision) return;
      this.revision = snapshot.revision;
      this.probes = { ipv4: { ...snapshot.probes.ipv4 }, ipv6: { ...snapshot.probes.ipv6 } };
      this.contextFailed = snapshot.contextFailed;
      this.updatedAt = snapshot.updatedAtMs;
    },
    async setActive(active: boolean) {
      this.active = active;
      const generation = ++this.generation;
      this.unsubscribe?.();
      this.unsubscribe = null;
      this.refreshQueued = false;
      if (!active) {
        this.validating = false;
        return;
      }
      this.validating = true;
      try {
        const stop = await NetworkAddressService.onChanged(snapshot => {
          if (this.generation !== generation || !this.active) return;
          try {
            this.accept(snapshot);
          } catch {
            this.contextFailed = true;
          }
        });
        if (this.generation !== generation || !this.active) {
          stop();
          return;
        }
        this.unsubscribe = stop;
      } catch (error) {
        LoggerService.warn('monitoring', 'network_address_subscription_failed', { error });
      }
      if (this.generation !== generation || !this.active) return;
      if (this.contextPending) await this.contextPending;
      if (this.generation !== generation || !this.active) return;
      await this.read('open');
      if (this.generation === generation) this.validating = false;
    },
    read(request: NetworkAddressRequest): Promise<void> {
      if (!this.active) return Promise.resolve();
      if (this.contextPending) {
        if (request === 'refresh') this.refreshQueued = true;
        return this.contextPending;
      }
      const generation = this.generation;
      this.contextPending = (async () => {
        try {
          const snapshot = await NetworkAddressService.snapshot(request);
          if (generation === this.generation) this.accept(snapshot);
        } catch (error) {
          if (generation !== this.generation || !this.active) return;
          this.contextFailed = true;
          LoggerService.warn('monitoring', 'network_address_snapshot_failed', { error });
        } finally {
          this.contextPending = null;
          if (this.active && this.refreshQueued) {
            this.refreshQueued = false;
            void this.read('refresh');
          }
        }
      })();
      return this.contextPending;
    },
    checkContext(): Promise<void> {
      return this.read('observe');
    },
    refresh(): Promise<void> {
      return this.read('refresh');
    },
  },
});
