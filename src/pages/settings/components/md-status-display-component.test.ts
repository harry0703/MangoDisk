// @vitest-environment happy-dom
import { flushPromises, mount as mountComponent } from '@vue/test-utils';
import { useResidentSettingsStore } from '@/stores/resident-settings-store';
import { createPinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import Settings from './md-status-display-settings.vue';
import MdConfirmDialog from '@/components/custom/md-confirm-dialog.vue';
import TaskbarPosition from './md-taskbar-position.vue';
import { Select } from '@/components/ui/select';
import WindowsFeedback from './md-windows-display-feedback.vue';
import { ResidentService } from '@/lib/services/resident/resident-service';
import { WindowsMonitoringService } from '@/lib/services/system-resources/windows-monitoring-service';
import { useWindowsMonitoringStore } from '@/stores/windows-monitoring-store';
import { preferencesFixture, readingFixture } from '@/tests/fixtures/resident';
import type { ResidentPreferences } from '@/lib/models/resident/resident';

vi.mock('@/lib/services/resident/resident-service', () => ({
  ResidentService: {
    preferences: vi.fn(),
    savePreferences: vi.fn(),
    defaultPreferences: vi.fn(),
    onPreferencesChanged: vi.fn(),
    catalogue: vi.fn(),
    onReading: vi.fn(),
    displayStatus: vi.fn(),
    onDisplayStatus: vi.fn(),
    releaseMemory: vi.fn(),
    quit: vi.fn(),
    openMain: vi.fn(),
    openPanel: vi.fn(),
  },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('@/lib/services/system-resources/windows-monitoring-service', () => ({
  WindowsMonitoringService: { observe: vi.fn(), install: vi.fn(), uninstall: vi.fn() },
}));
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), warning: vi.fn() } }));
vi.mock('@/lib/services/settings/byte-size-service', () => ({
  ByteSizeService: { memory: (value: number) => `${value} B` },
}));
// Reka's modal accessibility checks require the rendered content to belong to document.body.
// Keeping every wrapper attached also prevents aria-hidden from serializing a detached DOM tree
// to stderr, which can make otherwise fast interaction tests exceed their timeout.
const mount: typeof mountComponent = ((component, options) =>
  mountComponent(component, { ...options, attachTo: options?.attachTo ?? document.body })) as typeof mountComponent;
