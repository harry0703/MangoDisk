// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Select } from '@/components/ui/select';
import Monitor from './md-taskbar-monitor.vue';
import { TaskbarMonitorService } from '@/lib/services/resident/taskbar-monitor-service';

vi.mock('@/lib/services/resident/taskbar-monitor-service', () => ({
  TaskbarMonitorService: { catalogue: vi.fn(), identify: vi.fn(), openSettings: vi.fn() },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
const screens = [
  { id: 'built-in', name: 'Internal', number: 1, primary: true, hasTaskbar: true, width: 1920, height: 1080 },
  { id: 'external', name: 'External', number: 2, primary: false, hasTaskbar: true, width: 2560, height: 1440 },
];
function render(value: string | null) {
  return mount(Monitor, {
    props: { value },
    global: {
      plugins: [
        createI18n({ legacy: false, locale: 'en', missingWarn: false, fallbackWarn: false, messages: { en: {} } }),
      ],
    },
  });
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(TaskbarMonitorService.catalogue).mockResolvedValue({ schemaVersion: 1, monitors: screens });
});
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});
describe('taskbar monitor selection', () => {
  it('emits a stable identity and a primary-display policy independently of numbering', async () => {
    const wrapper = render(null);
    await flushPromises();
    wrapper.findComponent(Select).vm.$emit('update:modelValue', 'external');
    wrapper.findComponent(Select).vm.$emit('update:modelValue', 'primary');
    wrapper.findComponent(Select).vm.$emit('update:modelValue', 'unknown');
    expect(wrapper.emitted('change')).toEqual([['external'], [null]]);
    wrapper.unmount();
  });
  it('preserves the saved identity through disconnect, reconnect and a missing taskbar', async () => {
    const wrapper = render('external');
    await flushPromises();
    expect(wrapper.text()).not.toContain('systemStatus.taskbarMonitorFallback');
    vi.mocked(TaskbarMonitorService.catalogue).mockResolvedValue({ schemaVersion: 1, monitors: screens.slice(0, 1) });
    await vi.advanceTimersByTimeAsync(3000);
    expect(wrapper.text()).toContain('systemStatus.taskbarMonitorFallback');
    expect(wrapper.emitted('change')).toBeUndefined();
    vi.mocked(TaskbarMonitorService.catalogue).mockResolvedValue({
      schemaVersion: 1,
      monitors: screens.map(screen => ({ ...screen, hasTaskbar: screen.primary })),
    });
    await vi.advanceTimersByTimeAsync(3000);
    expect(wrapper.text()).toContain('systemStatus.taskbarMonitorNoTaskbar');
    const open = wrapper.findAll('button').find(button => button.text() === 'systemStatus.taskbarMonitorSettings');
    await open!.trigger('click');
    expect(TaskbarMonitorService.openSettings).toHaveBeenCalledOnce();
    vi.mocked(TaskbarMonitorService.catalogue).mockResolvedValue({
      schemaVersion: 1,
      monitors: [...screens].reverse(),
    });
    await vi.advanceTimersByTimeAsync(3000);
    expect(wrapper.text()).not.toContain('systemStatus.taskbarMonitorNoTaskbar');
    expect(wrapper.emitted('change')).toBeUndefined();
    wrapper.unmount();
    const calls = vi.mocked(TaskbarMonitorService.catalogue).mock.calls.length;
    await vi.advanceTimersByTimeAsync(6000);
    expect(TaskbarMonitorService.catalogue).toHaveBeenCalledTimes(calls);
  });
  it('recovers discovery failures and identifies screens without changing preferences', async () => {
    vi.mocked(TaskbarMonitorService.catalogue).mockRejectedValueOnce(new Error('offline'));
    const wrapper = render(null);
    await flushPromises();
    expect(wrapper.text()).toContain('systemStatus.taskbarMonitorFailed');
    await vi.advanceTimersByTimeAsync(3000);
    expect(wrapper.text()).not.toContain('systemStatus.taskbarMonitorFailed');
    const identify = wrapper.findAll('button').find(button => button.text() === 'systemStatus.taskbarMonitorIdentify');
    await identify!.trigger('click');
    expect(TaskbarMonitorService.identify).toHaveBeenCalledOnce();
    expect(wrapper.emitted('change')).toBeUndefined();
    wrapper.unmount();
  });
  it('keeps identification disabled for the full duration after retrying a failure', async () => {
    vi.mocked(TaskbarMonitorService.identify)
      .mockRejectedValueOnce(new Error('enumeration unavailable'))
      .mockResolvedValue(undefined);
    const wrapper = render(null);
    try {
      await flushPromises();
      const identify = wrapper
        .findAll('button')
        .find(button => button.text() === 'systemStatus.taskbarMonitorIdentify')!;
      await identify.trigger('click');
      await flushPromises();
      await vi.advanceTimersByTimeAsync(1000);
      const retry = wrapper.findAll('button').find(button => button.text() === 'monitoring.refresh')!;
      await retry.trigger('click');
      await flushPromises();
      expect(TaskbarMonitorService.identify).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(2000);
      expect(identify.attributes('disabled')).toBeDefined();
      await vi.advanceTimersByTimeAsync(1000);
      expect(identify.attributes('disabled')).toBeUndefined();
      expect(wrapper.emitted('change')).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });
});
