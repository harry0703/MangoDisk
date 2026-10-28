// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { describe, expect, it } from 'vitest';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import ja from '@/locales/ja-JP.json';
import ko from '@/locales/ko-KR.json';
import de from '@/locales/de-DE.json';
import fr from '@/locales/fr-FR.json';
import es from '@/locales/es-ES.json';
import pt from '@/locales/pt-BR.json';
import ru from '@/locales/ru-RU.json';
import tr from '@/locales/tr-TR.json';
import uk from '@/locales/uk-UA.json';
import type { BatteryDetails } from '@/lib/models/system-resources/power';
import MdIcon from '@/components/icons/md-icon.vue';
import MdTooltip from '@/components/custom/md-tooltip.vue';
import { emptyReadings } from '@/lib/utils/system-resources/system-resources';
import Page from './md-power-details.vue';

const details: BatteryDetails = {
  chargePercent: 72,
  externalConnected: true,
  chargeState: 'charging',
  secondsToEmpty: null,
  secondsToFull: 4800,
  healthPercent: 84,
  cycleCount: 245,
  capacity: { current: 5200, fullCharge: 7218, design: 8579, unit: 'milliampHours' },
  voltageVolts: 12.77,
  currentMilliamps: 640,
  adapter: { inputWatts: 35.1, reportedWatts: 140, voltageVolts: 28 },
};
function reading() {
  const value = emptyReadings();
  value.observedAtMs = 10000;
  value.batteryPresent = true;
  value.batteryDetails = { status: 'ready', sampledAtMs: 10000, value: structuredClone(details) };
  value.batteryPower = {
    status: 'ready',
    sampledAtMs: 10000,
    value: { watts: 8.2, state: 'charging', source: 'appleSmartBattery' },
  };
  value.batteryTemperature = {
    status: 'ready',
    sampledAtMs: 10000,
    value: { celsius: 31, source: 'appleSmartBattery' },
  };
  value.systemPower = { status: 'ready', sampledAtMs: 10000, value: { watts: 26.4 } };
  return value;
}
describe.each([
  ['en-US', en],
  ['zh-CN', zh],
  ['zh-TW', tw],
  ['ja-JP', ja],
  ['ko-KR', ko],
  ['de-DE', de],
  ['fr-FR', fr],
  ['es-ES', es],
  ['pt-BR', pt],
  ['ru-RU', ru],
  ['tr-TR', tr],
  ['uk-UA', uk],
] as const)('power detail presentation (%s)', (locale, messages) => {
  function render(value = reading()) {
    return mount(Page, {
      props: { reading: value, active: true },
      global: {
        plugins: [createI18n({ legacy: false, locale, messages: { [locale]: messages } })],
        stubs: { MdResourceTrend: true, MdTooltip: { props: ['text'], template: '<span><slot /></span>' } },
      },
    });
  }
  it('separates native power sources and exposes every battery fact without disclosure', () => {
    const wrapper = render();
    const text = wrapper.text();
    for (const label of [
      messages.power.systemPower,
      messages.power.intoBattery,
      messages.power.inputPower,
      messages.power.reportedPower,
    ])
      expect(text).toContain(label);
    for (const watts of [26.4, 8.2, 35.1, 140])
      expect(text).toContain(new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(watts));
    expect(wrapper.find('details').exists()).toBe(false);
    expect(wrapper.findAll('.battery-facts .resource-fact')).toHaveLength(8);
    expect(wrapper.findAll('.adapter-facts .resource-fact')).toHaveLength(3);
    for (const key of ['fullCharge', 'design', 'batteryVoltage', 'batteryCurrent', 'adapterVoltage'] as const)
      expect(text).toContain(messages.power[key]);
    expect(wrapper.getComponent(MdTooltip).props('text')).toContain(messages.power.current);
    expect(wrapper.getComponent(MdTooltip).props('text')).toContain(new Intl.NumberFormat(locale).format(5200));
    expect(text).not.toContain(messages.power.current);
    expect(text).toContain('mAh');
    expect(text).toContain('84');
    wrapper.unmount();
  });
  it('shows a connected full battery without a fictitious charging-time estimate', () => {
    const value = reading();
    value.batteryDetails.value!.chargeState = 'full';
    value.batteryDetails.value!.secondsToFull = null;
    const wrapper = render(value);
    expect(wrapper.text()).toContain(messages.power.states.full);
    expect(wrapper.find('.battery-state small').exists()).toBe(false);
    expect(wrapper.text()).not.toContain('65535');
    wrapper.unmount();
  });
  it('does not imply an active estimate when the OS provides no charging time', () => {
    const value = reading();
    value.batteryDetails.value!.secondsToFull = null;
    const wrapper = render(value);
    expect(wrapper.text()).toContain(messages.power.states.charging);
    expect(wrapper.find('.battery-state small').exists()).toBe(false);
    wrapper.unmount();
  });
  it('distinguishes finishing from full without inventing a remaining time', async () => {
    const value = reading();
    Object.assign(value.batteryDetails.value!, {
      chargePercent: 100,
      chargeState: 'finishing',
      secondsToFull: null,
    });
    const wrapper = render(value);
    expect(wrapper.get('.battery-state').text()).toBe(messages.power.states.finishing);
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('batteryCharging');
    expect(wrapper.find('.battery-state small').exists()).toBe(false);
    const estimate = structuredClone(value);
    estimate.batteryDetails.value!.secondsToFull = 120;
    await wrapper.setProps({ reading: estimate });
    expect(wrapper.find('.battery-state small').exists()).toBe(true);
    const full = structuredClone(estimate);
    full.batteryDetails.value!.chargeState = 'full';
    await wrapper.setProps({ reading: full });
    expect(wrapper.get('.battery-state').text()).toBe(messages.power.states.full);
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('plug');
    expect(wrapper.find('.battery-state small').exists()).toBe(false);
    wrapper.unmount();
  });
  it('keeps a full percentage independent from the native charging state', () => {
    const value = reading();
    value.batteryDetails.value!.chargePercent = 100;
    const wrapper = render(value);
    expect(wrapper.get('.battery-state').text()).toContain(messages.power.states.charging);
    expect(wrapper.get('.battery-state').text()).not.toContain(messages.power.states.finishing);
    wrapper.unmount();
  });
  it('preserves Windows energy-capacity units and missing current', () => {
    const value = reading();
    value.batteryDetails.value!.capacity!.unit = 'milliwattHours';
    value.batteryDetails.value!.currentMilliamps = null;
    const wrapper = render(value);
    expect(wrapper.text()).toContain('mWh');
    expect(wrapper.text()).not.toContain('mAh');
    expect(wrapper.text()).toContain('—');
    wrapper.unmount();
  });
  it('omits unsupported system meters and adapters without native facts', async () => {
    const value = reading();
    value.batteryDetails.value!.adapter = null;
    const wrapper = render(value);
    await wrapper.setProps({ systemPowerSupported: false });
    expect(wrapper.find('.system-power').exists()).toBe(false);
    expect(wrapper.text()).not.toContain(messages.power.adapter);
    expect(wrapper.text()).toContain(messages.power.details);
    wrapper.unmount();
  });
  it('keeps system power on desktops without a battery section', () => {
    const value = reading();
    value.batteryPresent = false;
    const wrapper = render(value);
    expect(wrapper.text()).toContain(new Intl.NumberFormat(locale).format(26.4));
    expect(wrapper.find('.battery-status').exists()).toBe(false);
    expect(wrapper.find('details').exists()).toBe(false);
    expect(wrapper.text()).not.toContain(messages.power.adapter);
    wrapper.unmount();
  });
  it('keeps battery and adapter markup unchanged when a background refresh completes', async () => {
    const value = reading();
    value.observedAtMs = 16000;
    value.batteryDetails.status = 'stale';
    value.batteryPower.status = 'stale';
    value.batteryTemperature.status = 'stale';
    const wrapper = render(value);
    const before = ['.battery-status', '.battery-facts', '.adapter-facts'].map(selector =>
      wrapper.get(selector).html()
    );
    const updated = reading();
    updated.observedAtMs = 16000;
    await wrapper.setProps({ reading: updated });
    expect(
      ['.battery-status', '.battery-facts', '.adapter-facts'].map(selector => wrapper.get(selector).html())
    ).toEqual(before);
    wrapper.unmount();
  });
  it('shows previous power and battery facts without a refresh label', () => {
    const value = reading();
    value.observedAtMs = 16000;
    const wrapper = render(value);
    expect(wrapper.text()).toContain(new Intl.NumberFormat(locale).format(26.4));
    expect(wrapper.text()).toContain('245');
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    expect(wrapper.text()).toContain(messages.power.states.charging);
    wrapper.unmount();
  });
});