function global() {
  return {
    // Reka's Teleport wrapper shares Vue's stub name. Preserve its slot so
    // dialog content remains testable without replacing the dialog behavior.
    renderStubDefaultSlot: true,
    stubs: { teleport: true },
    plugins: [
      createPinia(),
      createI18n({
        legacy: false,
        locale: 'en',
        messages: { en: { systemStatus: { moveAnnouncement: '{name}: {position}/{count}' } } },
        missingWarn: false,
        fallbackWarn: false,
      }),
    ],
  };
}
async function openConfiguration(wrapper: ReturnType<typeof mount>) {
  const configure = wrapper.find('#resident-configure');
  if (configure.exists()) {
    await configure.trigger('click');
    await flushPromises();
  }
}
const wrappers: ReturnType<typeof mount>[] = [];
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
    state: 'ready',
    driverAvailable: true,
    serviceInstalled: true,
  });
  vi.mocked(ResidentService.displayStatus).mockResolvedValue('tray');
  vi.mocked(ResidentService.onDisplayStatus).mockResolvedValue(vi.fn());
  vi.mocked(ResidentService.preferences).mockResolvedValue(preferencesFixture());
  vi.mocked(ResidentService.defaultPreferences).mockResolvedValue(preferencesFixture());
  vi.mocked(ResidentService.onPreferencesChanged).mockResolvedValue(vi.fn());
  vi.mocked(ResidentService.savePreferences).mockImplementation(async value => {
    const committed = { ...value, revision: value.revision + 1 };
    vi.mocked(ResidentService.preferences).mockResolvedValue(committed);
    return committed;
  });
  vi.mocked(ResidentService.catalogue).mockResolvedValue(readingFixture());
  vi.mocked(ResidentService.onReading).mockResolvedValue(vi.fn());
});
afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount());
  vi.useRealTimers();
});
describe('status display interactions', () => {
  it.each([
    { isMacOs: true, isLinux: false, hint: 'systemStatus.displayHint' },
    { isMacOs: false, isLinux: false, hint: 'systemStatus.displayHint' },
    { isMacOs: false, isLinux: true, hint: 'systemStatus.linuxDisplayHint' },
  ])('renders the accessible description on $isMacOs/$isLinux', async ({ hint, ...props }) => {
    const wrapper = mount(Settings, { props, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    const description = wrapper.get('#resident-enabled-hint');
    expect(description.text()).toBe(hint);
    expect(wrapper.get('#resident-enabled').attributes('aria-describedby')).toBe(description.attributes('id'));
  });

  it('retains a failed reset confirmation when a queued native notification reloads preferences', async () => {
    let notify!: () => void;
    vi.mocked(ResidentService.onPreferencesChanged).mockImplementation(async callback => {
      notify = callback;
      return vi.fn<() => void>();
    });
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    let reject!: (error: Error) => void;
    vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(new Promise((_, fail) => (reject = fail)));
    await wrapper.get('#resident-restore-defaults').trigger('click');
    const confirmation = wrapper.findComponent(MdConfirmDialog);
    confirmation.vm.$emit('confirm');
    await flushPromises();
    expect(confirmation.props('busy')).toBe(true);
    notify();
    reject(new Error('disk full'));
    await flushPromises();
    expect(useResidentSettingsStore().error).toBe(false);
    expect(confirmation.props('open')).toBe(true);
    expect(confirmation.props('busy')).toBe(false);
    expect(wrapper.get('[role="alert"]').text()).toContain('systemStatus.saveFailed');
    confirmation.vm.$emit('confirm');
    await flushPromises();
    expect(confirmation.props('open')).toBe(false);
  });

  it('requires confirmation for defaults, supports cancellation and applies the confirmed reset', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const store = useResidentSettingsStore();
    await store.change({ macosDisplayMode: 'both', usageColors: false });
    vi.mocked(ResidentService.savePreferences).mockClear();
    await wrapper.get('#resident-restore-defaults').trigger('click');
    const confirmation = wrapper.findComponent(MdConfirmDialog);
    expect(confirmation.props('open')).toBe(true);
    expect(ResidentService.defaultPreferences).not.toHaveBeenCalled();
    confirmation.vm.$emit('update:open', false);
    await flushPromises();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    await wrapper.get('#resident-restore-defaults').trigger('click');
    confirmation.vm.$emit('confirm');
    await flushPromises();
    expect(confirmation.props('open')).toBe(false);
    expect(store.preferences).toMatchObject({ usageColors: true, macosDisplayMode: 'both', enabled: true });
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    expect(wrapper.find('#resident-restore-defaults').exists()).toBe(true);
  });

  it('defaults to content, preserves per-mode tab scroll and resets on reopening', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const tabs = () => wrapper.findAll('[role="tab"]');
    const active = () => wrapper.get('[role="tabpanel"][data-state="active"]');
    expect(tabs()).toHaveLength(3);
    expect(tabs()[0]!.attributes('aria-selected')).toBe('true');
    expect(active().find('#status-cpu').exists()).toBe(true);
    expect(active().find('#taskbar-compact').exists()).toBe(false);
    const contentPanel = active();
    active().element.scrollTop = 120;
    await active().trigger('scroll');
    const handle = wrapper.get('[data-metric="cpu"] .drag-handle');
    await handle.trigger('keydown', { key: ' ' });
    expect(handle.attributes('aria-pressed')).toBe('true');
    await tabs()[1]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(handle.attributes('aria-pressed')).toBe('false');
    expect(active().find('#taskbar-compact').exists()).toBe(true);
    expect(wrapper.find('.appearance-preview').exists()).toBe(false);
    active().element.scrollTop = 48;
    await active().trigger('scroll');
    await contentPanel.trigger('scroll');
    await tabs()[0]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(active().element.scrollTop).toBe(120);
    await tabs()[1]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(active().element.scrollTop).toBe(48);
    await tabs()[0]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    const store = useResidentSettingsStore();
    await store.change({ windowsDisplayMode: 'floating' });
    await flushPromises();
    expect(active().element.scrollTop).toBe(0);
    await store.change({ windowsDisplayMode: 'taskbar' });
    await flushPromises();
    expect(active().element.scrollTop).toBe(120);
    await tabs()[2]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(active().find('input[name="taskbar-position"]').exists()).toBe(true);
    await wrapper
      .findAll('button')
      .find(button => button.text() === 'systemStatus.done')!
      .trigger('click');
    await flushPromises();
    await openConfiguration(wrapper);
    expect(tabs()[0]!.attributes('aria-selected')).toBe('true');
    expect(active().element.scrollTop).toBe(0);
  });
  it.each(['floating', 'both'] as const)('keeps Windows fullscreen controls off macOS in %s mode', async mode => {
    const preferences = preferencesFixture();
    preferences.macosDisplayMode = mode;
    preferences.windowsDisplayMode = 'both';
    preferences.floating.hideOnFullscreen = false;
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferences);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(2);
    await wrapper.findAll('[role="tab"]')[1]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(wrapper.find('#floating-font-size').exists()).toBe(true);
    expect(wrapper.find('#floating-fullscreen').exists()).toBe(false);
  });
  it.each(['floating', 'both'] as const)('exposes passive Linux appearance in %s mode', async mode => {
    const preferences = preferencesFixture();
    preferences.linuxDisplayMode = mode;
    preferences.windowsDisplayMode = 'both';
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferences);
    const wrapper = mount(Settings, { props: { isMacOs: false, isLinux: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(2);
    expect(wrapper.find('#linux-display-mode').exists()).toBe(true);
    expect(wrapper.find('#status-app-icon').attributes('disabled')).toBeDefined();
    await wrapper.findAll('[role="tab"]')[1]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(wrapper.find('#floating-font-size').exists()).toBe(true);
    expect(wrapper.find('#linux-tray-compact').exists()).toBe(mode === 'both');
    expect(wrapper.find('#floating-fullscreen').exists()).toBe(false);
    expect(wrapper.find('#floating-click-through').exists()).toBe(false);
  });
  it.each([
    { isMacOs: true, isLinux: false },
    { isMacOs: false, isLinux: true },
  ])('omits Windows-only behavior on %o', async props => {
    const wrapper = mount(Settings, { props, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(2);
    expect(wrapper.find('#windows-display-mode').exists()).toBe(false);
    await wrapper.findAll('[role="tab"]')[1]!.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    const active = wrapper.get('[role="tabpanel"][data-state="active"]');
    expect(active.find('.appearance-preview').exists()).toBe(false);
    expect(active.find('#floating-font-size').exists()).toBe(false);
    expect(active.find('#taskbar-background').exists()).toBe(false);
    const compact = active.get(props.isMacOs ? '#menu-bar-compact' : '#linux-tray-compact');
    await compact.trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining(props.isMacOs ? { menuBarCompact: true } : { taskbarCompact: true })
    );
  });

  it.each(['resolve', 'reject'] as const)(
    'keeps unrelated controls stable during a disk preference save that will %s',
    async outcome => {
      const saved = preferencesFixture();
      saved.metrics = saved.metrics.map(metric => ({
        ...metric,
        enabled: ['memory', 'network', 'disk', 'diskTemperature', 'fanSpeed'].includes(metric.id),
      }));
      vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
      const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      let complete!: (value: ResidentPreferences) => void;
      let reject!: (error: Error) => void;
      vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(
        new Promise((resolve, failure) => {
          complete = resolve;
          reject = failure;
        })
      );
      const temperature = wrapper.get('[data-metric="diskTemperature"]').getComponent(Select);
      temperature.vm.$emit('update:modelValue', 'physical-b');
      await flushPromises();
      const store = useResidentSettingsStore();
      expect(store.saving).toBe(true);
      expect(wrapper.get('#resident-enabled').attributes('disabled')).toBeUndefined();
      for (const id of ['disk', 'diskTemperature', 'network', 'fanSpeed']) {
        expect(wrapper.get(`[data-metric="${id}"]`).getComponent(Select).props('disabled')).toBe(false);
      }
      expect(wrapper.get('#color-configure-disk').attributes('disabled')).toBeUndefined();
      wrapper.get('[data-metric="disk"]').getComponent(Select).vm.$emit('update:modelValue', 'volume-b');
      await flushPromises();
      expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
      const done = wrapper.findAll('button').find(button => button.text() === 'systemStatus.done')!;
      expect(done.attributes('disabled')).toBeUndefined();
      await done.trigger('click');
      await flushPromises();
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(store.saving).toBe(true);
      if (outcome === 'resolve') {
        complete({ ...vi.mocked(ResidentService.savePreferences).mock.calls[0]![0], revision: 1 });
      } else {
        reject(new Error('injected disk preference failure'));
      }
      await flushPromises();
      expect(store.saving).toBe(false);
      expect(store.preferences?.enabled).toBe(true);
      if (outcome === 'resolve') {
        expect(ResidentService.savePreferences).toHaveBeenCalledTimes(2);
        expect(store.preferences).toMatchObject({
          revision: 2,
          diskTemperatureDevice: 'physical-b',
          diskVolume: 'volume-b',
        });
      } else {
        expect(store.draft).toEqual(saved);
        expect(wrapper.text()).toContain('systemStatus.saveFailed');
      }
    }
  );

  it('locks only the enable switch while its own change is pending and restores it on failure', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    let reject!: (error: Error) => void;
    vi.mocked(ResidentService.savePreferences).mockReturnValueOnce(
      new Promise((_, failure) => {
        reject = failure;
      })
    );
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.get('#resident-enabled').attributes('disabled')).toBeDefined();
    expect(wrapper.get('#resident-enabled').attributes('aria-checked')).toBe('true');
    reject(new Error('injected enable preference failure'));
    await flushPromises();
    expect(wrapper.get('#resident-enabled').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('#resident-enabled').attributes('aria-checked')).toBe('true');
    expect(wrapper.text()).toContain('systemStatus.saveFailed');
  });

  it('limits selected fans to two, preserves missing identities and saves each display mode', async () => {
    const reading = {
      ...readingFixture(),
      fanSpeed: {
        status: 'ready' as const,
        sampledAtMs: 1,
        value: {
          fans: [
            { id: 0, rpm: 0 },
            { id: 1, rpm: 2400 },
            { id: 2, rpm: 3000 },
          ],
        },
      },
    };
    vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="fanSpeed"]');
    expect(row.get('#status-fanSpeed').attributes('aria-checked')).toBe('false');
    expect(row.find('.color-toggle').exists()).toBe(true);
    expect(row.getComponent(Select).props('disabled')).toBe(true);
    await row.get('#status-fanSpeed').trigger('click');
    await flushPromises();
    row.getComponent(Select).vm.$emit('update:modelValue', 'average');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ fanDisplay: 'average' })
    );
    row.getComponent(Select).vm.$emit('update:modelValue', 'selected');
    await flushPromises();
    expect(row.get('#fan-choice-0').attributes('disabled')).toBeDefined();
    await row.get('#fan-choice-1').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ fanDisplay: 'selected', fanIds: [0, 1] })
    );
    expect(row.get('#fan-choice-2').attributes('disabled')).toBeDefined();
    const accept = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
    vi.mocked(ResidentService.savePreferences).mockClear();
    accept({
      ...reading,
      revision: 10,
      fanSpeed: {
        ...reading.fanSpeed,
        value: {
          fans: [
            { id: 1, rpm: 2500 },
            { id: 2, rpm: 3000 },
          ],
        },
      },
    });
    await flushPromises();
    expect(row.get('#fan-choice-0').attributes('aria-checked')).toBe('true');
    expect(row.text()).toContain('systemStatus.disconnected');
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    await row.get('#fan-choice-0').trigger('click');
    await flushPromises();
    expect(row.get('#fan-choice-2').attributes('disabled')).toBeUndefined();
    await row.get('#fan-choice-2').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ fanIds: [1, 2] }));
    row.getComponent(Select).vm.$emit('update:modelValue', 'highest');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ fanDisplay: 'highest', fanIds: [1, 2] })
    );
    expect(row.find('#fan-choice-1').exists()).toBe(false);
    await row.get('#status-fanSpeed').trigger('click');
    await flushPromises();
    expect(row.getComponent(Select).props('disabled')).toBe(true);
  });

  it('distinguishes fanless hardware, unavailable sampling and a stopped fan', async () => {
    const saved = preferencesFixture();
    saved.metrics.find(row => row.id === 'fanSpeed')!.enabled = true;
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const reading = {
      ...readingFixture(),
      fanSpeed: { status: 'ready' as const, sampledAtMs: 1, value: { fans: [] } },
    };
    vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="fanSpeed"]');
    expect(row.get('[role="status"]').text()).toBe('systemStatus.noFans');
    const accept = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
    let revision = 1;
    for (const status of ['unsupported', 'failed', 'stale', 'disconnected', 'loading'] as const) {
      accept({ ...reading, revision: ++revision, fanSpeed: { status, sampledAtMs: null, value: null } });
      await flushPromises();
      expect(row.get('[role="status"]').text()).toBe(`systemStatus.${status === 'failed' ? 'readFailed' : status}`);
      accept({ ...reading, revision: ++revision, fanSpeed: { status, sampledAtMs: 1, value: { fans: [] } } });
      await flushPromises();
      expect(row.get('[role="status"]').text()).toBe(`systemStatus.${status === 'failed' ? 'readFailed' : status}`);
    }
    accept({ ...reading, revision: ++revision, fanSpeed: { status: 'ready', sampledAtMs: 1, value: null } });
    await flushPromises();
    expect(row.get('[role="status"]').text()).toBe('systemStatus.readFailed');
    accept({
      ...reading,
      revision: revision + 1,
      fanSpeed: { status: 'ready', sampledAtMs: 3, value: { fans: [{ id: 0, rpm: 0 }] } },
    });
    await flushPromises();
    expect(row.find('[role="status"]').exists()).toBe(false);
    expect(row.findComponent(Select).exists()).toBe(false);
    expect(row.find('.fan-options').exists()).toBe(false);
    expect(row.find('.color-toggle').exists()).toBe(true);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });

  it('opens the display dialog after a navigation request waits for preferences', async () => {
    let resolvePreferences!: (value: ReturnType<typeof preferencesFixture>) => void;
    vi.mocked(ResidentService.preferences).mockReturnValueOnce(
      new Promise(resolve => {
        resolvePreferences = resolve;
      })
    );
    const wrapper = mount(Settings, { props: { isMacOs: false, openRevision: 1 }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    resolvePreferences(preferencesFixture());
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    await wrapper.get('[role="dialog"]').trigger('keydown', { key: 'Escape' });
    await flushPromises();
    await wrapper.setProps({ openRevision: 2 });
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });

  const localizedMessages = import.meta.glob<{
    systemStatus: {
      diskTemperature: string;
      fanSpeed: string;
      fanAutomatic: string;
      fanAverage: string;
      fanSelected: string;
      fanNumber: string;
      noFans: string;
    };
  }>('../../../locales/*.json', {
    eager: true,
    import: 'default',
  });
  it.each(Object.entries(localizedMessages))(
    'renders disk temperature and fan selection in %s',
    async (_path, messages) => {
      vi.mocked(ResidentService.catalogue).mockResolvedValue({
        ...readingFixture(),
        fanSpeed: {
          status: 'ready',
          sampledAtMs: 1,
          value: {
            fans: [
              { id: 0, rpm: 0 },
              { id: 1, rpm: 2400 },
            ],
          },
        },
      });
      const options = global();
      options.plugins[1] = createI18n({
        legacy: false,
        locale: 'en',
        messages: { en: messages },
        missingWarn: false,
        fallbackWarn: false,
      });
      const wrapper = mount(Settings, { props: { isMacOs: true }, global: options });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      expect(wrapper.get('[data-metric="diskTemperature"] label').text()).toBe(messages.systemStatus.diskTemperature);
      const fan = wrapper.get('[data-metric="fanSpeed"]');
      expect(fan.get('label').text()).toBe(messages.systemStatus.fanSpeed);
      expect(fan.get('.selection-toggle').text()).toBe(messages.systemStatus.fanAutomatic);
      await fan.get('[role="checkbox"]').trigger('click');
      await flushPromises();
      fan.getComponent(Select).vm.$emit('update:modelValue', 'selected');
      await flushPromises();
      expect(fan.get('.selection-toggle').text()).toBe(messages.systemStatus.fanSelected);
      expect(fan.text()).toContain(messages.systemStatus.fanNumber.replace('{number}', '1'));
      await fan.get('#fan-choice-1').trigger('click');
      await flushPromises();
      expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ fanIds: [0, 1] }));
      fan.getComponent(Select).vm.$emit('update:modelValue', 'average');
      await flushPromises();
      expect(fan.get('.selection-toggle').text()).toBe(messages.systemStatus.fanAverage);
      const accept = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
      accept({
        ...readingFixture(),
        revision: 100,
        fanSpeed: { status: 'ready', sampledAtMs: 1, value: { fans: [] } },
      });
      await flushPromises();
      expect(fan.get('[role="status"]').text()).toBe(messages.systemStatus.noFans);
    }
  );
  it.each([true, false])('offers disk temperature independently of disk usage on macOS=%s', async isMacOs => {
    const wrapper = mount(Settings, { props: { isMacOs }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const rows = wrapper.findAll('.metric-row').map(row => row.attributes('data-metric'));
    expect(rows.indexOf('diskTemperature')).toBe(rows.indexOf('disk') + 1);
    expect(wrapper.get('#status-diskTemperature').attributes('data-state')).toBe('unchecked');
    await wrapper.get('#status-diskTemperature').trigger('click');
    await flushPromises();
    const saved = vi.mocked(ResidentService.savePreferences).mock.calls.at(-1)![0];
    expect(saved.metrics.find(row => row.id === 'diskTemperature')?.enabled).toBe(true);
    expect(saved.metrics.find(row => row.id === 'disk')?.enabled).toBe(false);
    expect(saved.windowsCpuTemperatureEnabled).toBe(false);
    expect(wrapper.find('[data-metric="diskTemperature"] .selection-toggle').exists()).toBe(true);
  });
  it('keeps absent battery items visible with their reason and preserves saved choices', async () => {
    const saved = preferencesFixture();
    for (const row of saved.metrics) {
      if (['batteryTemperature', 'chargingPower', 'batteryPower'].includes(row.id)) row.enabled = true;
    }
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const reading = readingFixture();
    reading.batteryPresent = false;
    vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    for (const id of ['batteryTemperature', 'chargingPower', 'batteryPower']) {
      expect(wrapper.get(`[data-metric="${id}"] [role="status"]`).text()).toBe('systemStatus.noBattery');
      expect(wrapper.get(`#status-${id}`).attributes('disabled')).toBeUndefined();
    }
    expect(wrapper.find('[data-metric="systemPower"]').exists()).toBe(true);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    const onReading = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
    onReading({
      ...readingFixture(99),
      batteryPresent: true,
      batteryTemperature: { status: 'unsupported', sampledAtMs: null, value: null },
    });
    await flushPromises();
    for (const id of ['batteryTemperature', 'chargingPower', 'batteryPower']) {
      expect(wrapper.get(`[data-metric="${id}"] [role="checkbox"]`).attributes('aria-checked')).toBe('true');
    }
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });

  it('persists a fixed GPU and retains a missing choice until automatic is selected', async () => {
    const saved = preferencesFixture();
    saved.metrics.find(row => row.id === 'gpu')!.enabled = true;
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    vi.mocked(ResidentService.catalogue).mockResolvedValue({
      ...readingFixture(),
      gpuAdapters: [
        { id: 'gpu-a', name: 'GPU 0 · Intel' },
        { id: 'gpu-b', name: 'GPU 1 · NVIDIA' },
      ],
    });
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="gpu"]');
    const selector = row.getComponent(Select);
    expect(row.get('.selection-toggle').text()).toBe('systemStatus.automatic');
    selector.vm.$emit('update:modelValue', 'selected');
    await flushPromises();
    await row.get('#gpu-choice-usage-1').trigger('click');
    await flushPromises();
    await row.get('#gpu-choice-usage-0').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ gpuAdapter: null, gpuDisplay: { mode: 'selected', adapterIds: ['gpu-b'] } })
    );
    expect(row.get('.selection-toggle').text()).toBe('gpuDetails.selectedAdapters');
    const onReading = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
    onReading({ ...readingFixture(99), gpuAdapters: [{ id: 'gpu-a', name: 'GPU 0 · Intel' }] });
    await flushPromises();
    expect(row.get('.selection-toggle').text()).toBe('gpuDetails.selectedAdapters');
    expect(row.text()).toContain('systemStatus.savedDisconnected');
    selector.vm.$emit('update:modelValue', 'automatic');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ gpuAdapter: null, gpuDisplay: { mode: 'automatic' } })
    );
  });
  it('offers the complete catalogue on macOS and Windows with platform-specific status', async () => {
    for (const isMacOs of [true, false]) {
      const wrapper = mount(Settings, { props: { isMacOs }, global: global() });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      expect(wrapper.find('[data-metric="batteryTemperature"]').exists()).toBe(true);
      expect(wrapper.find('[data-metric="chargingPower"]').exists()).toBe(true);
      expect(wrapper.find('[data-metric="batteryPower"]').exists()).toBe(true);
      expect(wrapper.find('[data-metric="systemPower"]').exists()).toBe(true);
      if (!isMacOs)
        expect(wrapper.get('[data-metric="systemPower"] [role="status"]').text()).toBe(
          'systemStatus.platformUnsupported'
        );
      expect(wrapper.find('[data-metric="fanSpeed"]').exists()).toBe(true);
      if (isMacOs) {
        for (const id of ['batteryTemperature', 'chargingPower', 'batteryPower', 'systemPower', 'fanSpeed']) {
          expect(wrapper.get(`[data-metric="${id}"] [role="checkbox"]`).attributes('aria-checked')).toBe('false');
        }
      }
      wrapper.unmount();
    }
  });
  it('shows one explanation when charging power is unsupported', async () => {
    const reading = readingFixture();
    reading.chargingPower.status = 'unsupported';
    vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="chargingPower"]');
    expect(
      row.findAll(
        'button[aria-label="systemStatus.chargingPowerHint"], button[aria-label="systemStatus.batteryUnsupportedHint"]'
      )
    ).toHaveLength(1);
    expect(
      row.get('.metric-label-group').find('button[aria-label="systemStatus.batteryUnsupportedHint"]').exists()
    ).toBe(true);
  });
  it('opens the native resource panel without changing resident preferences', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await wrapper.get('#resident-open-panel').trigger('click');
    await flushPromises();
    expect(ResidentService.openPanel).toHaveBeenCalledOnce();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    vi.mocked(ResidentService.openPanel).mockRejectedValueOnce(new Error('native panel unavailable'));
    await wrapper.get('#resident-open-panel').trigger('click');
    await flushPromises();
    expect(wrapper.find('[role="alert"]').text()).toBe('monitoring.unavailable');
    expect(wrapper.get('#resident-open-panel').attributes('disabled')).toBeUndefined();
  });

  it('saves macOS density and usage thresholds without changing metric choices', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#menu-bar-compact').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ menuBarCompact: true, metrics: preferencesFixture().metrics })
    );
    await wrapper.get('[data-metric="cpu"] .color-toggle').trigger('click');
    expect(wrapper.find('#color-editor-cpu').exists()).toBe(true);
    const editor = wrapper.getComponent({ name: 'MdStatusColorRuleEditor' });
    const rule = {
      ...preferencesFixture().colorRules[0]!,
      rules: [
        { condition: 'below', value: 60, color: 'green' },
        { condition: 'range', from: 60, to: 90, color: 'orange' },
        { condition: 'atLeast', value: 90, color: 'red' },
      ],
    };
    editor.vm.$emit('save', rule);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ colorRules: expect.arrayContaining([rule]) })
    );
    await wrapper.get('#usage-colors').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ usageColors: false }));
  });

  it('offers shared usage colors on Windows without the macOS density setting', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('#usage-colors').exists()).toBe(true);
    expect(wrapper.find('#menu-bar-compact').exists()).toBe(false);
    expect(wrapper.find('#status-cpuTemperature').exists()).toBe(true);
  });

  it.each(['notInstalled', 'unavailable', 'incompatible', 'unsupportedHardware'] as const)(
    'checks the current service and keeps the Windows temperature unchecked when %s',
    async state => {
      const saved = preferencesFixture();
      saved.windowsCpuTemperatureEnabled = true;
      saved.metrics.find(row => row.id === 'cpuTemperature')!.enabled = false;
      vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
      vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
        state,
        driverAvailable: false,
        serviceInstalled: state !== 'notInstalled',
      });
      const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      useWindowsMonitoringStore().observation = { state: 'ready', driverAvailable: true, serviceInstalled: true };
      await wrapper.get('#status-cpuTemperature').trigger('click');
      await flushPromises();
      expect(WindowsMonitoringService.observe).toHaveBeenCalled();
      expect(wrapper.get('#status-cpuTemperature').attributes('data-state')).toBe('unchecked');
      expect(ResidentService.savePreferences).not.toHaveBeenCalled();
      expect(wrapper.text()).toContain('windowsMonitoring.title');
    }
  );

  it('shows the service failure beside the temperature label before opening management', async () => {
    vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
      state: 'unavailable',
      driverAvailable: false,
      serviceInstalled: true,
    });
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="cpuTemperature"]');
    expect(row.get('[role="status"]').text()).toBe('windowsMonitoring.states.unavailable');
    expect(row.get('.metric-label-group').find('.md-help-action').exists()).toBe(false);
    expect(wrapper.text()).not.toContain('windowsMonitoring.title');
  });

  it('checks the current service before saving the Windows temperature display choice', async () => {
    const saved = preferencesFixture();
    saved.windowsCpuTemperatureEnabled = true;
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#status-cpuTemperature').trigger('click');
    await flushPromises();
    expect(WindowsMonitoringService.observe).toHaveBeenCalledTimes(2);
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        metrics: saved.metrics.map(row => ({ ...row, enabled: row.id === 'cpuTemperature' ? true : row.enabled })),
      })
    );
    expect(wrapper.text()).not.toContain('windowsMonitoring.title');
  });

  it('does not save a late temperature check after the display configuration closes', async () => {
    const saved = preferencesFixture();
    saved.windowsCpuTemperatureEnabled = true;
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    let resolve!: (value: Awaited<ReturnType<typeof WindowsMonitoringService.observe>>) => void;
    vi.mocked(WindowsMonitoringService.observe)
      .mockResolvedValueOnce({ state: 'ready', driverAvailable: true, serviceInstalled: true })
      .mockReturnValueOnce(
        new Promise(done => {
          resolve = done;
        })
      );
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#status-cpuTemperature').trigger('click');
    await flushPromises();
    expect(wrapper.get('#status-cpuTemperature').attributes('disabled')).toBeDefined();
    const done = wrapper.findAll('button').find(row => row.text() === 'systemStatus.done')!;
    await done.trigger('click');
    resolve({ state: 'ready', driverAvailable: true, serviceInstalled: true });
    await flushPromises();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    expect(wrapper.text()).not.toContain('windowsMonitoring.title');
  });

  it('saves macOS temperature independently of CPU utilization and preserves a temperature-only entry', async () => {
    const saved = preferencesFixture();
    saved.showIcon = false;
    saved.metrics.forEach(row => (row.enabled = row.id === 'cpuTemperature'));
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.get('#status-cpuTemperature').attributes('disabled')).toBeDefined();
    expect(wrapper.get('#status-app-icon').attributes('data-state')).toBe('unchecked');
    await wrapper.get('#status-app-icon').trigger('click');
    await flushPromises();
    expect(wrapper.get('#status-cpuTemperature').attributes('disabled')).toBeUndefined();
    await wrapper.get('#status-cpuTemperature').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ showIcon: true, metrics: saved.metrics.map(row => ({ ...row, enabled: false })) })
    );
    await wrapper.get('#status-cpuTemperature').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ metrics: saved.metrics })
    );
  });

  it('retains the visible fallback logo when temperature is enabled', async () => {
    const saved = preferencesFixture();
    saved.showIcon = false;
    saved.metrics.forEach(row => (row.enabled = false));
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.get('#status-app-icon').attributes('data-state')).toBe('checked');
    await wrapper.get('#status-cpuTemperature').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        showIcon: true,
        metrics: saved.metrics.map(row => ({ ...row, enabled: row.id === 'cpuTemperature' })),
      })
    );
  });

  it('reorders temperature with the keyboard, restores failed writes and reloads the saved order', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('#menu-bar-cpu-temperature').exists()).toBe(false);
    expect(wrapper.findAll('.drag-handle')).toHaveLength(15);
    expect(wrapper.get('[data-metric="cpuTemperature"] label').text()).toBe('cpuTemperature.label');
    const handle = wrapper.get('[data-metric="cpuTemperature"] .drag-handle');
    await handle.trigger('keydown', { key: ' ' });
    const position = wrapper
      .findAll('.metric-row')
      .findIndex(row => row.attributes('data-metric') === 'cpuTemperature');
    for (let step = 0; step < position; step++) await handle.trigger('keydown', { key: 'ArrowUp' });
    await handle.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    const saved = vi.mocked(ResidentService.savePreferences).mock.calls.at(-1)![0];
    expect(saved.metrics[0]?.id).toBe('cpuTemperature');
    expect(saved.metrics.find(row => row.id === 'cpuTemperature')?.enabled).toBe(false);
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('save failed'));
    await handle.trigger('keydown', { key: ' ' });
    await handle.trigger('keydown', { key: 'ArrowDown' });
    await handle.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(wrapper.findAll('.metric-row')[0]!.attributes('data-metric')).toBe('cpuTemperature');
    expect(wrapper.find('.settings-feedback[role="status"]').text()).toContain('systemStatus.saveFailed');
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    wrapper.unmount();
    const reloaded = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(reloaded);
    await flushPromises();
    await openConfiguration(reloaded);
    expect(reloaded.findAll('.metric-row')[0]!.attributes('data-metric')).toBe('cpuTemperature');
  });

  it('drags the temperature row independently and saves its enabled state and order', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#status-cpuTemperature').trigger('click');
    await flushPromises();
    wrapper.findAll('.metric-row').forEach((row, index) => {
      vi.spyOn(row.element, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, index * 42, 400, 42));
    });
    const hit = vi.spyOn(document, 'elementFromPoint').mockReturnValue(wrapper.findAll('.metric-row')[0]!.element);
    await wrapper.get('[data-metric="cpuTemperature"] .drag-handle').trigger('pointerdown', {
      pointerId: 1,
      button: 0,
      clientX: 10,
      clientY: wrapper.get('[data-metric="cpuTemperature"]').element.getBoundingClientRect().top + 20,
    });
    window.dispatchEvent(new PointerEvent('pointermove', { pointerId: 1, clientX: 10, clientY: 20 }));
    await flushPromises();
    window.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1, clientX: 10, clientY: 20 }));
    hit.mockRestore();
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(2);
    const saved = vi.mocked(ResidentService.savePreferences).mock.calls.at(-1)![0];
    expect(saved.metrics[0]).toEqual({ id: 'cpuTemperature', enabled: true });
    expect(saved.metrics.find(row => row.id === 'cpu')?.enabled).toBe(false);
  });

  it('shows only Linux tray controls and reorders the visible metrics', async () => {
    const saved = { ...preferencesFixture() };
    const gpuIndex = saved.metrics.findIndex(row => row.id === 'gpu');
    saved.metrics.splice(1, 0, ...saved.metrics.splice(gpuIndex, 1));
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const wrapper = mount(Settings, { props: { isMacOs: false, isLinux: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);

    expect(wrapper.findComponent(TaskbarPosition).exists()).toBe(false);
    expect(wrapper.findComponent(WindowsFeedback).exists()).toBe(false);
    expect(wrapper.find('#linux-tray-compact').exists()).toBe(true);
    expect(wrapper.get('[data-metric="cpuTemperature"] [role="status"]').text()).toBe(
      'systemStatus.platformUnsupported'
    );
    expect(wrapper.get('#resident-enabled-hint').text()).toBe('systemStatus.linuxDisplayHint');
    expect(wrapper.find('#taskbar-background').exists()).toBe(false);
    expect(wrapper.get('[data-metric="gpu"] [role="status"]').text()).toBe('systemStatus.platformUnsupported');
    expect(wrapper.findAll('.drag-handle')).toHaveLength(
      saved.metrics.filter(metric => !metric.id.endsWith('Frequency')).length
    );

    await wrapper.get('#linux-tray-compact').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ taskbarCompact: true, metrics: saved.metrics })
    );

    const memoryHandle = wrapper.get('[data-metric="memory"] .drag-handle');
    await memoryHandle.trigger('keydown', { key: ' ' });
    await memoryHandle.trigger('keydown', { key: 'ArrowUp' });
    await memoryHandle.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    const ids = saved.metrics.map(row => row.id);
    const visibleIds = ids.filter(id => !id.endsWith('Frequency'));
    const previousVisible = visibleIds[visibleIds.indexOf('memory') - 1]!;
    const memoryIndex = ids.indexOf('memory');
    const targetIndex = ids.indexOf(previousVisible);
    const [moving] = ids.splice(memoryIndex, 1);
    ids.splice(targetIndex, 0, moving!);
    expect(wrapper.findAll('.metric-row').map(row => row.attributes('data-metric'))).toEqual(
      ids.filter(id => !id.endsWith('Frequency'))
    );
    const preferences = vi.mocked(ResidentService.savePreferences).mock.calls.at(-1)![0];
    expect(preferences.metrics.map(row => row.id)).toEqual(ids);
    expect(preferences.metrics.find(row => row.id === 'gpu')?.enabled).toBe(false);
  });

  it('keeps the page compact and saves dialog edits without toggling residency', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    expect(ResidentService.catalogue).not.toHaveBeenCalled();
    await wrapper.get('.setting-copy').trigger('click');
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    await openConfiguration(wrapper);
    expect(wrapper.find('#resident-display-options').exists()).toBe(true);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    await wrapper.get('#status-cpu').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ enabled: true }));
    const done = wrapper.findAll('button').find(button => button.text() === 'systemStatus.done');
    expect(done).toBeDefined();
    await done!.trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    await openConfiguration(wrapper);
    expect(wrapper.get('#status-cpu').attributes('aria-checked')).toBe('true');
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
  });

  it('shows a Windows fallback on the main page without opening configuration', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
    });
    vi.mocked(ResidentService.displayStatus).mockResolvedValue('noSpace');
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    expect(wrapper.get('.status-settings [role="status"]').text()).toBe('systemStatus.taskbarNoSpace');
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    expect(ResidentService.catalogue).not.toHaveBeenCalled();
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-configure').exists()).toBe(false);
    expect(wrapper.find('.status-settings [role="status"]').exists()).toBe(false);
    expect(wrapper.get('#resident-enabled-hint').text()).toBe('systemStatus.displayHint');
  });

  it.each([true, false])('locks the last selected item with Logo=%s until another item is selected', async showIcon => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      showIcon,
      metrics: preferencesFixture().metrics.map(row => ({ ...row, enabled: !showIcon && row.id === 'memory' })),
    });
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const last = wrapper.get(showIcon ? '#status-app-icon' : '#status-memory');
    expect(last.attributes('disabled')).toBeDefined();
    expect(last.attributes('aria-checked')).toBe('true');
    await wrapper.get('#status-cpu').trigger('click');
    await flushPromises();
    expect(last.attributes('disabled')).toBeUndefined();
    await last.trigger('click');
    await flushPromises();
    expect(wrapper.get('#status-cpu').attributes('disabled')).toBeDefined();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ showIcon: false }));
  });

  it('shows the native Logo fallback as selected for older empty preferences without writing on load', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      showIcon: false,
      metrics: preferencesFixture().metrics.map(row => ({ ...row, enabled: false })),
    });
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.get('#status-app-icon').attributes('aria-checked')).toBe('true');
    expect(wrapper.get('#status-app-icon').attributes('disabled')).toBeDefined();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    expect(wrapper.text()).not.toContain('systemStatus.keepEntry');
    await wrapper.get('#status-memory').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ showIcon: true }));
    expect(wrapper.get('#status-app-icon').attributes('disabled')).toBeUndefined();
  });

  it('collapses only after a committed disable and restores every saved display choice', async () => {
    const saved = { ...preferencesFixture(), networkInterface: 'wifi', taskbarBackground: false };
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith({ ...saved, enabled: false });
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    await openConfiguration(wrapper);
    expect(wrapper.find('#resident-display-options').exists()).toBe(true);
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith({ ...saved, revision: 1 });
    expect(ResidentService.quit).not.toHaveBeenCalled();
  });

  it('keeps the retry visible when enabling a collapsed display fails', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({ ...preferencesFixture(), enabled: false });
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('storage'));
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.get('#resident-enabled').attributes('aria-checked')).toBe('false');
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    expect(wrapper.find('.settings-feedback button').exists()).toBe(true);
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    await openConfiguration(wrapper);
    expect(wrapper.find('#resident-display-options').exists()).toBe(true);
  });

  it('shows typed fallback feedback only for an active taskbar selection', async () => {
    vi.mocked(ResidentService.displayStatus).mockResolvedValue('noSpace');
    const preferences = { ...preferencesFixture() };
    const wrapper = mount(WindowsFeedback, { props: { preferences }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe('systemStatus.taskbarNoSpace');
    await wrapper.setProps({ preferences: { ...preferences, enabled: false } });
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
  });

  it('keeps a newer display event when the initial status request resolves late', async () => {
    let finish!: (value: 'noSpace') => void;
    vi.mocked(ResidentService.displayStatus).mockReturnValue(new Promise(resolve => (finish = resolve)));
    const stop = vi.fn();
    vi.mocked(ResidentService.onDisplayStatus).mockImplementation(async listener => {
      listener('taskbar');
      return stop;
    });
    const wrapper = mount(WindowsFeedback, {
      props: { preferences: { ...preferencesFixture() } },
      global: global(),
    });
    await flushPromises();
    finish('noSpace');
    await flushPromises();
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    wrapper.unmount();
    expect(stop).toHaveBeenCalledOnce();
  });

  it('switches Windows presentation while preserving metric choices and enables taskbar reordering', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('input[name="windows-display-mode"]').exists()).toBe(false);
    expect(wrapper.findAll('.drag-handle')).toHaveLength(
      preferencesFixture().metrics.filter(metric => !metric.id.endsWith('Frequency') || metric.id === 'cpuFrequency')
        .length
    );
    await wrapper.get('input[name="taskbar-position"][value="left"]').setValue(true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        taskbarPosition: 'left',
        metrics: preferencesFixture().metrics,
      })
    );
    await wrapper.get('input[name="taskbar-position"][value="auto"]').setValue(true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ schemaVersion: 42, taskbarPosition: 'auto' })
    );
  });

  it('saves compact density without changing position or metric choices', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#taskbar-compact').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ taskbarCompact: true, metrics: preferencesFixture().metrics })
    );
    await wrapper.get('#taskbar-compact').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ taskbarCompact: false })
    );
  });

  it('saves the taskbar background toggle without a display destination selector', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('#taskbar-background').exists()).toBe(true);
    expect(wrapper.find('input[name="windows-display-mode"]').exists()).toBe(false);
    await wrapper.get('#taskbar-background').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ taskbarBackground: false })
    );
  });

  it('commits keyboard reordering once and cancels without changing saved order', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global(), attachTo: document.body });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const handle = wrapper.findAll('.drag-handle')[1]!;
    await handle.trigger('keydown', { key: ' ' });
    await handle.trigger('keydown', { key: 'ArrowUp' });
    await flushPromises();
    expect(document.activeElement).toBe(wrapper.get('[data-metric="memory"] .drag-handle').element);
    expect(wrapper.get('[aria-live="polite"]').text()).toContain(`: 1/${wrapper.findAll('.metric-row').length}`);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    await handle.trigger('keydown', { key: 'Escape' });
    expect(wrapper.findAll('.metric-heading label')[0]!.text()).toBe('systemStatus.cpuShort');
    await handle.trigger('keydown', { key: ' ' });
    await handle.trigger('keydown', { key: 'ArrowUp' });
    await handle.trigger('keydown', { key: 'Enter' });
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
    expect(vi.mocked(ResidentService.savePreferences).mock.calls[0]?.[0].metrics[0]?.id).toBe('memory');
  });

  it('offers Windows reordering and changes only the selected metric', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.findAll('.drag-handle')).toHaveLength(
      preferencesFixture().metrics.filter(metric => !metric.id.endsWith('Frequency') || metric.id === 'cpuFrequency')
        .length
    );
    await wrapper.get('#status-network').trigger('click');
    await flushPromises();
    const preferences = vi.mocked(ResidentService.savePreferences).mock.calls[0]![0];
    expect(preferences.metrics.filter(metric => metric.enabled).map(metric => metric.id)).toEqual([
      'memory',
      'network',
    ]);
    expect(preferences.enabled).toBe(true);
  });

  it('opens source choices at the arrow, saves a choice and closes without inline controls', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      metrics: preferencesFixture().metrics.map(metric => ({ ...metric, enabled: true })),
    });
    vi.mocked(ResidentService.catalogue).mockResolvedValue({
      ...readingFixture(),
      interfaces: [
        {
          id: 'wifi',
          name: 'Wi-Fi',
          kind: 'wifi',
          connected: true,
          physical: true,
          details: null,
          defaultRouteMetric: 1,
        },
      ],
    });
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global(), attachTo: document.body });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const network = wrapper.get('[data-metric="network"] .selection-toggle');
    await network.trigger('keydown', { key: 'ArrowDown' });
    await flushPromises();
    expect(network.attributes('aria-expanded')).toBe('true');
    expect(wrapper.find('.selection-options').exists()).toBe(false);
    const choice = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]')).find(
      item => item.textContent === 'Wi-Fi'
    );
    expect(choice).toBeDefined();
    choice!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ networkInterface: 'wifi' })
    );
    expect(network.attributes('aria-expanded')).toBe('false');
    expect(wrapper.get('[data-metric="network"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe('Wi-Fi');
    expect(network.text()).toBe('Wi-Fi');
    await wrapper.get('#status-network').trigger('click');
    await flushPromises();
    expect(network.attributes('disabled')).toBeDefined();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ networkInterface: 'wifi' })
    );
  });

  it('saves the Logo checkbox without changing metric selections', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.get('.logo-marker img').attributes('src')).toBe('/mangodisk.svg');
    await wrapper.get('#status-app-icon').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ showIcon: false, metrics: preferencesFixture().metrics })
    );
  });

  it('reconnects the live catalogue after a subscription failure', async () => {
    vi.mocked(ResidentService.onReading).mockRejectedValueOnce(new Error('subscription'));
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const retry = wrapper.findAll('button').find(button => button.text() === 'monitoring.refresh');
    expect(retry).toBeDefined();
    await retry!.trigger('click');
    await flushPromises();
    expect(ResidentService.onReading).toHaveBeenCalledTimes(2);
    expect(ResidentService.catalogue).toHaveBeenCalledTimes(1);
  });

  it.each(['pointercancel', 'Escape', 'blur', 'resize', 'scroll'])(
    'commits one drop and cancels on %s without saving',
    async cancellation => {
      const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      wrapper.findAll('.metric-row').forEach((row, index) => {
        vi.spyOn(row.element, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, index * 42, 400, 42));
      });
      const hit = vi.spyOn(document, 'elementFromPoint');
      const move = () =>
        window.dispatchEvent(new PointerEvent('pointermove', { pointerId: 1, clientX: 10, clientY: 10 }));
      await wrapper
        .findAll('.drag-handle')[1]!
        .trigger('pointerdown', { pointerId: 1, button: 0, clientX: 10, clientY: 55 });
      hit.mockReturnValue(wrapper.findAll('.metric-row')[0]!.element);
      move();
      await flushPromises();
      expect(document.querySelectorAll('.drag-preview')).toHaveLength(1);
      expect(document.querySelector('.drag-preview')?.getAttribute('aria-hidden')).toBe('true');
      expect(document.querySelector('.drag-preview [id]')).toBeNull();
      expect(wrapper.get('.pointer-moving').attributes('data-metric')).toBe('memory');
      move();
      await flushPromises();
      expect(wrapper.findAll('.metric-row')[0]!.attributes('data-metric')).toBe('memory');
      expect(ResidentService.savePreferences).not.toHaveBeenCalled();
      window.dispatchEvent(
        cancellation === 'Escape'
          ? new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
          : new Event(cancellation)
      );
      await flushPromises();
      expect(wrapper.findAll('.metric-heading label')[0]!.text()).toBe('systemStatus.cpuShort');
      expect(document.querySelector('.drag-preview')).toBeNull();
      await wrapper
        .findAll('.drag-handle')[1]!
        .trigger('pointerdown', { pointerId: 1, button: 0, clientX: 10, clientY: 55 });
      hit.mockReturnValue(wrapper.findAll('.metric-row')[0]!.element);
      move();
      window.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1, clientX: 10, clientY: 10 }));
      hit.mockRestore();
      await flushPromises();
      expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
      expect(vi.mocked(ResidentService.savePreferences).mock.calls[0]![0].metrics[0]!.id).toBe('memory');
      expect(document.querySelector('.drag-preview')).toBeNull();
    }
  );

  it('keeps a saved disconnected interface in the selector', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      networkInterface: 'disconnected',
      metrics: preferencesFixture().metrics.map(metric => ({ ...metric, enabled: true })),
    });
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.get('[data-metric="network"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe(
      'systemStatus.savedDisconnected'
    );
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });

  it('updates closed selector labels when the interface language changes', async () => {
    const i18n = createI18n({
      legacy: false,
      locale: 'en',
      missingWarn: false,
      fallbackWarn: false,
      messages: {
        en: { systemStatus: { automatic: 'Automatic', systemDiskAutomatic: 'System disk (automatic)' } },
        ja: { systemStatus: { automatic: '自動選択', systemDiskAutomatic: 'システムディスク（自動）' } },
      },
    });
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      metrics: preferencesFixture().metrics.map(metric => ({ ...metric, enabled: true })),
    });
    const wrapper = mount(Settings, {
      props: { isMacOs: false },
      global: { ...global(), plugins: [createPinia(), i18n] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.get('[data-metric="network"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe('Automatic');
    expect(wrapper.get('[data-metric="disk"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe(
      'System disk (automatic)\nsystemStatus.savedDisconnected'
    );

    i18n.global.locale.value = 'ja';
    await flushPromises();
    expect(wrapper.get('[data-metric="network"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe('自動選択');
    expect(wrapper.get('[data-metric="disk"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe(
      'システムディスク（自動）\nsystemStatus.savedDisconnected'
    );
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });
});

describe('per-item color configuration', () => {
  it('saves fan colors without changing selected fans or other metric rules', async () => {
    const saved = preferencesFixture();
    saved.fanDisplay = 'selected';
    saved.fanIds = [0, 1];
    vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('#color-configure-fanSpeed').trigger('click');
    expect(wrapper.find('#color-editor-fanSpeed').exists()).toBe(true);
    const rule = { id: 'fanSpeed', enabled: true, rules: [{ condition: 'atLeast', value: 2000, color: 'blue' }] };
    wrapper.getComponent({ name: 'MdStatusColorRuleEditor' }).vm.$emit('save', rule);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        fanDisplay: 'selected',
        fanIds: [0, 1],
        colorRules: saved.colorRules.map(existing => (existing.id === 'fanSpeed' ? rule : existing)),
      })
    );
  });
  it('opens a separate dialog and keeps source choices independent', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('[data-metric="cpu"] .color-toggle').trigger('click');
    expect(wrapper.find('#color-editor-cpu').exists()).toBe(true);
    expect(wrapper.get('[data-metric="cpu"]').find('.color-rule-editor').exists()).toBe(false);
    await wrapper.get('[data-metric="gpuTemperature"] .color-toggle').trigger('click');
    expect(wrapper.find('#color-editor-cpu').exists()).toBe(false);
    expect(wrapper.find('#color-editor-gpuTemperature').exists()).toBe(true);
    expect(wrapper.find('[data-metric="gpuTemperature"] .selection-toggle').exists()).toBe(false);
    expect(wrapper.find('[data-metric="network"] .color-toggle').exists()).toBe(false);
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    wrapper.getComponent({ name: 'MdStatusColorRuleEditor' }).vm.$emit('cancel');
    await flushPromises();
    expect(wrapper.find('.color-rule-editor').exists()).toBe(false);
    expect(document.activeElement).toBe(wrapper.get('#color-configure-gpuTemperature').element);
  });
  it('retains the editor and draft after a rejected save', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('[data-metric="cpuTemperature"] .color-toggle').trigger('click');
    vi.mocked(ResidentService.savePreferences).mockRejectedValueOnce(new Error('write failed'));
    const editor = wrapper.getComponent({ name: 'MdStatusColorRuleEditor' });
    const rule = preferencesFixture().colorRules[1]!;
    editor.vm.$emit('save', rule);
    await flushPromises();
    expect(wrapper.find('#color-editor-cpuTemperature').exists()).toBe(true);
    expect(wrapper.text()).toContain('systemStatus.saveFailed');
  });
});

