// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, it, expect, vi } from 'vitest';
import { useNetworkAddressStore } from '@/stores/network-address-store';
import { NetworkAddressService } from '@/lib/services/system-resources/network-address-service';
import { ClipboardService } from '@/lib/services/shell/clipboard-service';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import NetworkAddresses from './md-network-addresses.vue';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import { preferencesFixture } from '@/tests/fixtures/resident';
import { ResidentService } from '@/lib/services/resident/resident-service';
import type { NetworkAddressSnapshot } from '@/lib/models/system-resources/network-addresses';
vi.mock('@/lib/services/resident/resident-service', () => ({
  ResidentService: { preferences: vi.fn(), savePreferences: vi.fn() },
}));
vi.mock('@/lib/services/system-resources/network-address-service', () => ({
  NetworkAddressService: { snapshot: vi.fn(), onChanged: vi.fn() },
}));
vi.mock('@/lib/services/shell/clipboard-service', () => ({ ClipboardService: { writeText: vi.fn() } }));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
let notify: (snapshot: NetworkAddressSnapshot) => void;
function cached(revision = 1): NetworkAddressSnapshot {
  return {
    schemaVersion: 1,
    revision,
    probes: {
      ipv4: {
        address: { schemaVersion: 1, ip: '203.0.113.1', family: 4, countryCode: 'SG' },
        loading: false,
        failed: false,
      },
      ipv6: {
        address: { schemaVersion: 1, ip: '2001:db8::1', family: 6, countryCode: 'SG' },
        loading: false,
        failed: false,
      },
    },
    contextFailed: false,
    updatedAtMs: 1000,
  };
}
const mounted: ReturnType<typeof mount>[] = [];
function render(active = false, enabled = true) {
  const pinia = createPinia();
  const settings = useResidentSettingsStore(pinia);
  settings.preferences = { ...preferencesFixture(), publicNetwork: { enabled } };
  settings.draft = { ...preferencesFixture(), publicNetwork: { enabled } };
  const i18n = createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': en, 'zh-CN': zh } });
  const wrapper = mount(NetworkAddresses, {
    attachTo: document.body,
    props: { active },
    global: {
      plugins: [pinia, i18n],
      stubs: { MdTooltip: { props: ['text'], template: '<div :data-hint="text"><slot/><slot name="content"/></div>' } },
    },
  });
  mounted.push(wrapper);
  return { wrapper, store: useNetworkAddressStore(pinia), i18n, settings };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(ResidentService.savePreferences).mockImplementation(async preferences => ({
    ...preferences,
    revision: preferences.revision + 1,
  }));
  vi.mocked(ResidentService.preferences).mockResolvedValue(preferencesFixture());
  vi.mocked(NetworkAddressService.onChanged).mockImplementation(async callback => {
    notify = callback;
    return vi.fn<() => void>();
  });
  vi.mocked(NetworkAddressService.snapshot).mockImplementation(async () => cached());
});
afterEach(() => {
  mounted.splice(0).forEach(wrapper => wrapper.unmount());
  vi.useRealTimers();
});
describe('public egress presentation', () => {
  it('keeps settings accessible when disabled and saves an explicit enable from its dialog', async () => {
    const { wrapper } = render(true, false);
    await flushPromises();
    expect(NetworkAddressService.snapshot).not.toHaveBeenCalled();
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('Not enabled');
    expect(wrapper.find('[data-family=ipv6]').exists()).toBe(false);
    expect(wrapper.find('button[aria-label="Refresh"]').exists()).toBe(false);
    await wrapper.get('button[aria-label="Network settings"]').trigger('click');
    await flushPromises();
    expect(document.querySelector('[role="dialog"]')?.querySelectorAll('[role="switch"]')).toHaveLength(1);
    expect(document.querySelector('[role="dialog"]')?.querySelector('[role="combobox"]')).toBeNull();
    expect(document.querySelector('#network-public-enabled')?.getAttribute('aria-checked')).toBe('false');
    document.querySelector<HTMLButtonElement>('#network-public-enabled')!.click();
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenCalledWith(
      expect.objectContaining({ publicNetwork: { enabled: true } })
    );
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(1);
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('203.0.113.1');
    await wrapper.setProps({ active: false });
    await flushPromises();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
  it('does not query before preferences load and discards pending results after disabling', async () => {
    let resolve!: (snapshot: NetworkAddressSnapshot) => void;
    vi.mocked(NetworkAddressService.snapshot).mockReturnValue(
      new Promise(accept => {
        resolve = accept;
      })
    );
    const { wrapper, store, settings } = render(true);
    await flushPromises();
    settings.draft!.publicNetwork.enabled = false;
    await flushPromises();
    expect(store.addresses.ipv4).toBeNull();
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('Not enabled');
    resolve(cached());
    await flushPromises();
    expect(store.addresses.ipv4).toBeNull();
    expect(store.busy).toBe(false);
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('Not enabled');
    window.dispatchEvent(new Event('online'));
    await flushPromises();
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(1);
    settings.preferences = null;
    settings.draft = null;
    await flushPromises();
    expect(wrapper.find('dl').exists()).toBe(false);
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(1);
  });
  it('stops checks while closed and reads the native cache when reopened', async () => {
    vi.useFakeTimers();
    const { wrapper } = render(true, true);
    await flushPromises();
    await wrapper.setProps({ active: false });
    await vi.advanceTimersByTimeAsync(120000);
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(1);
    const fresh = cached(10);
    fresh.probes.ipv4.address!.ip = '203.0.113.10';
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(fresh);
    await wrapper.setProps({ active: true });
    await flushPromises();
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('203.0.113.10');
    expect(vi.mocked(NetworkAddressService.snapshot).mock.calls.map(([request]) => request)).toEqual(['open', 'open']);
  });
  it('restores the switch when saving fails', async () => {
    const { wrapper, settings } = render(true);
    await flushPromises();
    await wrapper.get('button[aria-label="Network settings"]').trigger('click');
    await flushPromises();
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('injected persistence failure'));
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce({
      ...preferencesFixture(),
      revision: 1,
      publicNetwork: { enabled: true },
    });
    document.querySelector<HTMLButtonElement>('#network-public-enabled')!.click();
    await flushPromises();
    expect(document.querySelector('#network-public-enabled')?.getAttribute('aria-checked')).toBe('true');
    expect(document.querySelector('[role="alert"]')?.textContent).toContain('Could not update settings');
    expect(settings.preferences?.publicNetwork).toEqual({ enabled: true });
  });
  it('shows localized country names on the copyable IP without flags', async () => {
    const { wrapper, i18n } = render(true);
    await flushPromises();
    const ip = wrapper.get('button[aria-label="Copy: Public IP IPv4"]');
    expect(ip.text()).toBe('203.0.113.1');
    expect(ip.element.closest('[data-hint]')?.getAttribute('data-hint')).toContain('SG · Singapore');
    expect(wrapper.text()).not.toMatch(/[\u{1F1E6}-\u{1F1FF}]/u);
    i18n.global.locale.value = 'zh-CN';
    await flushPromises();
    const translated = wrapper.get('button[aria-label="复制: 公网 IP IPv4"]');
    expect(translated.element.closest('[data-hint]')?.getAttribute('data-hint')).toContain('SG · 新加坡');
    await translated.trigger('click');
    await flushPromises();
    expect(ClipboardService.writeText).toHaveBeenCalledWith('203.0.113.1');
    expect(wrapper.get('button[aria-label="已复制: 公网 IP IPv4"]').attributes('disabled')).toBeUndefined();
  });
  it('preserves IP display and copying when country information is missing', async () => {
    const value = cached();
    value.probes.ipv4 = { address: null, loading: false, failed: true };
    value.probes.ipv6.address = {
      schemaVersion: 1,
      ip: '2001:db8:1234:5678:abcd:abcd:abcd:abcd',
      family: 6,
      countryCode: null,
    };
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(value);
    const { wrapper } = render(true);
    await flushPromises();
    const ip = wrapper.get('button[aria-label="Copy: Public IP IPv6"]');
    expect(ip.text()).toContain('2001:db8:1234:5678:abcd:abcd:abcd:abcd');
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('Temporarily unavailable');
    await ip.trigger('click');
    await flushPromises();
    expect(ClipboardService.writeText).toHaveBeenCalledWith('2001:db8:1234:5678:abcd:abcd:abcd:abcd');
  });
  it('keeps IPv4 visible and hides IPv6 without an address while loading or failed', async () => {
    const value = cached();
    value.probes.ipv6 = { address: null, loading: true, failed: false };
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(value);
    const { wrapper, store } = render(true);
    await flushPromises();
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('203.0.113.1');
    expect(wrapper.find('[data-family=ipv6]').exists()).toBe(false);
    const failed = cached(2);
    failed.probes.ipv6 = { address: null, loading: false, failed: true };
    notify(failed);
    await flushPromises();
    expect(wrapper.find('[data-family=ipv6]').exists()).toBe(false);
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('203.0.113.1');
    expect(wrapper.text()).not.toContain('unsupported');
    expect(store.busy).toBe(false);
  });
  it('starts only while visible and stops local checks when hidden or unmounted', async () => {
    vi.useFakeTimers();
    const { wrapper } = render();
    await flushPromises();
    expect(NetworkAddressService.snapshot).not.toHaveBeenCalled();
    await wrapper.setProps({ active: true });
    await flushPromises();
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(15000);
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(4);
    await wrapper.setProps({ active: false });
    await vi.advanceTimersByTimeAsync(15000);
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(4);
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(15000);
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(4);
  });
  it('checks network changes on reconnect without forcing a refresh of an unchanged cache', async () => {
    const { wrapper } = render(true);
    await flushPromises();
    window.dispatchEvent(new Event('online'));
    await flushPromises();
    expect(vi.mocked(NetworkAddressService.snapshot).mock.calls.map(([request]) => request)).toEqual([
      'open',
      'observe',
    ]);
    await wrapper.setProps({ active: false });
    window.dispatchEvent(new Event('online'));
    await flushPromises();
    expect(NetworkAddressService.snapshot).toHaveBeenCalledTimes(2);
  });
  it('refreshes on demand and marks last known results after a failed refresh', async () => {
    const { wrapper } = render(true);
    await flushPromises();
    const failed = cached(2);
    failed.probes.ipv4.failed = true;
    failed.probes.ipv6.failed = true;
    vi.mocked(NetworkAddressService.snapshot).mockResolvedValue(failed);
    await wrapper.get('button[aria-label="Refresh"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('Not updated');
    expect(wrapper.get('[data-family=ipv4]').text()).toContain('203.0.113.1');
  });
});
