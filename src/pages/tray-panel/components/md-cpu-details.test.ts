// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { describe, expect, it, vi } from 'vitest';
import MdCpuDetails from './md-cpu-details.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdIconTrend from '@/components/icons/md-icon-trend.vue';
import enUS from '@/locales/en-US.json';
import type { ResourceReadings } from '@/lib/models/system-resources/system-resources';
import { emptyReadings } from '@/lib/utils/system-resources/system-resources';
vi.mock('@/lib/services/settings/operating-system-service', () => ({
  OperatingSystemService: { isMacOs: () => true },
}));

const render = (reading: ResourceReadings, temperatureEnabled = true, temperatureSupported = true) =>
  mount(MdCpuDetails, {
    props: { reading, temperatureEnabled, temperatureSupported },
    global: { plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': enUS } })] },
  });
const windowsReading = () => {
  const reading = emptyReadings();
  reading.observedAtMs = 70_000;
  reading.cpu = { status: 'ready', sampledAtMs: 70_000, value: { usedPercent: 60 } };
  reading.cpuIdentity = { model: 'Intel CPU @ 3.70GHz', frequencyClasses: [], nominalFrequencyMhz: 3700 };
  reading.cpuFrequency = {
    status: 'ready',
    sampledAtMs: 70_000,
    value: {
      averageMhz: 4860,
      efficiencyMhz: null,
      performanceMhz: null,
      superMhz: null,
      source: 'windowsPerformance',
      activity: [],
    },
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
  it('keeps the temperature heading and shows setup in the chart area when collection is off', async () => {
    const reading = windowsReading();
    reading.cpuTemperature = {
      status: 'ready',
      sampledAtMs: 70_000,
      value: { celsius: 60, kind: 'package', source: 'intelDtsPawnIo', sensorCount: 1 },
    };
    reading.cpuTemperatureHistory = [{ sampledAtMs: 70_000, primary: 60, secondary: null }];
    const wrapper = render(reading, false);
    const card = wrapper.get('.temperature-details');
    expect(card.get('.temperature-disabled-heading').text()).toContain(enUS.cpuTemperature.label);
    expect(card.get('strong').text()).toBe('—');
    expect(card.get('.temperature-chart').text()).toContain(enUS.windowsMonitoring.collectionOff);
    expect(card.findComponent(MdIconTrend).exists()).toBe(false);
    await card.get('button').trigger('click');
    expect(wrapper.emitted('configureTemperature')).toHaveLength(1);
    await wrapper.setProps({ temperatureEnabled: true });
    expect(wrapper.get('.temperature-details').text()).toContain('60°C');
    expect(wrapper.find('.temperature-placeholder').exists()).toBe(false);
    wrapper.unmount();
  });

  it.each(['loading', 'stale', 'failed'] as const)('places missing %s readings in the chart area', status => {
    const reading = windowsReading();
    reading.cpuTemperature.status = status;
    const wrapper = render(reading);
    expect(wrapper.get('.temperature-chart [role="status"]').text()).toBe(enUS.systemStatus[status]);
    expect(wrapper.find('.temperature-status').exists()).toBe(false);
    wrapper.unmount();
  });

  it('hides the entire temperature card when collection is unsupported', () => {
    const reading = emptyReadings();
    const wrapper = render(reading, false, false);
    expect(wrapper.find('.temperature-details').exists()).toBe(false);
    expect(wrapper.text()).not.toContain(enUS.windowsMonitoring.collectionOff);
    wrapper.unmount();
  });

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
    expect(wrapper.get('.temperature-details').findComponent(MdIconTrend).props('series')[0].line).toContain(' L');
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
    const line = wrapper.get('.temperature-details').findComponent(MdIconTrend).props('series')[0].line as string;
    expect(line).not.toContain(' L');
    expect(line).toContain(' l0.001,0');
    wrapper.unmount();
  });
  it('shows real recent load statistics alongside frequency with coverage hints and without rated data', async () => {
    const wrapper = render(windowsReading());
    const facts = wrapper.findAll('.resource-fact');
    expect(facts).toHaveLength(3);
    expect(facts[0].text()).toContain('4.9GHz');
    expect(facts[1].text()).toContain('1 min average45%');
    expect(facts[2].text()).toContain('1 min peak60%');
    expect(facts.map(fact => fact.text()).join(' ')).not.toContain('3.70');
    expect(facts[0].findComponent(MdTooltip).exists()).toBe(false);
    expect(facts[1].findComponent(MdTooltip).props('text')).toContain('estimated coverage: 4.0s');
    expect(facts[2].findComponent(MdTooltip).props('text')).toContain('gaps excluded');
    await facts[0].trigger('mouseenter');
    expect(wrapper.find('[role="tooltip"]').exists()).toBe(false);
    wrapper.unmount();
  });
  it('keeps frequency and load freshness independent and expires old statistics', async () => {
    const reading = windowsReading();
    reading.cpuFrequency.status = 'stale';
    const wrapper = render(reading);
    expect(wrapper.findAll('.cached')).toHaveLength(0);
    expect(wrapper.findAll('.resource-fact')[0].text()).toContain('4.9GHz');
    await wrapper.setProps({ reading: { ...reading, cpu: { ...reading.cpu, status: 'stale' } } });
    expect(wrapper.findAll('.cached')).toHaveLength(0);
    expect(wrapper.findAll('.resource-fact')[1].text()).toContain('45%');
    await wrapper.setProps({ reading: { ...reading, observedAtMs: 140_000 } });
    expect(wrapper.findAll('.resource-fact')).toHaveLength(1);
    expect(wrapper.find('.compact').exists()).toBe(true);
    await wrapper.setProps({
      reading: { ...reading, observedAtMs: 140_000, cpuFrequency: { ...reading.cpuFrequency, status: 'failed' } },
    });
    expect(wrapper.find('dl').exists()).toBe(false);
    wrapper.unmount();
  });
  it('identifies nominal coverage as an estimate when the first interval is unavailable', () => {
    const reading = emptyReadings();
    reading.observedAtMs = 70_000;
    reading.cpu = { status: 'ready', sampledAtMs: 70_000, value: { usedPercent: 60 } };
    // The transport carries no counter interval for this first successful reading.
    reading.cpuHistory = [{ sampledAtMs: 70_000, primary: 60, secondary: null }];
    const wrapper = render(reading);
    try {
      const hints = wrapper.findAllComponents(MdTooltip).filter(hint => hint.props('text')?.includes('2.0'));
      expect(hints).toHaveLength(2);
      for (const hint of hints) expect(hint.props('text')).toContain('estimated');
    } finally {
      wrapper.unmount();
    }
  });
  it('does not manufacture missing frequency or historical zero readings', () => {
    const reading = emptyReadings();
    reading.cpuFrequency = {
      status: 'ready',
      sampledAtMs: 1,
      value: {
        averageMhz: NaN,
        efficiencyMhz: 0,
        performanceMhz: null,
        superMhz: null,
        source: 'windowsPerformance',
        activity: [],
      },
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
      superMhz: null,
      source: 'applePerformanceStates',
      activity: [],
    };
    const wrapper = render(reading);
    expect(wrapper.findAll('.resource-fact')).toHaveLength(3);
    expect(wrapper.text()).not.toContain('1 min peak');
    expect(wrapper.findAll('.resource-fact')[2].text()).toContain('1 min average45%');
    expect(wrapper.text()).toContain('Efficiency cores1.5GHz');
    expect(wrapper.text()).toContain('Performance cores4.0GHz');
    expect(wrapper.text()).not.toContain('Average frequency4.9');
    wrapper.unmount();
  });
  it('keeps M5 and M6 super-core frequencies distinct from other core classes', () => {
    for (const efficiencyMhz of [null, 1800]) {
      const reading = windowsReading();
      reading.cpuFrequency.value = {
        averageMhz: null,
        efficiencyMhz,
        performanceMhz: 3200,
        superMhz: 4700,
        source: 'applePerformanceStates',
        activity: [],
      };
      const wrapper = render(reading);
      expect(wrapper.text()).toContain('Performance cores3.2GHz');
      expect(wrapper.text()).toContain('Super cores4.7GHz');
      if (efficiencyMhz !== null) expect(wrapper.text()).toContain('Efficiency cores1.8GHz');
      else expect(wrapper.text()).not.toContain('Efficiency cores');
      expect(wrapper.findAll('.resource-fact')).toHaveLength(3);
      expect(wrapper.text().includes('1 min average')).toBe(efficiencyMhz === null);
      expect(wrapper.text()).not.toContain('1 min peak');
      wrapper.unmount();
    }
  });
  it('keeps three core frequencies in one row and restores load statistics when a core is idle', async () => {
    const reading = windowsReading();
    const frequency = {
      averageMhz: 3000,
      efficiencyMhz: 1800,
      performanceMhz: 3200,
      superMhz: 4700,
      source: 'applePerformanceStates' as const,
      activity: [],
    };
    reading.cpuFrequency.value = frequency;
    const wrapper = render(reading);
    expect(wrapper.findAll('.resource-fact').map(fact => fact.text())).toEqual([
      'Efficiency cores1.8GHz',
      'Performance cores3.2GHz',
      'Super cores4.7GHz',
    ]);
    await wrapper.setProps({
      reading: { ...reading, cpuFrequency: { ...reading.cpuFrequency, value: { ...frequency, performanceMhz: null } } },
    });
    expect(wrapper.findAll('.resource-fact')).toHaveLength(3);
    expect(wrapper.text()).toContain('1 min average45%');
    expect(wrapper.text()).not.toContain('Performance cores');
    await wrapper.setProps({ reading });
    expect(wrapper.findAll('.resource-fact')).toHaveLength(3);
    expect(wrapper.text()).not.toContain('1 min average');
    wrapper.unmount();
  });
  it('limits mixed overall and single-class frequency observations to three facts', () => {
    const reading = windowsReading();
    reading.cpuFrequency.value!.efficiencyMhz = 1500;
    const wrapper = render(reading);
    expect(wrapper.findAll('.resource-fact')).toHaveLength(3);
    expect(wrapper.text()).toContain('Average frequency4.9GHz');
    expect(wrapper.text()).toContain('Efficiency cores1.5GHz');
    expect(wrapper.text()).toContain('1 min average45%');
    expect(wrapper.text()).not.toContain('1 min peak');
    wrapper.unmount();
  });
});

it('places fresh component power inside the usage card and removes unavailable power', async () => {
  const reading = windowsReading();
  reading.cpuPower = { status: 'ready', sampledAtMs: 70000, value: { watts: 6.7, source: 'ioReport' } };
  const wrapper = render(reading);
  expect(wrapper.get('.cpu-summary .resource-meta [data-component-power="cpu"]').text()).toBe('6.7W');
  expect(wrapper.findAll('[data-component-power="cpu"]')).toHaveLength(1);
  await wrapper.setProps({
    reading: { ...reading, cpuPower: { status: 'unsupported', sampledAtMs: null, value: null } },
  });
  expect(wrapper.find('[data-component-power="cpu"]').exists()).toBe(false);
  wrapper.unmount();
});