describe('GPU temperature display settings', () => {
  function gpuReading(revision = 1, adapterId = 'a') {
    const reading = readingFixture(revision);
    reading.observedAtMs = revision;
    reading.gpuTemperature = {
      status: 'ready',
      sampledAtMs: revision,
      value: { adapterId, adapterName: adapterId, celsius: 52, kind: 'device', source: 'driver', sensorCount: 1 },
    };
    reading.gpuAdapters = [
      { id: 'a', name: 'GPU 0' },
      { id: 'b', name: 'GPU 1' },
    ];
    reading.gpuDevices = [
      {
        adapterId,
        adapterName: adapterId,
        usage: {
          status: 'ready',
          sampledAtMs: revision,
          value: { adapterId, adapterName: adapterId, usedPercent: 50, details: null },
        },
        usageHistory: [],
        temperature: reading.gpuTemperature,
        temperatureHistory: [],
      },
    ];
    return reading;
  }
  beforeEach(() => {
    vi.mocked(ResidentService.catalogue).mockResolvedValue(gpuReading());
  });
  it('reloads the GPU chosen in another window before editing display settings', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    const external = { ...preferencesFixture(), revision: 9, gpuAdapter: 'gpu-b' };
    vi.mocked(ResidentService.preferences).mockResolvedValue(external);
    vi.mocked(ResidentService.catalogue).mockResolvedValue({
      ...gpuReading(1, 'gpu-b'),
      gpuAdapters: [{ id: 'gpu-b', name: 'GPU 1 · Radeon' }],
    });
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="gpuTemperature"]');
    expect(row.find('.selection-toggle').exists()).toBe(false);
    await row.get('#status-gpuTemperature').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ revision: 9, gpuAdapter: 'gpu-b' })
    );
  });
  it('starts unselected on macOS and shares GPU selection without requiring GPU usage display', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="gpuTemperature"]');
    expect(row.get('#status-gpuTemperature').attributes('data-state')).toBe('unchecked');
    expect(row.find('.md-help-action').exists()).toBe(false);
    expect(row.get('.selection-toggle').attributes('disabled')).toBeUndefined();
    await row.get('#status-gpuTemperature').trigger('click');
    await flushPromises();
    const saved = vi.mocked(ResidentService.savePreferences).mock.calls.at(-1)![0];
    expect(saved.metrics.find(item => item.id === 'gpuTemperature')?.enabled).toBe(true);
    expect(saved.metrics.find(item => item.id === 'gpu')?.enabled).toBe(false);
  });
  it.each([true, false])(
    'explains unsupported GPU temperature and keeps device selection with isMacOs=%s',
    async isMacOs => {
      const initial = readingFixture(1);
      initial.gpuTemperature.status = 'unsupported';
      initial.gpuAdapters = [
        { id: 'a', name: 'Unsupported GPU' },
        { id: 'b', name: 'Supported GPU' },
      ];
      vi.mocked(ResidentService.catalogue).mockResolvedValue(initial);
      vi.mocked(WindowsMonitoringService.observe).mockResolvedValue({
        state: 'unsupportedHardware',
        driverAvailable: false,
        serviceInstalled: false,
      });
      const wrapper = mount(Settings, { props: { isMacOs }, global: global() });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      const row = wrapper.get('[data-metric="gpuTemperature"]');
      expect(row.get('[role="status"]').text()).toBe('systemStatus.unsupported');
      if (!isMacOs) {
        expect(wrapper.get('[data-metric="cpuTemperature"] [role="status"]').text()).toBe(
          row.get('[role="status"]').text()
        );
      }
      expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
      expect(row.get('.selection-toggle').attributes('disabled')).toBeUndefined();
      row.getComponent(Select).vm.$emit('update:modelValue', 'selected');
      await flushPromises();
      await row.get('#gpu-choice-temperature-1').trigger('click');
      await flushPromises();
      await row.get('#gpu-choice-temperature-0').trigger('click');
      await flushPromises();
      const savedChoices = vi.mocked(ResidentService.savePreferences).mock.calls.length;
      expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
        expect.objectContaining({ gpuAdapter: null, gpuTemperatureDisplay: { mode: 'selected', adapterIds: ['b'] } })
      );
      const onReading = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
      const pending = readingFixture(2);
      pending.gpuAdapters = initial.gpuAdapters;
      pending.gpuDevices = [
        {
          adapterId: 'b',
          adapterName: 'Supported GPU',
          usage: { status: 'loading', sampledAtMs: null, value: null },
          usageHistory: [],
          temperature: { status: 'loading', sampledAtMs: null, value: null },
          temperatureHistory: [],
        },
      ];
      onReading(pending);
      await flushPromises();
      expect(row.get('[role="status"]').text()).toBe('systemStatus.loading');
      expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
      row.getComponent({ name: 'MdCheckbox' }).vm.$emit('update:modelValue', true);
      await flushPromises();
      expect(ResidentService.savePreferences).toHaveBeenCalledTimes(savedChoices + 1);
      const unsupported = readingFixture(3);
      unsupported.gpuAdapters = initial.gpuAdapters;
      unsupported.gpuDevices = pending.gpuDevices.map(device => ({
        ...device,
        temperature: { ...device.temperature, status: 'unsupported' },
      }));
      unsupported.gpuTemperature.status = 'unsupported';
      onReading(unsupported);
      await flushPromises();
      expect(row.get('[role="status"]').text()).toBe('systemStatus.unsupported');
      expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
      const supported = gpuReading(4, 'b');
      supported.gpuAdapters = initial.gpuAdapters;
      onReading(supported);
      await flushPromises();
      expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
      expect(row.find('[role="status"]').exists()).toBe(false);
    }
  );
  it.each(['stale', 'failed', 'disconnected'] as const)(
    'allows enabling temperature while preserving a %s reading',
    async status => {
      const reading = gpuReading();
      reading.gpuTemperature.status = status;
      reading.gpuDevices[0]!.temperature.status = status;
      vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
      const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
      wrappers.push(wrapper);
      await flushPromises();
      await openConfiguration(wrapper);
      const row = wrapper.get('[data-metric="gpuTemperature"]');
      expect(row.get('[role="status"]').text()).toBe(`systemStatus.${status === 'failed' ? 'readFailed' : status}`);
      expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
      row.getComponent({ name: 'MdCheckbox' }).vm.$emit('update:modelValue', true);
      await flushPromises();
      expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
        expect.objectContaining({
          metrics: expect.arrayContaining([{ id: 'gpuTemperature', enabled: true }]),
        })
      );
    }
  );
  it('rejects temperature readings belonging to the previously selected adapter', async () => {
    const preferences = { ...preferencesFixture(), gpuDisplay: { mode: 'selected' as const, adapterIds: ['b'] } };
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferences);
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="gpuTemperature"]');
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
    expect(row.get('[role="status"]').text()).toBe('systemStatus.loading');
    row.getComponent({ name: 'MdCheckbox' }).vm.$emit('update:modelValue', true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        metrics: expect.arrayContaining([{ id: 'gpuTemperature', enabled: true }]),
      })
    );
  });
  it('selects temperature independently and restores following without changing usage', async () => {
    const catalog = gpuReading();
    catalog.gpuAdapters = [
      { id: 'a', name: 'GPU 0' },
      { id: 'b', name: 'GPU 1' },
    ];
    vi.mocked(ResidentService.catalogue).mockResolvedValue(catalog);
    vi.mocked(ResidentService.preferences).mockResolvedValue({ ...preferencesFixture(), gpuAdapter: 'a' });
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="gpuTemperature"]');
    expect(row.get('.selection-toggle').text()).toBe('gpuTemperature.followUsage');
    row.getComponent(Select).vm.$emit('update:modelValue', 'selected');
    await flushPromises();
    await row.get('#gpu-choice-temperature-1').trigger('click');
    await flushPromises();
    await row.get('#gpu-choice-temperature-0').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ gpuAdapter: 'a', gpuTemperatureDisplay: { mode: 'selected', adapterIds: ['b'] } })
    );
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
    const accept = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
    accept(gpuReading(2, 'b'));
    await flushPromises();
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
    row.getComponent(Select).vm.$emit('update:modelValue', 'followUsage');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ gpuAdapter: 'a', gpuTemperatureDisplay: { mode: 'automatic' } })
    );
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
    accept(gpuReading(3, 'a'));
    await flushPromises();
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
  });
  it('retains fixed temperature when usage changes and rejects another device sample', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      gpuAdapter: 'a',
      gpuTemperatureDisplay: { mode: 'selected', adapterIds: ['b'] },
    });
    vi.mocked(ResidentService.catalogue).mockResolvedValue(gpuReading(1, 'b'));
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const row = wrapper.get('[data-metric="gpuTemperature"]');
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
    const store = useResidentSettingsStore();
    await store.change({ gpuAdapter: 'c' });
    await flushPromises();
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
    const accept = vi.mocked(ResidentService.onReading).mock.calls.at(-1)![0];
    accept(gpuReading(2, 'a'));
    await flushPromises();
    expect(row.get('#status-gpuTemperature').attributes('disabled')).toBeUndefined();
  });

  it('allows an unavailable saved temperature item to be turned off', async () => {
    const preferences = preferencesFixture();
    preferences.metrics.find(item => item.id === 'gpuTemperature')!.enabled = true;
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferences);
    const reading = readingFixture();
    reading.gpuTemperature.status = 'unsupported';
    vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    const checkbox = wrapper.get('#status-gpuTemperature');
    expect(checkbox.attributes('disabled')).toBeUndefined();
    await checkbox.trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ metrics: expect.arrayContaining([{ id: 'gpuTemperature', enabled: false }]) })
    );
  });
  it('offers the shared GPU temperature display on Windows', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('[data-metric="gpuTemperature"]').exists()).toBe(true);
  });
});

