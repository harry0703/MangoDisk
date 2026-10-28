// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import FloatingMonitor from './index.vue';
import type { FloatingSnapshot } from '@/lib/models/resident/floating-monitor';
import en from '@/locales/en-US.json';

const native = vi.hoisted(() => ({
  ready: vi.fn(),
  layout: vi.fn(),
  onSnapshot: vi.fn(),
  onAppearanceChanged: vi.fn(),
  menu: vi.fn(),
  drag: vi.fn(),
  togglePage: vi.fn(),
}));
vi.mock('@/lib/services/resident/floating-monitor-service', () => ({ FloatingMonitorService: native }));
vi.mock('@/lib/services/resident/floating-layout-service', () => ({
  FloatingLayoutService: {
    measure: (_root: HTMLElement, rows: { entry: { id: string; gpuDevice?: { id: string } }; label: string }[]) =>
      rows.map(({ entry, label }) => ({
        id: entry.id,
        gpuDeviceId: entry.gpuDevice?.id ?? null,
        labelWidth: label.length * 6,
        valueWidth: 49,
      })),
  },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { info: vi.fn(), warn: vi.fn() } }));
const locales = Object.fromEntries(
  Object.entries(
    import.meta.glob<typeof en>('/src/locales/*.json', {
      eager: true,
      import: 'default',
    })
  ).map(([path, messages]) => [path.split('/').at(-1)!.replace('.json', ''), messages])
);
const wrappers: ReturnType<typeof mount>[] = [];
let accept: (value: FloatingSnapshot) => void;
let stop: Mock<() => void>;
function snapshot(): FloatingSnapshot {
  return {
    schemaVersion: 12,
    interactive: true,
    contextMenu: true,
    columns: [70, 90, 85],
    preferences: {
      layout: 'horizontal',
      alwaysOnTop: true,
      locked: false,
      backgroundOpacity: 100,
      snapToEdges: true,
      fontSize: 'standard',
      bold: false,
      dividers: true,
      clickThrough: false,
      hideOnFullscreen: true,
      background: 'default',
    },
    entries: [
      { id: 'cpu', digits: '12', marker: 'C', text: 'CPU 12%', tooltip: 'CPU 12%', tone: 'green' },
      { id: 'fanSpeed', digits: '1602', marker: 'F', text: 'Fan 1602 RPM', tooltip: 'Fan 1602 RPM', tone: 'green' },
      { id: 'upload', digits: '999', marker: '↑M', text: '↑999 MB/s', tooltip: 'Upload 999 MB/s', tone: 'normal' },
      { id: 'download', digits: '0', marker: '↓B', text: '↓0 B/s', tooltip: 'Download 0 B/s', tone: 'normal' },
      {
        id: 'gpuTemperature',
        digits: '—',
        marker: '°C',
        text: 'GPU —',
        tooltip: 'GPU unavailable',
        tone: 'normal',
        gpuDevice: { id: 'gpu-1', name: 'Graphics', label: 'GPU 1' },
      },
    ],
  };
}
function render(locale = 'en-US') {
  const wrapper = mount(FloatingMonitor, {
    global: {
      plugins: [createPinia(), createI18n({ legacy: false, locale, messages: locales })],
    },
  });
  wrappers.push(wrapper);
  return wrapper;
}
beforeEach(() => {
  vi.resetAllMocks();
  stop = vi.fn();
  native.ready.mockResolvedValue(snapshot());
  native.layout.mockResolvedValue(true);
  native.onSnapshot.mockImplementation(async callback => {
    accept = callback;
    return stop;
  });
  native.onAppearanceChanged.mockResolvedValue(vi.fn());
  native.drag.mockResolvedValue(undefined);
  native.menu.mockResolvedValue(undefined);
  native.togglePage.mockResolvedValue(undefined);
});
afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount());
});
describe('floating monitor interaction and lifecycle', () => {
  it('keeps frequency, idle and unavailable values in a stable layout', async () => {
    const value = snapshot();
    value.interactive = false;
    value.entries = [
      {
        id: 'cpuSuperFrequency',
        digits: '2.4',
        marker: 'CPUS',
        text: '',
        tooltip: 'CPU S',
        tone: 'normal',
        frequency: { state: 'active', idleLabel: 'Idle' },
      },
    ];
    native.ready.mockResolvedValue(value);
    const wrapper = render();
    await flushPromises();
    await wrapper.get('[data-metric="cpuSuperFrequency"]').trigger('mouseenter');
    await flushPromises();
    const grid = wrapper.get('main').attributes('style');
    accept({ ...value, entries: [{ ...value.entries[0]!, digits: '3.2' }] });
    await flushPromises();
    expect(wrapper.get('main').attributes('style')).toBe(grid);
    accept({
      ...value,
      entries: [{ ...value.entries[0]!, digits: 'Idle', frequency: { state: 'idle', idleLabel: 'Idle' } }],
    });
    await flushPromises();
    expect(wrapper.get('main').attributes('style')).toBe(grid);
    expect(wrapper.get('.metric-value').text()).toBe('Idle');
    expect(native.layout).toHaveBeenCalledOnce();
    await wrapper.get('[data-metric="cpuSuperFrequency"]').trigger('mouseleave');
    accept({
      ...value,
      entries: [{ ...value.entries[0]!, digits: '—', frequency: { state: 'unavailable', idleLabel: 'Idle' } }],
    });
    await flushPromises();
    expect(wrapper.get('.metric-value').text()).toBe('—');
    expect(wrapper.get('main').attributes('style')).toBe(grid);
    expect(native.layout).toHaveBeenCalledOnce();
  });
  it.each(Object.entries(locales))('keeps macOS metrics passive and stable in %s', async (locale, messages) => {
    const value = { ...snapshot(), interactive: false };
    native.ready.mockResolvedValue(value);
    const wrapper = render(locale);
    await flushPromises();
    expect(wrapper.findAll('button')).toHaveLength(0);
    expect(wrapper.get('main').attributes('tabindex')).toBe('0');
    expect(wrapper.get('[data-metric="fanSpeed"] .metric-label').text()).toBe(messages.floatingMonitor.labels.fan);
    const count = native.layout.mock.calls.length;
    const grid = wrapper.get('main').attributes('style');
    await wrapper.get('[data-metric="cpu"]').trigger('click');
    await wrapper.get('main').trigger('contextmenu');
    await wrapper.get('main').trigger('keydown', { key: 'F10', shiftKey: true });
    expect(native.togglePage).not.toHaveBeenCalled();
    expect(native.menu).toHaveBeenCalledTimes(2);
    for (const digits of ['0', '9', '100', '—']) {
      accept({ ...value, entries: value.entries.map(entry => (entry.id === 'cpu' ? { ...entry, digits } : entry)) });
      await flushPromises();
      expect(wrapper.get('main').attributes('style')).toBe(grid);
    }
    expect(native.layout).toHaveBeenCalledTimes(count);
  });
  it('keeps macOS primary dragging separate from right-click actions', async () => {
    native.ready.mockResolvedValue({ ...snapshot(), interactive: false });
    const wrapper = render();
    await flushPromises();
    const surface = wrapper.get('main');
    const metric = wrapper.get('[data-metric="cpu"]');
    await metric.trigger('pointerdown', { button: 2, screenX: 10, screenY: 10 });
    await metric.trigger('pointermove', { buttons: 2, screenX: 30, screenY: 10 });
    await metric.trigger('contextmenu');
    expect(native.menu).toHaveBeenCalledOnce();
    expect(native.drag).not.toHaveBeenCalled();
    Object.defineProperty(surface.element, 'setPointerCapture', { value: vi.fn() });
    await metric.trigger('pointerdown', { button: 0, pointerId: 1, screenX: 10, screenY: 10 });
    await metric.trigger('pointermove', { buttons: 1, screenX: 20, screenY: 10 });
    await metric.trigger('pointerup');
    await metric.trigger('click', { detail: 1 });
    expect(native.drag).toHaveBeenCalledOnce();
    expect(native.menu).toHaveBeenCalledOnce();
    expect(native.togglePage).not.toHaveBeenCalled();
    accept({ ...snapshot(), interactive: false, contextMenu: false });
    await flushPromises();
    await surface.trigger('contextmenu');
    expect(native.menu).toHaveBeenCalledOnce();
  });
  it('shows focus cues only after keyboard navigation and clears them for pointer menus', async () => {
    native.ready.mockResolvedValue({ ...snapshot(), interactive: false });
    const wrapper = render();
    await flushPromises();
    const surface = wrapper.get('main');
    expect(surface.attributes('data-keyboard-focus')).toBe('false');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab' }));
    await flushPromises();
    expect(surface.attributes('data-keyboard-focus')).toBe('true');
    await surface.trigger('keydown', { key: 'F10', shiftKey: true });
    expect(native.menu).toHaveBeenCalledOnce();
    expect(surface.attributes('data-keyboard-focus')).toBe('true');
    await surface.trigger('contextmenu');
    expect(surface.attributes('data-keyboard-focus')).toBe('false');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await flushPromises();
    expect(surface.attributes('data-keyboard-focus')).toBe('false');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab' }));
    await flushPromises();
    await surface.trigger('pointerdown', { button: 2 });
    expect(surface.attributes('data-keyboard-focus')).toBe('false');
    expect(native.togglePage).not.toHaveBeenCalled();
  });
  it('opens only the context menu when macOS has no selected metrics', async () => {
    native.ready.mockResolvedValue({ ...snapshot(), interactive: false, entries: [] });
    const wrapper = render();
    await flushPromises();
    await wrapper.get('.empty-state').trigger('click');
    expect(native.menu).not.toHaveBeenCalled();
    await wrapper.get('.empty-state').trigger('contextmenu');
    expect(native.menu).toHaveBeenCalledOnce();
    expect(native.togglePage).not.toHaveBeenCalled();
  });
  it('completes the last horizontal grid row without adding metric buttons or measurements', async () => {
    const wrapper = render();
    await flushPromises();
    expect(wrapper.findAll('.empty-cell')).toHaveLength(1);
    expect(wrapper.get('.empty-cell').attributes('aria-hidden')).toBe('true');
    expect(wrapper.get('.empty-cell').classes()).toEqual(expect.arrayContaining(['column-divider', 'row-divider']));
    expect(wrapper.findAll('button.metric')).toHaveLength(5);
    expect(native.layout.mock.calls.at(-1)![0]).toHaveLength(5);
    const value = snapshot();
    value.entries = value.entries.slice(0, 4);
    accept(value);
    await flushPromises();
    expect(wrapper.findAll('.empty-cell')).toHaveLength(2);
    value.entries = value.entries.slice(0, 3);
    accept({ ...value });
    await flushPromises();
    expect(wrapper.findAll('.empty-cell')).toHaveLength(0);
    accept(snapshot());
    await flushPromises();
    expect(wrapper.findAll('.empty-cell')).toHaveLength(1);
    accept({ ...snapshot(), preferences: { ...snapshot().preferences, layout: 'vertical' } });
    await flushPromises();
    expect(wrapper.findAll('.empty-cell')).toHaveLength(0);
  });
  it('updates dividers in both layouts without measuring or adding interactive cells', async () => {
    const wrapper = render();
    await flushPromises();
    const count = native.layout.mock.calls.length;
    const value = snapshot();
    for (const layout of ['horizontal', 'vertical'] as const) {
      for (const dividers of [false, true]) {
        accept({ ...value, preferences: { ...value.preferences, layout, dividers } });
        await flushPromises();
        expect(wrapper.get('main').attributes('data-dividers')).toBe(String(dividers));
        expect(wrapper.findAll('button.metric')).toHaveLength(value.entries.length);
        if (layout === 'vertical') {
          expect(wrapper.findAll('button.row-divider')).toHaveLength(value.entries.length - 1);
          expect(wrapper.findAll('.empty-cell')).toHaveLength(0);
        }
      }
    }
    expect(native.layout).toHaveBeenCalledTimes(count);
  });
  it.each(Object.entries(locales))(
    'renders translated metrics with zero and unavailable values in %s',
    async (locale, messages) => {
      const wrapper = render(locale);
      await flushPromises();
      expect(wrapper.get('[data-metric="cpu"] .metric-label').text()).toBe(messages.floatingMonitor.labels.cpu);
      expect(wrapper.get('[data-metric="download"] .metric-value').text()).toBe('0B/s');
      expect(wrapper.get('[data-metric="upload"] .metric-label').text()).toBe(messages.floatingMonitor.labels.upload);
      expect(wrapper.get('[data-metric="upload"] .metric-value').text()).toBe('999MB/s');
      expect(wrapper.get('[data-metric="fanSpeed"] .metric-label').text()).toBe(
        messages.floatingMonitor.labels.fanSpeed
      );
      expect(wrapper.get('[data-metric="fanSpeed"] .metric-value').text()).toBe('1602');
      expect(wrapper.find('[data-metric="fanSpeed"] small').exists()).toBe(false);
      expect(wrapper.get('[data-metric="download"] .metric-label').text()).toBe(
        messages.floatingMonitor.labels.download
      );
      const vertical = snapshot();
      vertical.preferences.layout = 'vertical';
      accept(vertical);
      await flushPromises();
      expect(wrapper.get('[data-metric="download"] .metric-label').text()).toBe(
        messages.floatingMonitor.labels.download
      );
      expect(wrapper.get('[data-metric="gpuTemperature"] .metric-value').text()).toBe('—');
      expect(wrapper.text()).not.toMatch(/floatingMonitor\.|systemStatus\.|monitoring\./u);
    }
  );
  it('remeasures font and weight changes but keeps geometry fixed for palette and opacity changes', async () => {
    const wrapper = render();
    await flushPromises();
    const value = snapshot();
    value.preferences.fontSize = 'extraLarge';
    value.preferences.bold = true;
    accept(value);
    await flushPromises();
    expect(native.layout).toHaveBeenCalledTimes(2);
    expect(native.layout.mock.calls.at(-1)![1]).toEqual({ fontSize: 'extraLarge', bold: true });
    expect(wrapper.get('main').attributes('style')).toContain('--floating-font-scale: 1.5');
    expect(wrapper.get('main').attributes('data-bold')).toBe('true');
    accept({ ...value, preferences: { ...value.preferences, background: 'black', backgroundOpacity: 50 } });
    await flushPromises();
    expect(wrapper.get('main').attributes('data-background')).toBe('black');
    expect(native.layout).toHaveBeenCalledTimes(2);
  });
  it('keeps native columns and digit/unit slots fixed when only readings change', async () => {
    const wrapper = render();
    await flushPromises();
    expect(native.layout).toHaveBeenCalledOnce();
    const metric = wrapper.get('[data-metric="upload"]');
    const style = metric.attributes('style');
    const grid = wrapper.get('main').attributes('style');
    const value = snapshot();
    value.entries.find(entry => entry.id === 'upload')!.digits = '9';
    value.entries.find(entry => entry.id === 'upload')!.marker = '↑K';
    accept(value);
    await flushPromises();
    expect(metric.attributes('style')).toBe(style);
    expect(wrapper.get('main').attributes('style')).toBe(grid);
    expect(native.layout).toHaveBeenCalledOnce();
    expect(metric.get('.metric-value').text()).toBe('9KB/s');
    expect(metric.find('.metric-unit').exists()).toBe(false);
    expect(metric.find('.metric-digits').exists()).toBe(false);
  });
  it('keeps fan columns fixed at the four-digit boundary and retains the exact accessible reading', async () => {
    const wrapper = render();
    await flushPromises();
    const metric = wrapper.get('[data-metric="fanSpeed"]');
    const style = metric.attributes('style');
    for (const [digits, expected] of [
      ['9999', '9999'],
      ['12345', '12.3k'],
      ['30000', '30k'],
    ]) {
      const value = snapshot();
      const fan = value.entries.find(entry => entry.id === 'fanSpeed')!;
      fan.digits = digits!;
      fan.tooltip = `GPU fan ${digits} RPM`;
      accept(value);
      await flushPromises();
      expect(metric.get('.metric-value').text()).toBe(expected);
      expect(metric.attributes('aria-label')).toBe(fan.tooltip);
      expect(metric.attributes('style')).toBe(style);
      expect(native.layout).toHaveBeenCalledOnce();
    }
  });
  it('serializes sizing updates so a changed selection follows the pending measurement', async () => {
    let finish: (accepted: boolean) => void = () => {};
    native.layout.mockReturnValueOnce(
      new Promise<boolean>(resolve => {
        finish = resolve;
      })
    );
    const wrapper = render();
    await flushPromises();
    const value = snapshot();
    value.entries = value.entries.filter(entry => entry.id !== 'fanSpeed');
    accept(value);
    await flushPromises();
    expect(native.layout).toHaveBeenCalledOnce();
    finish(true);
    await flushPromises();
    expect(native.layout).toHaveBeenCalledTimes(2);
    expect(native.layout.mock.calls[1]![0]).toHaveLength(value.entries.length);
    expect(wrapper.find('[data-metric="fanSpeed"]').exists()).toBe(false);
  });
  it('separates metric clicks from whole-surface dragging and preserves locked interactions', async () => {
    const wrapper = render();
    await flushPromises();
    const surface = wrapper.get('main');
    const metric = wrapper.get('[data-metric="gpuTemperature"]');
    surface.element.setPointerCapture = vi.fn();
    metric.element.setPointerCapture = vi.fn();
    await metric.trigger('pointerdown', { button: 0, pointerId: 1, screenX: 10, screenY: 10 });
    await metric.trigger('pointermove', { buttons: 1, screenX: 12, screenY: 10 });
    await metric.trigger('pointerup');
    await metric.trigger('click', { detail: 1 });
    expect(native.togglePage).toHaveBeenCalledWith('gpu');
    expect(native.drag).not.toHaveBeenCalled();
    await metric.trigger('pointerdown', { button: 0, pointerId: 1, screenX: 10, screenY: 10 });
    await metric.trigger('pointermove', { buttons: 1, screenX: 16, screenY: 10 });
    await metric.trigger('pointermove', { buttons: 1, screenX: 20, screenY: 10 });
    await metric.trigger('pointerup');
    await metric.trigger('click', { detail: 1 });
    expect(native.drag).toHaveBeenCalledOnce();
    expect(native.togglePage).toHaveBeenCalledOnce();
    await surface.trigger('pointerdown', { button: 0, pointerId: 1, screenX: 20, screenY: 20 });
    await surface.trigger('pointermove', { buttons: 0, screenX: 30, screenY: 20 });
    expect(native.drag).toHaveBeenCalledOnce();
    const value = snapshot();
    value.preferences.locked = true;
    accept(value);
    await flushPromises();
    await surface.trigger('pointerdown', { button: 0, screenX: 10, screenY: 10 });
    await surface.trigger('pointermove', { buttons: 1, screenX: 20, screenY: 10 });
    expect(native.drag).toHaveBeenCalledOnce();
    await wrapper.get('[data-metric="download"]').trigger('click');
    expect(native.togglePage).toHaveBeenLastCalledWith('network');
    await surface.trigger('contextmenu');
    await surface.trigger('keydown', { key: 'F10', shiftKey: true });
    expect(native.menu).toHaveBeenCalledTimes(2);
  });
  it('updates background opacity without fading metrics or replacing their hit targets', async () => {
    const wrapper = render();
    await flushPromises();
    const metric = wrapper.get('[data-metric="cpu"]').element;
    const value = snapshot();
    value.preferences.backgroundOpacity = 60;
    accept(value);
    await flushPromises();
    expect(wrapper.get('main').attributes('style')).toContain('--floating-background-opacity: 60%');
    expect(wrapper.get('[data-metric="cpu"]').element).toBe(metric);
    await wrapper.get('[data-metric="cpu"]').trigger('click');
    expect(native.togglePage).toHaveBeenCalledWith('cpu');
  });
  it('keeps device labels distinct in a compact multi-GPU group', async () => {
    const value = snapshot();
    const gpu = value.entries.find(entry => entry.id === 'gpuTemperature')!;
    value.entries.push({ ...gpu, gpuDevice: { id: 'gpu-2', name: 'Another graphics card', label: 'GPU 2' } });
    native.ready.mockResolvedValue(value);
    const wrapper = render();
    await flushPromises();
    expect(wrapper.findAll('[data-metric="gpuTemperature"] .metric-label').map(label => label.text())).toEqual([
      'GPU 1',
      'GPU 2',
    ]);
  });
  it.each(Object.keys(locales))('identifies a single hybrid GPU power source in %s', async locale => {
    const value = snapshot();
    value.entries.push({
      id: 'gpuPower',
      digits: '0.4',
      marker: 'W',
      text: 'GPU power 0.4W',
      tooltip: 'Intel Graphics · GPU power 0.4W',
      tone: 'normal',
      gpuDevice: { id: 'intel', name: 'Intel Graphics', label: 'GPU1' },
    });
    native.ready.mockResolvedValue(value);
    const wrapper = render(locale);
    await flushPromises();
    expect(wrapper.get('[data-metric="gpuPower"] .metric-label').text()).toContain('GPU1');
    accept({
      ...value,
      entries: value.entries.map(entry =>
        entry.id === 'gpuPower' ? { ...entry, gpuDevice: { ...entry.gpuDevice!, label: 'GPU' } } : entry
      ),
    });
    await flushPromises();
    expect(wrapper.get('[data-metric="gpuPower"] .metric-label').text()).toBe(
      locales[locale]!.floatingMonitor.labels.gpuPower
    );
  });
  it('keeps the latest native event when an older initial read finishes later', async () => {
    let resolve: (value: FloatingSnapshot) => void = () => {};
    native.ready.mockReturnValue(
      new Promise<FloatingSnapshot>(done => {
        resolve = done;
      })
    );
    const wrapper = render();
    await flushPromises();
    const latest = snapshot();
    latest.entries[0]!.digits = '83';
    accept(latest);
    resolve(snapshot());
    await flushPromises();
    expect(wrapper.get('[data-metric="cpu"] .metric-value').text()).toBe('83%');
    wrapper.unmount();
    expect(stop).toHaveBeenCalledOnce();
    wrappers.pop();
  });
  it('does not activate fallback sizing when a request fails after disposal', async () => {
    let reject: (reason: Error) => void = () => {};
    native.layout.mockReturnValueOnce(
      new Promise<boolean>((_done, fail) => {
        reject = fail;
      })
    );
    const wrapper = render();
    await flushPromises();
    expect(native.layout).toHaveBeenCalledOnce();
    wrapper.unmount();
    wrappers.pop();
    reject(new Error('Window closed'));
    await flushPromises();
    expect(native.layout).toHaveBeenCalledOnce();
  });
  it('releases a subscription that resolves after the window is disposed', async () => {
    let resolve: (stop: () => void) => void = () => {};
    native.onSnapshot.mockReturnValue(
      new Promise<() => void>(done => {
        resolve = done;
      })
    );
    const wrapper = render();
    wrapper.unmount();
    wrappers.pop();
    resolve(stop);
    await flushPromises();
    expect(stop).toHaveBeenCalledOnce();
    expect(native.ready).not.toHaveBeenCalled();
  });
});