describe('power status and capacity', () => {
  function render(value = reading()) {
    return mount(Page, {
      props: { reading: value, active: true },
      global: {
        plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': en } })],
        stubs: { MdResourceTrend: true, MdTooltip: { props: ['text'], template: '<span><slot /></span>' } },
      },
    });
  }
  it.each([
    ['charging', true, 72, 'batteryCharging'],
    ['charging', null, 72, 'batteryCharging'],
    ['charging', false, 72, 'battery'],
    ['finishing', true, 100, 'batteryCharging'],
    ['finishing', null, 100, 'batteryCharging'],
    ['finishing', false, 100, 'battery'],
    ['full', true, 100, 'plug'],
    ['paused', true, 80, 'plug'],
    ['connected', true, 80, 'plug'],
    ['unknown', true, 80, 'battery'],
    ['unknown', false, 80, 'battery'],
    ['full', false, 100, 'batteryFull'],
    ['discharging', false, 100, 'batteryFull'],
    ['discharging', true, 100, 'batteryFull'],
    ['discharging', null, 100, 'batteryFull'],
    ['discharging', false, 76, 'batteryFull'],
    ['discharging', false, 75, 'batteryMedium'],
    ['discharging', false, 26, 'batteryMedium'],
    ['discharging', false, 25, 'batteryLow'],
    ['discharging', false, 1, 'batteryLow'],
    ['discharging', false, 0, 'battery'],
    ['discharging', false, null, 'battery'],
    ['discharging', false, -1, 'battery'],
    ['discharging', false, 101, 'battery'],
    ['discharging', false, Number.NaN, 'battery'],
  ] as const)('renders %s with external=%s and level=%s as %s', (state, external, percent, expected) => {
    const value = reading();
    Object.assign(value.batteryDetails.value!, {
      chargeState: state,
      externalConnected: external,
      chargePercent: percent,
    });
    const wrapper = render(value);
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe(expected);
    expect(wrapper.get('.battery-shell').attributes('aria-hidden')).toBe('true');
    const source = wrapper.find('.battery-source');
    const expectedSource =
      expected === 'batteryCharging' ? 'zap' : expected === 'plug' ? 'plug' : state === 'unknown' ? 'help' : null;
    expect(source.exists()).toBe(expectedSource !== null);
    if (expectedSource) expect(source.attributes('data-source')).toBe(expectedSource);
    if (expectedSource === 'zap') expect(wrapper.get('.battery-status').attributes('data-status-tone')).toBe('success');
    expect(wrapper.text()).toContain(en.power.states[state]);
    wrapper.unmount();
  });
  it.each([
    [0, 'critical'],
    [15, 'critical'],
    [19.9, 'critical'],
    [20, 'low'],
    [40, 'low'],
    [40.1, 'healthy'],
    [100, 'healthy'],
    [null, 'unknown'],
    [Number.NaN, 'unknown'],
    [-1, 'unknown'],
    [101, 'unknown'],
  ] as const)('colors a %s percent battery as %s independently of its supply state', async (percent, tone) => {
    const value = reading();
    const wrapper = render(value);
    for (const state of ['discharging', 'charging', 'connected', 'paused', 'full', 'unknown'] as const) {
      Object.assign(value.batteryDetails.value!, {
        chargeState: state,
        externalConnected: state === 'discharging' ? false : true,
        chargePercent: percent,
      });
      await wrapper.setProps({ reading: structuredClone(value) });
      expect(wrapper.get('.battery-status').attributes('data-level-tone')).toBe(tone);
    }
    wrapper.unmount();
  });
  it.each([
    ['discharging', 100, 'normal'],
    ['discharging', 20, 'normal'],
    ['discharging', 15.1, 'normal'],
    ['discharging', 15, 'critical'],
    ['discharging', 8, 'critical'],
    ['discharging', null, 'normal'],
    ['charging', 8, 'success'],
    ['paused', 80, 'normal'],
    ['connected', 80, 'normal'],
    ['full', 100, 'success'],
    ['unknown', 80, 'normal'],
  ] as const)('colors the %s state at %s percent as %s', (state, percent, tone) => {
    const value = reading();
    Object.assign(value.batteryDetails.value!, {
      chargeState: state,
      externalConnected: state === 'discharging' ? false : true,
      chargePercent: percent,
    });
    const wrapper = render(value);
    expect(wrapper.get('.battery-status').attributes('data-status-tone')).toBe(tone);
    wrapper.unmount();
  });
  it.each([
    [105, 'success'],
    [100, 'success'],
    [90, 'success'],
    [89.9, 'warning'],
    [89.95, 'success'],
    [85, 'warning'],
    [80, 'warning'],
    [79.9, 'critical'],
    [79.95, 'warning'],
    [0, 'critical'],
    [null, undefined],
    [-1, undefined],
    [Number.NaN, undefined],
    [Number.POSITIVE_INFINITY, undefined],
  ] as const)('colors capacity retention %s as %s independently of charge level', (percent, tone) => {
    const value = reading();
    Object.assign(value.batteryDetails.value!, {
      healthPercent: percent,
      chargeState: 'discharging',
      externalConnected: false,
      chargePercent: 18,
    });
    const wrapper = render(value);
    const health = wrapper
      .findAll('.battery-facts .resource-fact')
      .find(fact => fact.get('dt').text() === en.power.health)!;
    expect(health.get('dd').attributes('data-tone')).toBe(tone);
    if (percent === 89.9 || percent === 79.9) expect(health.get('dd').text()).toContain(String(percent));
    if (percent === 89.95) expect(health.get('dd').text()).toContain('90');
    if (percent === 79.95) expect(health.get('dd').text()).toContain('80');
    expect(wrapper.get('.battery-capacity').attributes('data-health-tone')).toBe(tone);
    expect(wrapper.get('.battery-status').attributes('data-level-tone')).toBe('critical');
    expect(wrapper.findAll('dd[data-tone]')).toHaveLength(tone ? 1 : 0);
    if (!tone) expect(health.get('dd').text()).toContain('—');
    wrapper.unmount();
  });
  it.each([79.9, 83.7, 89.9, 89.95])('keeps the accessible capacity percentage consistent at %s', percent => {
    const value = reading();
    Object.assign(value.batteryDetails.value!, {
      healthPercent: percent,
      capacity: { current: 5000, fullCharge: percent * 100, design: 10000, unit: 'milliampHours' },
    });
    const wrapper = render(value);
    const health = wrapper
      .findAll('.battery-facts .resource-fact')
      .find(fact => fact.get('dt').text() === en.power.health)!;
    expect(wrapper.get('[role="meter"]').attributes('aria-valuetext')).toBe(health.get('dd').text());
    wrapper.unmount();
  });
  it('compares full-charge and design capacity without confusing it with the current charge level', async () => {
    const value = reading();
    const wrapper = render(value);
    expect(Number(wrapper.get('[role="meter"]').attributes('aria-valuenow'))).toBeCloseTo((7218 / 8579) * 100);
    value.batteryDetails.value!.capacity!.current = 1000;
    await wrapper.setProps({ reading: structuredClone(value) });
    expect(Number(wrapper.get('[role="meter"]').attributes('aria-valuenow'))).toBeCloseTo((7218 / 8579) * 100);
    value.batteryDetails.value!.capacity!.fullCharge = 9000;
    await wrapper.setProps({ reading: structuredClone(value) });
    expect(wrapper.get('[role="meter"]').attributes('aria-valuenow')).toBe('100');
    expect(wrapper.text()).toContain('9,000');
    wrapper.unmount();
  });
  it.each([null, 0, -1, Number.NaN, Number.POSITIVE_INFINITY])(
    'omits the capacity meter when design capacity is invalid (%s)',
    design => {
      const value = reading();
      value.batteryDetails.value!.capacity!.design = design;
      const wrapper = render(value);
      expect(wrapper.find('[role="meter"]').exists()).toBe(false);
      expect(wrapper.text()).toContain(en.power.fullCharge);
      expect(wrapper.text()).toContain('7,218');
      wrapper.unmount();
    }
  );
  it('updates the icon when power changes and retains the previous supply state', async () => {
    const value = reading();
    const wrapper = render(value);
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('batteryCharging');
    const onBattery = structuredClone(value);
    Object.assign(onBattery.batteryDetails.value!, { chargeState: 'discharging', externalConnected: false });
    await wrapper.setProps({ reading: onBattery });
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('batteryMedium');
    const plugged = structuredClone(value);
    Object.assign(plugged.batteryDetails.value!, { chargeState: 'full', chargePercent: 100 });
    await wrapper.setProps({ reading: plugged });
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('plug');
    await wrapper.setProps({ reading: { ...plugged, observedAtMs: 16000 } });
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('plug');
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    await wrapper.setProps({ reading: emptyReadings() });
    expect(wrapper.get('.battery-state-label').getComponent(MdIcon).props('name')).toBe('battery');
    wrapper.unmount();
  });
});