it('selects several menu-bar GPUs with checkboxes without discarding detail or temperature choices', async () => {
  const saved = preferencesFixture();
  saved.gpuAdapter = 'a';
  saved.gpuTemperatureDisplay = { mode: 'selected', adapterIds: ['b'] };
  saved.metrics.find(metric => metric.id === 'gpu')!.enabled = true;
  const reading = readingFixture();
  reading.gpuAdapters = [
    { id: 'a', name: 'GPU 0 · Intel' },
    { id: 'b', name: 'GPU 1 · Radeon' },
  ];
  vi.mocked(ResidentService.preferences).mockResolvedValue(saved);
  vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
  const wrapper = mount(Settings, { props: { isMacOs: true }, global: global() });
  wrappers.push(wrapper);
  await flushPromises();
  await openConfiguration(wrapper);
  expect(wrapper.find('#gpu-choice-usage-0').exists()).toBe(false);
  wrapper.get('[data-metric="gpu"]').getComponent(Select).vm.$emit('update:modelValue', 'selected');
  await flushPromises();
  expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
    expect.objectContaining({ gpuAdapter: 'a', gpuDisplay: { mode: 'selected', adapterIds: ['a'] } })
  );
  expect(wrapper.get('#gpu-choice-usage-0').attributes('disabled')).toBeDefined();
  await wrapper.get('#gpu-choice-usage-1').trigger('click');
  await flushPromises();
  expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
    expect.objectContaining({
      gpuDisplay: { mode: 'selected', adapterIds: ['a', 'b'] },
      gpuTemperatureDisplay: { mode: 'selected', adapterIds: ['b'] },
    })
  );
  await wrapper.get('#gpu-choice-temperature-0').trigger('click');
  await flushPromises();
  expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
    expect.objectContaining({
      gpuDisplay: { mode: 'selected', adapterIds: ['a', 'b'] },
      gpuTemperatureDisplay: { mode: 'selected', adapterIds: ['b', 'a'] },
    })
  );
});

