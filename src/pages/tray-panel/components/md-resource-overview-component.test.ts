// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Overview from './md-resource-overview.vue';
import { emptyReadings } from '@/lib/utils/system-resources/system-resources';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import ja from '@/locales/ja-JP.json';
import ko from '@/locales/ko-KR.json';
import pt from '@/locales/pt-BR.json';
import ru from '@/locales/ru-RU.json';
import tr from '@/locales/tr-TR.json';
import uk from '@/locales/uk-UA.json';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
vi.mock('@/lib/services/settings/byte-size-service', () => ({
  ByteSizeService: {
    bytes: (value: number) => `${value} B`,
    diskCapacity: (value: number) => `${value} B`,
    memory: (value: number) => `${value} B`,
  },
}));

describe('resource details', () => {
  beforeEach(() => vi.spyOn(OperatingSystemService, 'currentPlatform').mockReturnValue('macos'));
  afterEach(() => vi.restoreAllMocks());

  it.each([en, zh, tw, ja, ko, pt, ru, tr, uk])(
    'shows real GPU activity and device identity in each locale',
    messages => {
      const reading = emptyReadings();
      reading.observedAtMs = 1000;
      reading.observedAtMs = 1000;
      reading.gpu = {
        status: 'ready',
        sampledAtMs: 1000,
        value: { usedPercent: 0, adapterId: 'gpu-1', adapterName: 'NVIDIA GeForce RTX 4090', details: null },
      };
      reading.gpuHistory = [{ sampledAtMs: 1000, primary: 0, secondary: null }];
      const wrapper = mount(Overview, {
        props: { metric: 'gpu', reading },
        global: { plugins: [createI18n({ legacy: false, locale: 'test', messages: { test: messages } })] },
      });
      expect(wrapper.get('.resource-value').text()).toBe('0%');
      expect(wrapper.get('.gpu-source').text()).toBe('NVIDIA GeForce RTX 4090');
      expect(wrapper.getComponent(MdTooltip).props('text')).toBe(messages.systemStatus.gpuUsageHint);
      expect(wrapper.find('.resource-trend.gpu').exists()).toBe(true);
      wrapper.unmount();
    }
  );

  it.each(['macos', 'windows'] as const)('shows identified GPU temperature in the %s overview', async platform => {
    vi.spyOn(OperatingSystemService, 'currentPlatform').mockReturnValue(platform);
    const reading = emptyReadings();
    reading.observedAtMs = 1000;
    reading.observedAtMs = 2000;
    reading.gpu = {
      status: 'ready',
      sampledAtMs: 2000,
      value: { usedPercent: 0, adapterId: 'gpu-1', adapterName: 'GPU', details: null },
    };
    reading.gpuTemperature = {
      status: 'ready',
      sampledAtMs: 2000,
      value: {
        adapterId: 'gpu-1',
        adapterName: 'GPU',
        celsius: 42.5,
        kind: 'device',
        source: 'driver',
        sensorCount: 1,
      },
    };
    const wrapper = mount(Overview, {
      props: { metric: 'gpu', reading, interactive: true },
      global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
    });
    expect(wrapper.get('.temperature-value').text()).toBe('43°C');
    await wrapper.get('.temperature-value').trigger('click');
    expect(wrapper.emitted('gpu')).toHaveLength(1);
    await wrapper.setProps({
      reading: { ...reading, gpuTemperature: { status: 'unsupported', sampledAtMs: null, value: null } },
    });
    expect(wrapper.find('.temperature-value').exists()).toBe(false);
    wrapper.unmount();
  });

  it('opens CPU details when the temperature target is clicked', async () => {
    const reading = emptyReadings();
    reading.observedAtMs = 1000;
    reading.observedAtMs = 1000;
    reading.cpuTemperature = {
      status: 'ready',
      sampledAtMs: 1000,
      value: { celsius: 48, kind: 'coreAverage', source: 'appleSmc', sensorCount: 16 },
    };
    const wrapper = mount(Overview, {
      props: { metric: 'cpu', reading, interactive: true },
      global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
    });
    await wrapper.get('.temperature-value').trigger('click');
    expect(wrapper.emitted('cpu')).toHaveLength(1);
    await wrapper.setProps({ showCpuTemperature: false });
    expect(wrapper.find('.cpu-temperature').exists()).toBe(false);
    wrapper.unmount();
  });

  it.each(['loading', 'unsupported', 'failed', 'stale'] as const)(
    'shows retained %s GPU data only with an explicit status',
    status => {
      const reading = emptyReadings();
      reading.observedAtMs = 1000;
      reading.gpu = {
        status,
        sampledAtMs: 1000,
        value: { usedPercent: 0, adapterId: 'a', adapterName: 'GPU A', details: null },
      };
      const wrapper = mount(Overview, {
        props: { metric: 'gpu', reading },
        global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
      });
      expect(wrapper.get('.resource-value').text()).toBe(status === 'failed' || status === 'stale' ? '0%' : '—');
      expect(wrapper.find('[role="status"]').exists()).toBe(status !== 'stale');
      wrapper.unmount();
    }
  );

  it.each(['loading', 'unsupported', 'failed', 'stale'] as const)(
    'keeps the %s CPU status visible alongside its known model',
    status => {
      const reading = emptyReadings();
      reading.observedAtMs = 1000;
      reading.cpuIdentity = { model: 'Intel Core i9 CPU @ 3.70GHz', frequencyClasses: [], nominalFrequencyMhz: 3700 };
      reading.cpu = { status, sampledAtMs: 1000, value: { usedPercent: 45 } };
      const wrapper = mount(Overview, {
        props: { metric: 'cpu', reading },
        global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
      });
      expect(wrapper.get('.resource-value').text()).toBe(status === 'failed' || status === 'stale' ? '45%' : '—');
      expect(wrapper.get('.gpu-source').text()).toBe(reading.cpuIdentity.model);
      expect(wrapper.find('[role="status"]').exists()).toBe(status !== 'stale');
      wrapper.unmount();
    }
  );

  it.each(['stale', 'failed'] as const)(
    'silently refreshes a %s observation while preserving genuine errors',
    async status => {
      const reading = emptyReadings();
      reading.observedAtMs = 2000;
      reading.cpu = { status, sampledAtMs: 1000, value: { usedPercent: 45 } };
      reading.cpuIdentity = { model: 'Apple M3 Max', frequencyClasses: [], nominalFrequencyMhz: null };
      const wrapper = mount(Overview, {
        props: { metric: 'cpu', reading, interactive: true },
        global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
      });
      const state = en.systemStatus[status];
      if (status === 'failed') {
        expect(wrapper.get('.reading-error').text()).toBe(state);
        expect(wrapper.get('.reading-error').attributes('aria-label')).toBe(state);
        expect(wrapper.findAllComponents(MdTooltip).some(tooltip => tooltip.props('text') === state)).toBe(true);
      } else {
        expect(wrapper.find('.reading-error').exists()).toBe(false);
      }
      expect(wrapper.get('.gpu-source').text()).toBe('Apple M3 Max');
      expect(wrapper.get('.resource-value').text()).toBe('45%');
      await wrapper.setProps({ reading: { ...reading, cpu: { ...reading.cpu, status: 'ready', sampledAtMs: 2000 } } });
      expect(wrapper.find('.reading-error').exists()).toBe(false);
      expect(wrapper.get('.resource-value').text()).toBe('45%');
      wrapper.unmount();
    }
  );

  it.each(['macos', 'linux', 'windows'] as const)('limits the reclaimable-space hint to macOS on %s', platform => {
    vi.spyOn(OperatingSystemService, 'currentPlatform').mockReturnValue(platform);
    const reading = emptyReadings();
    reading.observedAtMs = 1000;
    reading.disk = {
      status: 'ready',
      sampledAtMs: 0,
      value: {
        volume: { id: 'system', name: '/', system: true, fileSystem: null, mountPoints: [], devices: [] },
        totalBytes: 100,
        availableBytes: 75,
        usedBytes: 25,
        usedPercent: 25,
      },
    };
    const wrapper = mount(Overview, {
      props: { metric: 'disk', reading },
      global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
    });
    expect(wrapper.get('.resource-meta').text()).toContain('75 B / 100 B');
    expect(wrapper.getComponent(MdTooltip).props('text')).toBe(
      platform === 'macos' ? en.systemStatus.diskCapacityHint : null
    );
    wrapper.unmount();
  });

  it.each(['cpu', 'gpu', 'memory'] as const)(
    'opens %s details from the whole card only when interactive',
    async metric => {
      const wrapper = mount(Overview, {
        props: { metric, reading: emptyReadings(), interactive: true },
        global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
      });
      await wrapper.get('.card-navigation').trigger('click');
      expect(wrapper.emitted(metric)).toHaveLength(1);
      expect(wrapper.emitted(metric === 'cpu' ? 'memory' : 'cpu')).toBeUndefined();
      await wrapper.setProps({ interactive: false });
      expect(wrapper.find('.card-navigation').exists()).toBe(false);
      wrapper.unmount();
    }
  );

  it.each([en, zh, tw, ja, ko, pt, ru, tr, uk])(
    'shows memory pressure separately from capacity in each locale',
    messages => {
      for (const pressure of ['normal', 'warning', 'critical', 'unavailable'] as const) {
        const reading = emptyReadings();
        reading.observedAtMs = 1000;
        reading.memory = {
          status: 'ready',
          sampledAtMs: 1000,
          value: {
            schemaVersion: 4,
            sampledAtMs: 1000,
            memory: { totalBytes: 100, usedBytes: 88, freeBytes: 12, swapUsedBytes: 0, usedPercent: 88, pressure },
            processes: null,
          },
        };
        const wrapper = mount(Overview, {
          props: { metric: 'memory', reading },
          global: { plugins: [createI18n({ legacy: false, locale: 'test', messages: { test: messages } })] },
        });
        expect(wrapper.get('.resource-value').text()).toBe('88%');
        expect(wrapper.get('.resource-meta').text()).toContain('88 B / 100 B');
        expect(wrapper.get('[role="meter"]').attributes('aria-valuenow')).toBe('88');
        expect(wrapper.get('.overview-pressure .memory-pressure').attributes('data-pressure')).toBe(pressure);
        expect(wrapper.get('.memory-pressure').text()).toBe(messages.monitoring.pressure[pressure]);
        expect(wrapper.getComponent(MdTooltip).props('text')).toBe(messages.monitoring.pressure[`${pressure}Hint`]);
        wrapper.unmount();
      }
    }
  );

  it('keeps known pressure while refreshing and marks failed observations', async () => {
    const reading = emptyReadings();
    reading.observedAtMs = 1000;
    const wrapper = mount(Overview, {
      props: { metric: 'memory', reading },
      global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
    });
    expect(wrapper.find('.overview-pressure').exists()).toBe(false);
    reading.memory = {
      status: 'ready',
      sampledAtMs: 1000,
      value: {
        schemaVersion: 4,
        sampledAtMs: 1000,
        memory: {
          totalBytes: 100,
          usedBytes: 88,
          freeBytes: 12,
          swapUsedBytes: 0,
          usedPercent: 88,
          pressure: 'unsupported',
        },
        processes: null,
      },
    };
    await wrapper.setProps({ reading: { ...reading } });
    expect(wrapper.find('.overview-pressure').exists()).toBe(false);
    reading.memory.value!.memory.pressure = 'normal';
    for (const status of ['loading', 'stale', 'disconnected', 'failed', 'unsupported'] as const) {
      await wrapper.setProps({ reading: { ...reading, memory: { ...reading.memory, status } } });
      const expected = status === 'loading' || status === 'stale' ? 'normal' : 'stale';
      expect(wrapper.get('.memory-pressure').attributes('data-pressure')).toBe(expected);
      expect(wrapper.get('.memory-pressure').text()).toBe(en.monitoring.pressure[expected]);
      expect(wrapper.get('.resource-value').text()).toBe(status === 'stale' || status === 'failed' ? '88%' : '—');
    }
    await wrapper.setProps({ reading: { ...reading } });
    expect(wrapper.get('.memory-pressure').attributes('data-pressure')).toBe('normal');
    expect(wrapper.get('.resource-value').text()).toBe('88%');
    wrapper.unmount();
  });

  it('explains overview pressure on hover and opens memory details from the pressure label', async () => {
    const reading = emptyReadings();
    reading.observedAtMs = 1000;
    reading.memory = {
      status: 'ready',
      sampledAtMs: 1000,
      value: {
        schemaVersion: 4,
        sampledAtMs: 1000,
        memory: {
          totalBytes: 100,
          usedBytes: 88,
          freeBytes: 12,
          swapUsedBytes: 0,
          usedPercent: 88,
          pressure: 'normal',
        },
        processes: null,
      },
    };
    const wrapper = mount(Overview, {
      props: { metric: 'memory', reading, interactive: true },
      attachTo: document.body,
      global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
    });
    try {
      await wrapper.get('.memory-pressure').trigger('pointermove', { pointerType: 'mouse' });
      await vi.waitFor(() =>
        expect(document.querySelector('[role="tooltip"]')?.textContent).toContain(en.monitoring.pressure.normalHint)
      );
      await wrapper.setProps({ reading: { ...reading, memory: { ...reading.memory, status: 'stale' } } });
      expect(document.querySelector('[role="tooltip"]')?.textContent).toContain(en.monitoring.pressure.normalHint);
      await wrapper.get('.memory-pressure').trigger('click');
      expect(wrapper.emitted('memory')).toHaveLength(1);
      await wrapper.setProps({ interactive: false });
      await wrapper.get('.memory-pressure').trigger('click');
      expect(wrapper.emitted('memory')).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });

  it('leaves the disconnected interval blank instead of moving old samples to now', () => {
    const reading = emptyReadings();
    reading.observedAtMs = 1000;
    reading.observedAtMs = 90000;
    reading.cpu.status = 'stale';
    reading.cpuHistory = [{ sampledAtMs: 60000, primary: 40, secondary: null }];
    const wrapper = mount(Overview, {
      props: { metric: 'cpu', reading },
      global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en } })] },
    });
    expect(wrapper.get('.resource-value').text()).toBe('—');
    expect(wrapper.get('path[stroke]').attributes('d')).toMatch(/^M50,/);

    wrapper.unmount();
  });

  it.each([en, zh, tw, ja, ko])(
    'localizes the disk action and emits navigation without performing cleanup',
    async messages => {
      const reading = emptyReadings();
      reading.observedAtMs = 1000;
      reading.disk = {
        status: 'ready',
        sampledAtMs: 0,
        value: {
          volume: { id: 'test', name: 'Test', system: false, fileSystem: null, mountPoints: [], devices: [] },
          totalBytes: 100,
          availableBytes: 75,
          usedBytes: 25,
          usedPercent: 25,
        },
      };
      const wrapper = mount(Overview, {
        props: { metric: 'disk', reading },
        global: { plugins: [createI18n({ legacy: false, locale: 'test', messages: { test: messages } })] },
      });
      expect(wrapper.get('.cleanup-link').text()).toContain(messages.navigation.cleanup);
      expect(wrapper.find('.cleanup-button').exists()).toBe(false);
      await wrapper.setProps({ cleanupVariant: 'button' });
      expect(wrapper.find('.cleanup-button').exists()).toBe(true);
      expect(wrapper.get('.resource-meta').text()).toContain('75 B / 100 B');
      expect(wrapper.get('[role="meter"]').attributes('aria-valuenow')).toBe('25');
      await wrapper.get('.cleanup-link').trigger('click');
      expect(wrapper.emitted('cleanup')).toHaveLength(1);
      await wrapper.setProps({ reading: { ...reading, disk: { ...reading.disk, status: 'disconnected' } } });
      expect(wrapper.get('.resource-value').text()).toBe('—');
      expect(wrapper.get('.resource-source').text()).toBe('Test');
      expect(wrapper.find('[role="meter"]').exists()).toBe(false);
      wrapper.unmount();
    }
  );
});
