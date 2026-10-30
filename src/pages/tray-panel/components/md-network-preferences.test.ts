// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ResidentService } from '@/lib/services/resident/resident-service';
import type { ResidentPreferences } from '@/lib/models/resident/resident';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import { preferencesFixture } from '@/tests/fixtures/resident';
import { emptyReadings } from '@/lib/utils/system-resources/system-resources';
import { Select } from '@/components/ui/select';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import Network from './md-network-details.vue';

vi.mock('@/lib/services/resident/resident-service', () => ({
  ResidentService: { preferences: vi.fn(), onPreferencesChanged: vi.fn(), savePreferences: vi.fn() },
}));
vi.mock('@/lib/services/settings/operating-system-service', () => ({
  OperatingSystemService: { isWindows: () => false },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn(), info: vi.fn() } }));

function deferred() {
  let resolve!: (value: ResidentPreferences) => void;
  const promise = new Promise<ResidentPreferences>(accept => {
    resolve = accept;
  });
  return { promise, resolve };
}
function reading() {
  const value = emptyReadings();
  value.observedAtMs = 1000;
  const lan = {
    id: 'lan',
    name: 'Wired adapter',
    kind: 'ethernet' as const,
    physical: true,
    connected: true,
    defaultRouteMetric: 0,
    details: null,
  };
  value.interfaces = [lan, { ...lan, id: 'wifi', name: 'Wireless adapter', kind: 'wifi' }];
  value.network = {
    status: 'ready',
    sampledAtMs: 1000,
    value: {
      interface: lan,
      receivedBytesPerSecond: 1024,
      transmittedBytesPerSecond: 256,
      receivedBytesSinceStart: 0,
      transmittedBytesSinceStart: 0,
      receiveLinkBps: null,
      transmitLinkBps: null,
      selectionReason: 'defaultRoute',
      cumulative: null,
    },
  };
  return value;
}
let notify: () => void;
const stop = vi.fn();
const wrappers: ReturnType<typeof mount>[] = [];
function render() {
  const pinia = createPinia();
  const i18n = createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': en, 'zh-CN': zh } });
  const wrapper = mount(Network, {
    props: { reading: reading(), active: true },
    global: {
      plugins: [pinia, i18n],
      stubs: { MdResourceTrend: true, MdResourceFacts: true },
    },
  });
  wrappers.push(wrapper);
  return { wrapper, settings: useResidentSettingsStore(pinia), i18n };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(ResidentService.preferences).mockResolvedValue(preferencesFixture());
  vi.mocked(ResidentService.onPreferencesChanged).mockImplementation(async callback => {
    notify = callback;
    return stop;
  });
});
afterEach(() => wrappers.splice(0).forEach(wrapper => wrapper.unmount()));