it.each([
  { isMacOs: false, classes: ['average'] as const, visible: ['cpuFrequency'] },
  {
    isMacOs: true,
    classes: ['efficiency', 'performance'] as const,
    visible: ['cpuEfficiencyFrequency', 'cpuPerformanceFrequency'],
  },
  {
    isMacOs: true,
    classes: ['performance', 'super'] as const,
    visible: ['cpuPerformanceFrequency', 'cpuSuperFrequency'],
  },
  { isMacOs: true, classes: [] as const, visible: [] },
])('offers only installed frequency classes on macOS=$isMacOs: $classes', async ({ isMacOs, classes, visible }) => {
  const reading = readingFixture();
  reading.cpuIdentity = { model: null, nominalFrequencyMhz: null, frequencyClasses: [...classes] };
  vi.mocked(ResidentService.catalogue).mockResolvedValue(reading);
  const wrapper = mount(Settings, { props: { isMacOs }, global: global() });
  wrappers.push(wrapper);
  await flushPromises();
  await openConfiguration(wrapper);
  for (const id of ['cpuFrequency', 'cpuEfficiencyFrequency', 'cpuPerformanceFrequency', 'cpuSuperFrequency']) {
    expect(wrapper.find(`#status-${id}`).exists()).toBe(visible.some(value => value === id));
    expect(wrapper.find(`#color-configure-${id}`).exists()).toBe(false);
  }
});
