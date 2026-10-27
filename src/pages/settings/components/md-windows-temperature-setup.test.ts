// @vitest-environment happy-dom
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Setup from './md-windows-temperature-setup.vue';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import { useWindowsMonitoringStore } from '@/stores/windows-monitoring-store';
import { WindowsMonitoringService } from '@/lib/services/system-resources/windows-monitoring-service';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { preferencesFixture } from '@/tests/fixtures/resident';
import { toast } from 'vue-sonner';

vi.mock('@/lib/services/system-resources/windows-monitoring-service', () => ({
  WindowsMonitoringService: { observe: vi.fn(), install: vi.fn(), uninstall: vi.fn() },
}));
vi.mock('@/lib/services/resident/resident-service', () => ({
  ResidentService: { savePreferences: vi.fn(), preferences: vi.fn() },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), warning: vi.fn() } }));
const wrappers: VueWrapper[] = [];
async function open(selectDisplayItem = true) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const settings = useResidentSettingsStore();
  settings.preferences = preferencesFixture();
  settings.draft = preferencesFixture();
  const wrapper = mount(Setup, {
    props: { open: false, selectDisplayItem },
    attachTo: document.body,
    global: {
      renderStubDefaultSlot: true,
      stubs: { teleport: true },
      plugins: [
        pinia,
        createI18n({ legacy: false, locale: 'en', messages: { en: {} }, missingWarn: false, fallbackWarn: false }),
      ],
    },
  });
  wrappers.push(wrapper);
  await wrapper.setProps({ open: true });
  await flushPromises();
  return { wrapper, settings, monitoring: useWindowsMonitoringStore() };
}
function button(wrapper: VueWrapper, key: string) {
  return wrapper.findAll('button').find(row => row.text() === `windowsMonitoring.${key}`)!;
}
async function requestRemoval(wrapper: VueWrapper) {
  await button(wrapper, 'more').trigger('keydown', { key: 'Enter' });
  await flushPromises();
  const item = wrapper.findAll('[role="menuitem"]').find(row => row.text() === 'windowsMonitoring.remove')!;
  await item.trigger('click');
  await flushPromises();
}
describe('optional Windows temperature setup', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'notInstalled',
      driverAvailable: false,
      serviceInstalled: false,
    });
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferencesFixture());
    vi.mocked(ResidentService.savePreferences).mockImplementation(async value => ({
      ...value,
      revision: value.revision + 1,
    }));
  });
  afterEach(() => wrappers.splice(0).forEach(wrapper => wrapper.unmount()));
  it('remains disabled and does not install until the user chooses install', async () => {
    const { wrapper, settings } = await open();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
    expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
      outcome: 'ready',
      observation: { state: 'ready', driverAvailable: true, serviceInstalled: true },
    });
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(WindowsMonitoringService.install).toHaveBeenCalledOnce();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(true);
    expect(settings.preferences!.metrics.find(row => row.id === 'cpuTemperature')!.enabled).toBe(true);
    expect(wrapper.emitted('update:open')).toBeUndefined();
    expect(wrapper.text()).toContain('windowsMonitoring.states.ready');
  });
  it.each([false, true])(
    'offers reinstallation when enabled preferences outlive the service (select=%s)',
    async select => {
      const { wrapper, settings } = await open(select);
      for (const value of [settings.preferences!, settings.draft!]) {
        value.windowsCpuTemperatureEnabled = true;
        value.metrics.find(row => row.id === 'cpuTemperature')!.enabled = true;
      }
      await flushPromises();
      expect(button(wrapper, 'install')).toBeDefined();
      expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
      vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
        outcome: 'ready',
        observation: { state: 'ready', driverAvailable: true, serviceInstalled: true },
      });
      await button(wrapper, 'install').trigger('click');
      await flushPromises();
      expect(WindowsMonitoringService.install).toHaveBeenCalledOnce();
      expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(true);
      expect(settings.preferences!.metrics.find(row => row.id === 'cpuTemperature')!.enabled).toBe(true);
      expect(wrapper.emitted('update:open')).toBeUndefined();
      expect(wrapper.text()).toContain('windowsMonitoring.states.ready');
    }
  );
  it.each([
    'userCancelled',
    'installationFailed',
    'verificationFailed',
    'restartRequired',
    'serviceUnavailable',
  ] as const)('keeps the capability off after %s', async outcome => {
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
      outcome,
      observation: { state: 'notInstalled', driverAvailable: false, serviceInstalled: false },
    });
    const { wrapper, settings } = await open();
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    expect(wrapper.emitted('update:open')).toBeUndefined();
    expect(wrapper.text()).not.toContain(`windowsMonitoring.outcomes.${outcome}`);
    if (['installationFailed', 'verificationFailed', 'serviceUnavailable'].includes(outcome)) {
      expect(toast.error).toHaveBeenCalledWith(`windowsMonitoring.outcomes.${outcome}`);
    } else if (outcome === 'restartRequired') {
      expect(toast.warning).toHaveBeenCalledWith(`windowsMonitoring.outcomes.${outcome}`);
    } else {
      expect(toast.error).not.toHaveBeenCalled();
      expect(toast.warning).not.toHaveBeenCalled();
    }
  });
  it('keeps certificate diagnostics visible without enabling collection', async () => {
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
      outcome: 'certificateUntrusted',
      nativeCode: '0x800B0109',
      observation: { state: 'notInstalled', driverAvailable: false, serviceInstalled: false },
    });
    const { wrapper, settings } = await open();
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('windowsMonitoring.outcomes.certificateUntrusted');
    expect(wrapper.text()).toContain('0x800B0109');
    expect(wrapper.text()).toContain('windowsMonitoring.certificateHint');
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });
  it('clears an old trust failure when a later enable observes a ready service', async () => {
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
      outcome: 'certificateUntrusted',
      nativeCode: '0x800B0109',
      observation: { state: 'notInstalled', driverAvailable: false, serviceInstalled: false },
    });
    const { wrapper, settings } = await open();
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('0x800B0109');
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(true);
    expect(wrapper.text()).toContain('windowsMonitoring.states.ready');
    expect(wrapper.text()).not.toContain('0x800B0109');
    expect(wrapper.text()).not.toContain('windowsMonitoring.certificateHint');
    expect(WindowsMonitoringService.install).toHaveBeenCalledOnce();
  });
  it('keeps the operation code and troubleshooting hint from the same diagnostic', async () => {
    vi.mocked(WindowsMonitoringService.install).mockResolvedValue({
      outcome: 'verificationFailed',
      nativeCode: '0x80096010',
      observation: {
        state: 'notInstalled',
        driverAvailable: false,
        serviceInstalled: false,
        dependencyIssue: { reason: 'certificateUntrusted', nativeCode: '0x800B0109' },
      },
    });
    const { wrapper } = await open();
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('windowsMonitoring.outcomes.verificationFailed');
    expect(wrapper.text()).toContain('0x80096010');
    expect(wrapper.text()).not.toContain('windowsMonitoring.certificateHint');
    expect(wrapper.text()).not.toContain('0x800B0109');
  });
  it.each(['certificateUntrusted', 'verificationFailed'] as const)(
    'updates collection after %s only when the service is confirmed removed',
    async outcome => {
      vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
        state: 'ready',
        driverAvailable: true,
        serviceInstalled: true,
      });
      const { wrapper, settings } = await open();
      settings.preferences!.windowsCpuTemperatureEnabled = true;
      settings.draft!.windowsCpuTemperatureEnabled = true;
      for (const serviceInstalled of [true, false]) {
        vi.mocked(WindowsMonitoringService.uninstall).mockResolvedValue({
          outcome,
          nativeCode: outcome === 'certificateUntrusted' ? '0x800B0109' : '0x80096010',
          observation: {
            state: serviceInstalled ? 'ready' : 'notInstalled',
            driverAvailable: false,
            serviceInstalled,
          },
        });
        await requestRemoval(wrapper);
        await button(wrapper, 'remove').trigger('click');
        await flushPromises();
        expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(serviceInstalled);
      }
    }
  );
  it('distinguishes a missing dependency from an installed untrusted dependency', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'notInstalled',
      driverAvailable: false,
      serviceInstalled: false,
      dependencyIssue: { reason: 'missing' },
    });
    const { wrapper, monitoring } = await open();
    expect(wrapper.text()).toContain('windowsMonitoring.dependency.missing');
    monitoring.observation = {
      state: 'notInstalled',
      driverAvailable: false,
      serviceInstalled: false,
      dependencyIssue: { reason: 'certificateUntrusted', nativeCode: '0x800B0109' },
    };
    await flushPromises();
    expect(wrapper.text()).toContain('windowsMonitoring.dependency.certificateUntrusted');
    expect(wrapper.text()).toContain('0x800B0109');
    expect(wrapper.text()).toContain('windowsMonitoring.certificateHint');
    expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
  });
  it('does not offer installation for unsupported hardware', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'unsupportedHardware',
      driverAvailable: false,
      serviceInstalled: false,
    });
    const { wrapper } = await open();
    expect(button(wrapper, 'install').attributes('disabled')).toBeDefined();
    expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
  });
  it('keeps an existing service removable when the CPU becomes unsupported', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'unsupportedHardware',
      driverAvailable: false,
      serviceInstalled: true,
    });
    const { wrapper } = await open();
    await requestRemoval(wrapper);
    expect(button(wrapper, 'remove')).toBeDefined();
    expect(button(wrapper, 'enable').attributes('disabled')).toBeDefined();
  });
  it('enables an already verified service without installation or elevation and preserves display choices', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    const { wrapper, settings } = await open(false);
    await button(wrapper, 'enable').trigger('click');
    await flushPromises();
    expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(true);
    expect(settings.preferences!.metrics).toEqual(preferencesFixture().metrics);
  });
  it('does not close or claim enabled when persistence fails', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    vi.mocked(ResidentService.savePreferences).mockRejectedValue(new Error('disk full'));
    const { wrapper, settings } = await open();
    await button(wrapper, 'enable').trigger('click');
    await flushPromises();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
    expect(wrapper.emitted('update:open')).toBeUndefined();
    expect(toast.error).toHaveBeenCalledWith('systemStatus.saveFailed');
  });
  it('blocks duplicate actions and closing during UAC or installation', async () => {
    let resolve!: (value: Awaited<ReturnType<typeof WindowsMonitoringService.install>>) => void;
    vi.mocked(WindowsMonitoringService.install).mockReturnValue(
      new Promise(done => {
        resolve = done;
      })
    );
    const { wrapper } = await open();
    await button(wrapper, 'install').trigger('click');
    await flushPromises();
    expect(button(wrapper, 'install').attributes('disabled')).toBeDefined();
    expect(
      wrapper.findAll('button').filter(row => row.attributes('disabled') !== undefined).length
    ).toBeGreaterThanOrEqual(2);
    resolve({
      outcome: 'userCancelled',
      observation: { state: 'notInstalled', driverAvailable: false, serviceInstalled: false },
    });
    await flushPromises();
    expect(button(wrapper, 'install').attributes('disabled')).toBeUndefined();
  });
  it('keeps collection enabled when the display item is hidden and offers only maintenance actions', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    const { wrapper, settings } = await open(false);
    settings.preferences!.windowsCpuTemperatureEnabled = true;
    settings.draft!.windowsCpuTemperatureEnabled = true;
    await flushPromises();
    expect(settings.preferences!.metrics.find(row => row.id === 'cpuTemperature')!.enabled).toBe(false);
    expect(button(wrapper, 'disable')).toBeUndefined();
    expect(button(wrapper, 'enable')).toBeUndefined();
    expect(wrapper.text()).toContain('windowsMonitoring.collectionHint');
    expect(wrapper.text()).toContain('windowsMonitoring.displayHint');
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    expect(WindowsMonitoringService.uninstall).not.toHaveBeenCalled();
  });
  it('can select the display item when collection is already enabled', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    const { wrapper, settings } = await open(true);
    settings.preferences!.windowsCpuTemperatureEnabled = true;
    settings.draft!.windowsCpuTemperatureEnabled = true;
    await flushPromises();
    await button(wrapper, 'enable').trigger('click');
    await flushPromises();
    expect(settings.preferences!.metrics.find(row => row.id === 'cpuTemperature')!.enabled).toBe(true);
    expect(WindowsMonitoringService.install).not.toHaveBeenCalled();
    expect(wrapper.emitted('update:open')).toBeUndefined();
    expect(wrapper.text()).toContain('windowsMonitoring.states.ready');
  });
  it('clears sampling and display selection only after a successful service removal', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    const { wrapper, settings } = await open();
    settings.preferences!.windowsCpuTemperatureEnabled = true;
    settings.draft!.windowsCpuTemperatureEnabled = true;
    vi.mocked(WindowsMonitoringService.uninstall).mockResolvedValueOnce({
      outcome: 'userCancelled',
      observation: { state: 'ready', driverAvailable: true, serviceInstalled: true },
    });
    await requestRemoval(wrapper);
    expect(WindowsMonitoringService.uninstall).not.toHaveBeenCalled();
    await button(wrapper, 'remove').trigger('click');
    await flushPromises();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(true);
    vi.mocked(WindowsMonitoringService.uninstall).mockResolvedValue({
      outcome: 'removed',
      observation: { state: 'notInstalled', driverAvailable: true, serviceInstalled: false },
    });
    await requestRemoval(wrapper);
    await button(wrapper, 'remove').trigger('click');
    await flushPromises();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
    expect(settings.preferences!.metrics.find(row => row.id === 'cpuTemperature')!.enabled).toBe(false);
    expect(WindowsMonitoringService.uninstall).toHaveBeenLastCalledWith(false);
  });
  it('removes the shared driver only after an explicit checked confirmation and resets that choice', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    const { wrapper } = await open();
    vi.mocked(WindowsMonitoringService.uninstall).mockResolvedValue({
      outcome: 'userCancelled',
      observation: { state: 'ready', driverAvailable: true, serviceInstalled: true },
    });
    await requestRemoval(wrapper);
    const checkbox = wrapper.find('#remove-pawnio-driver');
    expect(checkbox.attributes('data-state')).toBe('unchecked');
    await checkbox.trigger('click');
    await button(wrapper, 'remove').trigger('click');
    await flushPromises();
    expect(WindowsMonitoringService.uninstall).toHaveBeenLastCalledWith(true);
    await requestRemoval(wrapper);
    expect(wrapper.find('#remove-pawnio-driver').attributes('data-state')).toBe('unchecked');
  });
  it.each(['driverRemovalFailed', 'restartRequired'] as const)(
    'turns collection off when the service was removed but the driver reports %s',
    async outcome => {
      vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
        state: 'ready',
        driverAvailable: true,
        serviceInstalled: true,
      });
      const { wrapper, settings } = await open();
      settings.preferences!.windowsCpuTemperatureEnabled = true;
      settings.draft!.windowsCpuTemperatureEnabled = true;
      vi.mocked(WindowsMonitoringService.uninstall).mockResolvedValue({
        outcome,
        observation: { state: 'notInstalled', driverAvailable: true, serviceInstalled: false },
      });
      await requestRemoval(wrapper);
      await button(wrapper, 'remove').trigger('click');
      await flushPromises();
      expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
      expect(wrapper.emitted('update:open')).toBeUndefined();
    }
  );
  it('clears collection when service removal succeeds but file cleanup fails', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'ready',
      driverAvailable: true,
      serviceInstalled: true,
    });
    const { wrapper, settings } = await open();
    for (const value of [settings.preferences!, settings.draft!]) {
      value.windowsCpuTemperatureEnabled = true;
      value.metrics.find(row => row.id === 'cpuTemperature')!.enabled = true;
    }
    vi.mocked(WindowsMonitoringService.uninstall).mockResolvedValue({
      outcome: 'removalFailed',
      observation: { state: 'notInstalled', driverAvailable: true, serviceInstalled: false },
    });
    await requestRemoval(wrapper);
    await button(wrapper, 'remove').trigger('click');
    await flushPromises();
    expect(settings.preferences!.windowsCpuTemperatureEnabled).toBe(false);
    expect(settings.preferences!.metrics.find(row => row.id === 'cpuTemperature')!.enabled).toBe(false);
    expect(toast.error).toHaveBeenCalledWith('windowsMonitoring.outcomes.removalFailed');
  });
});
