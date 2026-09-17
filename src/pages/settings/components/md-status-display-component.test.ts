// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import Settings from './md-status-display-settings.vue';
import WindowsMode from './md-windows-display-mode.vue';
import { Select } from '@/components/ui/select';
import WindowsFeedback from './md-windows-display-feedback.vue';
import { ResidentService } from '@/lib/services/resident-service';
import { preferencesFixture, readingFixture } from '@/tests/fixtures/resident';

vi.mock('@/lib/services/resident-service', () => ({
  ResidentService: {
    preferences: vi.fn(),
    savePreferences: vi.fn(),
    catalogue: vi.fn(),
    onReading: vi.fn(),
    preview: vi.fn(),
    displayStatus: vi.fn(),
    onDisplayStatus: vi.fn(),
    releaseMemory: vi.fn(),
    quit: vi.fn(),
    openMain: vi.fn(),
  },
}));
vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('@/lib/services/byte-size-service', () => ({ ByteSizeService: { memory: (value: number) => `${value} B` } }));
function global() {
  return {
    // Reka's Teleport wrapper shares Vue's stub name. Preserve its slot so
    // dialog content remains testable without replacing the dialog behavior.
    renderStubDefaultSlot: true,
    stubs: { teleport: true },
    plugins: [
      createPinia(),
      createI18n({ legacy: false, locale: 'en', messages: { en: {} }, missingWarn: false, fallbackWarn: false }),
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
describe('status display interactions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(ResidentService.displayStatus).mockResolvedValue('tray');
    vi.mocked(ResidentService.onDisplayStatus).mockResolvedValue(vi.fn());
    vi.mocked(ResidentService.preferences).mockResolvedValue(preferencesFixture());
    vi.mocked(ResidentService.savePreferences).mockImplementation(async value => ({
      ...value,
      revision: value.revision + 1,
    }));
    vi.mocked(ResidentService.catalogue).mockResolvedValue(readingFixture());
    vi.mocked(ResidentService.onReading).mockResolvedValue(vi.fn());
  });
  afterEach(() => {
    wrappers.splice(0).forEach(wrapper => wrapper.unmount());
    vi.useRealTimers();
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
    const selectors = wrapper.findAllComponents(Select);
    selectors[0]!.vm.$emit('update:modelValue', '60');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ usageWarningPercent: 60, usageCriticalPercent: 90 })
    );
    await wrapper.get('#usage-colors').trigger('click');
    await flushPromises();
    expect(wrapper.find('#usage-warning').exists()).toBe(false);
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ usageColors: false, usageWarningPercent: 60 })
    );
  });

  it('offers shared usage colors on Windows without the macOS density setting', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('#usage-colors').exists()).toBe(true);
    expect(wrapper.find('#menu-bar-compact').exists()).toBe(false);
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
    await wrapper.get('#status-cpu').setValue(true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(expect.objectContaining({ enabled: true }));
    const done = wrapper.findAll('button').find(button => button.text() === 'systemStatus.done');
    expect(done).toBeDefined();
    await done!.trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    await openConfiguration(wrapper);
    expect((wrapper.get('#status-cpu').element as HTMLInputElement).checked).toBe(true);
    expect(ResidentService.savePreferences).toHaveBeenCalledTimes(1);
  });

  it('shows a Windows fallback on the main page without opening configuration', async () => {
    vi.mocked(ResidentService.preferences).mockResolvedValue({
      ...preferencesFixture(),
      windowsDisplayMode: 'taskbar',
    });
    vi.mocked(ResidentService.displayStatus).mockResolvedValue('noSpace');
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    expect(wrapper.get('.status-settings .settings-list [role="status"]').text()).toBe('systemStatus.taskbarNoSpace');
    expect(wrapper.find('#resident-display-options').exists()).toBe(false);
    expect(ResidentService.catalogue).not.toHaveBeenCalled();
    await wrapper.get('#resident-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.find('#resident-configure').exists()).toBe(false);
    expect(wrapper.find('.status-settings .settings-list [role="status"]').exists()).toBe(false);
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
    expect((last.element as HTMLInputElement).checked).toBe(true);
    await wrapper.get('#status-cpu').setValue(true);
    await flushPromises();
    expect(last.attributes('disabled')).toBeUndefined();
    await last.setValue(false);
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
    expect((wrapper.get('#status-app-icon').element as HTMLInputElement).checked).toBe(true);
    expect(wrapper.get('#status-app-icon').attributes('disabled')).toBeDefined();
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
    expect(wrapper.text()).not.toContain('systemStatus.keepEntry');
    await wrapper.get('#status-memory').setValue(true);
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
    const preferences = { ...preferencesFixture(), windowsDisplayMode: 'taskbar' as const };
    const wrapper = mount(WindowsFeedback, { props: { preferences }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe('systemStatus.taskbarNoSpace');
    await wrapper.setProps({ preferences: { ...preferences, enabled: false } });
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    await wrapper.setProps({ preferences: { ...preferences, windowsDisplayMode: 'tray' } });
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
      props: { preferences: { ...preferencesFixture(), windowsDisplayMode: 'taskbar' } },
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
    expect(wrapper.findAll('.drag-handle')).toHaveLength(0);
    await wrapper.get('input[name="windows-display-mode"][value="taskbar"]').setValue(true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        windowsDisplayMode: 'taskbar',
        metrics: preferencesFixture().metrics,
      })
    );
    expect(wrapper.findAll('.drag-handle')).toHaveLength(4);
    await wrapper.get('input[name="taskbar-position"][value="left"]').setValue(true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({
        taskbarPosition: 'left',
        windowsDisplayMode: 'taskbar',
        metrics: preferencesFixture().metrics,
      })
    );
    await wrapper.get('input[name="taskbar-position"][value="auto"]').setValue(true);
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ schemaVersion: 8, taskbarPosition: 'auto' })
    );
    await wrapper.get('input[name="windows-display-mode"][value="tray"]').setValue(true);
    await flushPromises();
    expect(wrapper.findAll('.drag-handle')).toHaveLength(0);
  });

  it('saves compact density without changing position or metric choices', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    await wrapper.get('input[name="windows-display-mode"][value="taskbar"]').setValue(true);
    await flushPromises();
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

  it('saves the background toggle and retains it when switching presentation modes', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.find('#taskbar-background').exists()).toBe(false);
    wrapper.getComponent(WindowsMode).vm.$emit('change', 'taskbar');
    await flushPromises();
    await wrapper.get('#taskbar-background').trigger('click');
    await flushPromises();
    expect(ResidentService.savePreferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ taskbarBackground: false, windowsDisplayMode: 'taskbar' })
    );
    wrapper.getComponent(WindowsMode).vm.$emit('change', 'tray');
    await flushPromises();
    expect(wrapper.find('#taskbar-background').exists()).toBe(false);
    wrapper.getComponent(WindowsMode).vm.$emit('change', 'taskbar');
    await flushPromises();
    expect(wrapper.get('#taskbar-background').attributes('aria-checked')).toBe('false');
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

  it('omits Windows reordering controls and changes only the selected metric', async () => {
    const wrapper = mount(Settings, { props: { isMacOs: false }, global: global() });
    wrappers.push(wrapper);
    await flushPromises();
    await openConfiguration(wrapper);
    expect(wrapper.findAll('.drag-handle')).toHaveLength(0);
    await wrapper.get('#status-network').setValue(true);
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
      interfaces: [{ id: 'wifi', name: 'Wi-Fi', kind: 'wifi', connected: true, physical: true, defaultRouteMetric: 1 }],
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
    await wrapper.get('#status-network').setValue(false);
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
    await wrapper.get('#status-app-icon').setValue(false);
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
        en: { systemStatus: { automatic: 'Automatic', systemDisk: 'System disk' } },
        ja: { systemStatus: { automatic: '自動選択', systemDisk: 'システムディスク' } },
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
    expect(wrapper.get('[data-metric="disk"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe('System disk');

    i18n.global.locale.value = 'ja';
    await flushPromises();
    expect(wrapper.get('[data-metric="network"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe('自動選択');
    expect(wrapper.get('[data-metric="disk"]').getComponent({ name: 'MdTooltip' }).props('text')).toBe(
      'システムディスク'
    );
    expect(ResidentService.savePreferences).not.toHaveBeenCalled();
  });
});
