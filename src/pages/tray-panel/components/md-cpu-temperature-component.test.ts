// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { describe, expect, it, vi } from 'vitest';
import MdCpuTemperature from './md-cpu-temperature.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import enUS from '@/locales/en-US.json';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';
import ja from '@/locales/ja-JP.json';
import ko from '@/locales/ko-KR.json';
import pt from '@/locales/pt-BR.json';
import ru from '@/locales/ru-RU.json';
import tr from '@/locales/tr-TR.json';
import uk from '@/locales/uk-UA.json';
import type { CpuTemperature, MetricReading } from '@/lib/models/system-resources/system-resources';
const reading = (): MetricReading<CpuTemperature> => ({
  status: 'ready',
  sampledAtMs: 1000,
  value: { celsius: 48.5, kind: 'coreAverage', source: 'appleSmc', sensorCount: 16 },
});
const render = (compact = false) =>
  mount(MdCpuTemperature, {
    props: { compact, reading: reading(), observedAtMs: 1000 },
    attachTo: document.body,
    global: { plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': enUS } })] },
  });
describe('CPU temperature presentation', () => {
  it.each([enUS, zhCN, zhTW, ja, ko, pt, ru, tr, uk])(
    'distinguishes Intel fallback sensor meanings',
    async messages => {
      const wrapper = mount(MdCpuTemperature, {
        props: { reading: reading(), observedAtMs: 1000 },
        global: { plugins: [createI18n({ legacy: false, locale: 'test', messages: { test: messages } })] },
      });
      try {
        for (const kind of ['diode', 'proximity'] as const) {
          await wrapper.setProps({ reading: { ...reading(), value: { ...reading().value!, kind, sensorCount: 1 } } });
          expect(wrapper.getComponent(MdTooltip).props('text')).toBe(messages.cpuTemperature[`${kind}Hint`]);
          expect(wrapper.get('strong').text()).toBe('49°C');
        }
      } finally {
        wrapper.unmount();
      }
    }
  );
  it('describes AMD die temperature without the Intel package sensor hint', async () => {
    const wrapper = render();
    try {
      await wrapper.setProps({
        reading: {
          ...reading(),
          value: { celsius: 65, kind: 'package', source: 'amdZenPawnIo', sensorCount: 1 },
        },
      });
      expect(wrapper.getComponent(MdTooltip).props('text')).toBe(enUS.cpuTemperature.amdZenHint);
      expect(wrapper.get('strong').text()).toBe('65°C');
    } finally {
      wrapper.unmount();
    }
  });
  it.each([48.5, 49.5, 50.5, 148.5])('rounds %s degrees consistently in overview and details', async celsius => {
    for (const compact of [false, true]) {
      const wrapper = render(compact);
      try {
        await wrapper.setProps({ reading: { ...reading(), value: { ...reading().value!, celsius } } });
        expect(wrapper.get('strong').text()).toBe(`${Math.round(celsius)}°C`);
      } finally {
        wrapper.unmount();
      }
    }
  });
  it.each([enUS, zhCN, zhTW, ja, ko, pt, ru, tr, uk])(
    'localizes the sensor hint without an inline caption',
    messages => {
      const wrapper = mount(MdCpuTemperature, {
        props: { reading: reading(), observedAtMs: 1000 },
        global: { plugins: [createI18n({ legacy: false, locale: 'test', messages: { test: messages } })] },
      });
      expect(wrapper.get('.temperature-help').attributes('aria-label')).toBe(messages.cpuTemperature.label);
      expect(wrapper.get('.temperature-help').text()).toBe('');
      expect(wrapper.get('.temperature-heading').text()).toContain(messages.cpuTemperature.label);
      expect(wrapper.getComponent(MdTooltip).props('text')).toBe(
        messages.cpuTemperature.coreAverageHint.replace('{count}', '16')
      );
      expect(wrapper.find('.temperature-scope').exists()).toBe(false);
      wrapper.unmount();
    }
  );
  it('keeps the overview value compact with an accessible CPU label', () => {
    const wrapper = render(true);
    expect(wrapper.find('.temperature-value').text()).toBe('49°C');
    expect(wrapper.find('.temperature-value').attributes('aria-label')).toBe('CPU temperature 49°C');
    wrapper.unmount();
  });
  it('keeps sensor meaning and numerical values stable while refreshing', async () => {
    const wrapper = render();
    try {
      expect(wrapper.text()).toContain('49°C');
      expect(wrapper.text()).not.toContain('16 readable CPU core sensors');
      expect(wrapper.get('.temperature-help').element.tagName).toBe('BUTTON');
      await wrapper.get('.temperature-help').trigger('pointermove', { pointerType: 'mouse' });
      await vi.waitFor(() =>
        expect(document.querySelector('[role="tooltip"]')?.textContent).toContain('16 readable CPU core sensors')
      );
      await wrapper.setProps({ observedAtMs: 11_001 });
      expect(wrapper.get('.temperature-heading').text()).toContain('49°C');
      expect(wrapper.get('.temperature-heading').text()).toBe('CPU temperature49°C');
      expect(document.querySelector('[role="tooltip"]')?.textContent).toContain('16 readable CPU core sensors');
    } finally {
      wrapper.unmount();
    }
  });
  it('hides unsupported overview data but explains the limitation in details', async () => {
    const wrapper = render(true);
    await wrapper.setProps({ reading: { status: 'unsupported', sampledAtMs: null, value: null } });
    expect(wrapper.find('.cpu-temperature').exists()).toBe(false);
    await wrapper.setProps({ compact: false });
    expect(wrapper.getComponent(MdTooltip).props('text')).toBe(enUS.cpuTemperature.unsupportedHint);
    expect(wrapper.get('[role="status"]').text()).toBe(enUS.systemStatus.unsupported);
    expect(wrapper.text()).not.toContain('0°C');
    wrapper.unmount();
  });
  it('labels an earlier observation after a failed query', async () => {
    const wrapper = render(true);
    await wrapper.setProps({ reading: { ...reading(), status: 'failed' } });
    expect(wrapper.text()).toContain('49°C');
    expect(wrapper.getComponent(MdTooltip).props('text')).toContain(enUS.systemStatus.failed);

    wrapper.unmount();
  });
});
