// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { describe, expect, it } from 'vitest';
import MdCpuDetails from './md-cpu-details.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIconTrend from '@/components/icons/md-icon-trend.vue';
import enUS from '@/locales/en-US.json';
import type { ResourceReadings } from '@/lib/models/system-resources';
import { emptyReadings } from '@/lib/utils/system-resources';
const render = (reading: ResourceReadings) =>
  mount(MdCpuDetails, {
    props: { reading },
    global: { plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': enUS } })] },
  });
const windowsReading = () => {
  const reading = emptyReadings();
  reading.observedAtMs = 70_000;
  reading.cpu = { status: 'ready', sampledAtMs: 70_000, value: { usedPercent: 60 } };
  reading.cpuIdentity = { model: 'Intel CPU @ 3.70GHz', nominalFrequencyMhz: 3700 };
  reading.cpuFrequency = {
    status: 'ready',
    sampledAtMs: 70_000,
    value: { averageMhz: 4860, efficiencyMhz: null, performanceMhz: null, source: 'windowsPerformance' },
  };
  reading.cpuHistory = [
    [9999, 100],
    [10000, 0],
    [30000, 30],
    [70000, 60],
    [70001, 100],
    [50000, NaN],
    [50001, -1],
    [50002, 101],
  ].map(([sampledAtMs, primary]) => ({ sampledAtMs: sampledAtMs!, primary: primary!, secondary: null }));
  return reading;
};
describe('CPU detail observations', () => {
  it('starts a new temperature curve when the sensor group changes', async () => {
    const reading = emptyReadings();
    reading.observedAtMs = 70_000;
    reading.cpuTemperature = {
      status: 'ready',
      sampledAtMs: 70_000,
      value: { celsius: 60, kind: 'coreAverage', source: 'appleSmc', sensorCount: 16 },
    };
    reading.cpuTemperatureHistory = [
      { sampledAtMs: 66_000, primary: 50, secondary: null },
      { sampledAtMs: 70_000, primary: 60, secondary: null },
    ];
    const wrapper = render(reading);
    expect(wrapper.findComponent(MdIconTrend).props('series')[0].line).toContain(' L');
    await wrapper.setProps({
      reading: {
        ...reading,
        observedAtMs: 74_000,
        cpuTemperature: {
          ...reading.cpuTemperature,
          sampledAtMs: 74_000,
          value: { ...reading.cpuTemperature.value!, celsius: 65, sensorCount: 8 },
        },
        cpuTemperatureHistory: [{ sampledAtMs: 74_000, primary: 65, secondary: null }],
      },
    });
    const line = wrapper.findComponent(MdIconTrend).props('series')[0].line as string;
    expect(line).not.toContain(' L');
    expect(line).toContain(' l0.001,0');
    wrapper.unmount();
  });
  it('shows real recent load statistics alongside frequency without rated data or tooltips', async () => {
    const wrapper = render(windowsReading());
    const facts = wrapper.findAll('.resource-fact');
    expect(facts).toHaveLength(3);
    expect(facts[0].text()).toContain('4.86GHz');
    expect(facts[1].text()).toContain('1 min average30%');
    expect(facts[2].text()).toContain('1 min peak60%');
    expect(wrapper.text()).not.toContain('3.70');
    expect(facts.every(fact => !fact.findComponent(MdTooltip).exists())).toBe(true);
    await facts[0].trigger('mouseenter');
    expect(wrapper.find('[role="tooltip"]').exists()).toBe(false);
    wrapper.unmount();
  });
  it('keeps frequency and load freshness independent and expires old statistics', async () => {
    const reading = windowsReading();
    reading.cpuFrequency.status = 'stale';
    const wrapper = render(reading);
    expect(wrapper.findAll('.cached')).toHaveLength(1);
    expect(wrapper.findAll('.resource-fact')[0].text()).toContain('Cached');
    await wrapper.setProps({ reading: { ...reading, cpu: { ...reading.cpu, status: 'stale' } } });
    expect(wrapper.findAll('.cached')).toHaveLength(3);
    await wrapper.setProps({ reading: { ...reading, observedAtMs: 140_000 } });
    expect(wrapper.findAll('.resource-fact')).toHaveLength(1);
    expect(wrapper.find('.compact').exists()).toBe(true);
    await wrapper.setProps({
      reading: { ...reading, observedAtMs: 140_000, cpuFrequency: { ...reading.cpuFrequency, status: 'failed' } },
    });
    expect(wrapper.find('dl').exists()).toBe(false);
    wrapper.unmount();
  });
  it('does not manufacture missing frequency or historical zero readings', () => {
    const reading = emptyReadings();
    reading.cpuFrequency = {
      status: 'ready',
      sampledAtMs: 1,
      value: { averageMhz: NaN, efficiencyMhz: 0, performanceMhz: null, source: 'windowsPerformance' },
    };
    const wrapper = render(reading);
    expect(wrapper.find('dl').exists()).toBe(false);
    wrapper.unmount();
  });
  it('shows both Apple frequency classes and average load in three cards without peak or overall frequency', () => {
    const reading = windowsReading();
    reading.cpuFrequency.value = {
      averageMhz: null,
      efficiencyMhz: 1500,
      performanceMhz: 4000,
      source: 'applePerformanceStates',
    };
    const wrapper = render(reading);
    expect(wrapper.findAll('.resource-fact')).toHaveLength(3);
    expect(wrapper.text()).not.toContain('1 min peak');
    expect(wrapper.findAll('.resource-fact')[2].text()).toContain('1 min average30%');
    expect(wrapper.text()).toContain('Efficiency cores1.50GHz');
    expect(wrapper.text()).toContain('Performance cores4.00GHz');
    expect(wrapper.text()).not.toContain('Average frequency4.86');
    wrapper.unmount();
  });
});
