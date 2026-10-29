import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { NetworkAddressSnapshot, NetworkAddressRequest } from '@/lib/models/system-resources/network-addresses';

export class NetworkAddressService {
  static snapshot(request: NetworkAddressRequest): Promise<NetworkAddressSnapshot> {
    return invoke('monitoring_network_addresses', { request });
  }
  static onChanged(callback: (snapshot: NetworkAddressSnapshot) => void): Promise<UnlistenFn> {
    return listen<NetworkAddressSnapshot>('resident-public-network', event => callback(event.payload));
  }
}
