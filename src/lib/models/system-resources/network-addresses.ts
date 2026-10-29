export type AddressFamily = 'ipv4' | 'ipv6';
export interface NetworkAddress {
  schemaVersion: 1;
  ip: string;
  family: 4 | 6;
  countryCode: string | null;
}
export interface AddressProbe {
  address: NetworkAddress | null;
  loading: boolean;
  failed: boolean;
}
export const ADDRESS_FAMILIES = ['ipv4', 'ipv6'] as const;
export const NETWORK_CONTEXT_INTERVAL_MS = 5000;
export type NetworkAddressRequest = 'observe' | 'open' | 'refresh';
export interface NetworkAddressSnapshot {
  schemaVersion: 1;
  revision: number;
  probes: Record<AddressFamily, AddressProbe>;
  contextFailed: boolean;
  updatedAtMs: number;
}
export interface PublicNetworkPreferences {
  enabled: boolean;
}