describe('power cache lifecycle', () => {
  function render(value = reading()) {
    return mount(Page, {
      props: { reading: value, active: true },
      global: {
        plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': en } })],
        stubs: { MdResourceTrend: true, MdTooltip: { props: ['text'], template: '<span><slot /></span>' } },
      },
    });
  }
  it('retains cached facts on reopen and replaces them when a new observation arrives', async () => {
    const value = reading();
    value.observedAtMs = 70000;
    value.batteryDetails.status = 'stale';
    const wrapper = render(value);
    expect(wrapper.get('.battery-level').text()).toContain('72');
    expect(wrapper.get('.adapter-facts').text()).toContain('35.1');
    expect(wrapper.find('.battery-state [role="status"]').exists()).toBe(false);
    expect(wrapper.get('.battery-state').text()).toContain('1 h 20 min');
    await wrapper.setProps({ active: false });
    await wrapper.setProps({ active: true });
    expect(wrapper.get('.battery-level').text()).toContain('72');
    const updated = reading();
    updated.batteryDetails.value!.chargePercent = 73;
    await wrapper.setProps({ reading: updated });
    expect(wrapper.get('.battery-level').text()).toContain('73');
    expect(wrapper.find('.battery-state [role="status"]').exists()).toBe(false);
    updated.batteryDetails.value!.externalConnected = false;
    updated.batteryDetails.value!.adapter = null;
    await wrapper.setProps({ reading: structuredClone(updated) });
    expect(wrapper.find('.adapter-facts').exists()).toBe(false);
    wrapper.unmount();
  });
  it('retains failure context beside the last facts and clears them on physical removal', async () => {
    const value = reading();
    value.batteryDetails.status = 'failed';
    const wrapper = render(value);
    expect(wrapper.get('.battery-level').text()).toContain('72');
    expect(wrapper.get('.battery-state').text()).toContain(en.systemStatus.failed);
    value.batteryPresent = false;
    await wrapper.setProps({ reading: structuredClone(value) });
    expect(wrapper.find('.battery-status').exists()).toBe(false);
    expect(wrapper.find('.adapter-facts').exists()).toBe(false);
    wrapper.unmount();
  });
});