describe('network selector preference synchronization', () => {
  it('reloads a newer notification received during the initial read', async () => {
    const pending = deferred();
    const latest = { ...preferencesFixture(), revision: 1, networkInterface: 'wifi' };
    vi.mocked(ResidentService.preferences).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(latest);
    const { settings } = render();
    await flushPromises();
    notify();
    pending.resolve(preferencesFixture());
    await flushPromises();
    expect(settings.preferences).toEqual(latest);
  });

  it('coalesces external notifications during a save and loads the latest committed selection', async () => {
    const { wrapper, settings } = render();
    await flushPromises();
    const pending = deferred();
    const saved = { ...preferencesFixture(), revision: 1, networkInterface: 'lan' };
    const latest = { ...saved, revision: 2, networkInterface: 'wifi' };
    vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(pending.promise);
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce(latest);
    wrapper.getComponent(Select).vm.$emit('update:modelValue', 'lan');
    expect(settings.saving).toBe(true);
    notify();
    notify();
    pending.resolve(saved);
    await flushPromises();
    expect(settings.preferences).toEqual(latest);
    expect(ResidentService.preferences).toHaveBeenCalledTimes(2);
  });

  it('updates a closed selector when an interface is renamed and the locale changes', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce({ ...preferencesFixture(), networkInterface: 'lan' });
    const { wrapper, settings, i18n } = render();
    await flushPromises();
    expect(wrapper.get('[role=combobox]').attributes('aria-label')).toContain('Wired adapter');
    const renamed = reading();
    renamed.interfaces[0]!.name = 'Renamed adapter';
    await wrapper.setProps({ reading: renamed });
    expect(wrapper.get('[role=combobox]').attributes('aria-label')).toContain('Renamed adapter');
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce({ ...preferencesFixture(), revision: 1 });
    notify();
    await flushPromises();
    expect(settings.draft?.networkInterface).toBeNull();
    i18n.global.locale.value = 'zh-CN';
    await flushPromises();
    expect(wrapper.get('[role=combobox]').attributes('aria-label')).toContain(zh.systemStatus.automatic);
  });

  it('does not label a missing saved adapter with another active adapter name', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce({
      ...preferencesFixture(),
      networkInterface: 'removed-adapter',
    });
    const { wrapper } = render();
    await flushPromises();
    expect(wrapper.get('[role=combobox]').attributes('aria-label')).toContain(en.systemStatus.savedDisconnected);
    expect(wrapper.get('[role=combobox]').attributes('aria-label')).not.toContain('Wired adapter');
  });

  it('silently renders the actual fallback while preserving the saved preference through recovery', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce({
      ...preferencesFixture(),
      networkInterface: 'removed-adapter',
    });
    const { wrapper, settings } = render();
    await flushPromises();
    expect(wrapper.getComponent(Select).props('modelValue')).toBe('removed-adapter');
    expect(wrapper.get('.network-identity').text()).toContain('Wired adapter');
    expect(wrapper.get('.download .rate-number').text()).toContain('1');
    expect(wrapper.find('[role=alert]').exists()).toBe(false);

    const restored = reading();
    restored.network.value!.interface = {
      ...restored.interfaces[0]!,
      id: 'removed-adapter',
      name: 'Restored adapter',
    };
    restored.interfaces.push(restored.network.value!.interface);
    restored.network.value!.selectionReason = 'manual';
    await wrapper.setProps({ reading: restored });
    expect(wrapper.get('.network-identity').text()).toContain('Restored adapter');
    expect(wrapper.get('[role=combobox]').attributes('aria-label')).toContain('Restored adapter');
    expect(settings.draft?.networkInterface).toBe('removed-adapter');
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });

  it('only persists automatic mode when the user explicitly selects it', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce({
      ...preferencesFixture(),
      networkInterface: 'removed-adapter',
    });
    vi.mocked(ResidentService.savePreferences).mockResolvedValueOnce({
      ...preferencesFixture(),
      revision: 1,
      networkInterface: null,
    });
    const { wrapper, settings } = render();
    await flushPromises();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    wrapper.getComponent(Select).vm.$emit('update:modelValue', 'automatic');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenCalledOnce();
    expect(settings.preferences?.networkInterface).toBeNull();
  });

  it('allows retrying a failed preference read without leaving the network page', async () => {
    vi.mocked(ResidentService.preferences).mockRejectedValueOnce(new Error('injected read failure'));
    const { wrapper } = render();
    await flushPromises();
    expect(wrapper.get('[role=combobox]').attributes('disabled')).toBeDefined();
    await wrapper.get('.network-selection-retry').trigger('click');
    await flushPromises();
    expect(wrapper.get('[role=combobox]').attributes('disabled')).toBeUndefined();
  });
  it('does not loop after a deferred refresh fails and can retry on demand', async () => {
    const pending = deferred();
    vi.mocked(ResidentService.preferences)
      .mockReturnValueOnce(pending.promise)
      .mockRejectedValueOnce(new Error('injected refresh failure'));
    const { wrapper, settings } = render();
    await flushPromises();
    notify();
    pending.resolve(preferencesFixture());
    await flushPromises();
    await flushPromises();
    expect(ResidentService.preferences).toHaveBeenCalledTimes(2);
    expect(settings.error).toBe(true);
    await wrapper.get('.network-selection-retry').trigger('click');
    await flushPromises();
    expect(ResidentService.preferences).toHaveBeenCalledTimes(3);
    expect(settings.error).toBe(false);
  });

  it('discards deferred notifications when unmounted during a read', async () => {
    const pending = deferred();
    vi.mocked(ResidentService.preferences).mockReturnValueOnce(pending.promise);
    const { wrapper } = render();
    await flushPromises();
    notify();
    wrapper.unmount();
    pending.resolve(preferencesFixture());
    await flushPromises();
    expect(stop).toHaveBeenCalledOnce();
    expect(ResidentService.preferences).toHaveBeenCalledOnce();
  });

  it('releases a subscription that finishes after unmount without starting a read', async () => {
    let subscribe!: (stop: () => void) => void;
    vi.mocked(ResidentService.onPreferencesChanged).mockReturnValueOnce(
      new Promise(resolve => {
        subscribe = resolve;
      })
    );
    const { wrapper } = render();
    wrapper.unmount();
    subscribe(stop);
    await flushPromises();
    expect(stop).toHaveBeenCalledOnce();
    expect(ResidentService.preferences).not.toHaveBeenCalled();
  });
  it('reconnects the preference subscription on retry after registration fails', async () => {
    vi.mocked(ResidentService.onPreferencesChanged).mockRejectedValueOnce(new Error('injected registration failure'));
    const { wrapper, settings } = render();
    await flushPromises();
    const latest = { ...preferencesFixture(), revision: 1, networkInterface: 'wifi' };
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce(latest);
    await wrapper.get('.network-selection-retry').trigger('click');
    await flushPromises();
    expect(ResidentService.onPreferencesChanged).toHaveBeenCalledTimes(2);
    expect(settings.preferences).toEqual(latest);
    const next = { ...latest, revision: 2, networkInterface: 'lan' };
    vi.mocked(ResidentService.preferences).mockResolvedValueOnce(next);
    notify();
    await flushPromises();
    expect(settings.preferences).toEqual(next);
  });
  it('closes the device menu on native hide without changing the selected interface', async () => {
    const { wrapper, settings } = render();
    await flushPromises();
    wrapper.getComponent(Select).vm.$emit('update:open', true);
    await flushPromises();
    expect(wrapper.getComponent(Select).props('open')).toBe(true);
    await wrapper.setProps({ active: false });
    expect(wrapper.getComponent(Select).props('open')).toBe(false);
    expect(settings.draft?.networkInterface).toBeNull();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });
});
