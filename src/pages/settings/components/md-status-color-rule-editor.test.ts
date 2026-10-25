// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import Editor from './md-status-color-rule-editor.vue';
import en from '@/locales/en-US.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import ja from '@/locales/ja-JP.json';
import ko from '@/locales/ko-KR.json';
import ru from '@/locales/ru-RU.json';
import pt from '@/locales/pt-BR.json';
import tr from '@/locales/tr-TR.json';
import uk from '@/locales/uk-UA.json';
import { colorMetricLimit, colorMetricUnit, defaultColorRule } from '@/lib/models/resident/resident-color-rules';
import type { MetricColorRule } from '@/lib/models/resident/resident-color-rules';

const locales = { en, zh, tw, ja, ko, ru, pt, tr, uk };
const wrappers: VueWrapper[] = [];
afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount());
  document.body.innerHTML = '';
});
async function editor(locale: keyof typeof locales = 'en', rule = defaultColorRule('cpuTemperature')) {
  const wrapper = mount(Editor, {
    attachTo: document.body,
    props: { rule, name: 'CPU', saving: false },
    global: {
      renderStubDefaultSlot: true,
      stubs: { teleport: true },
      plugins: [createI18n({ legacy: false, locale, messages: locales, missingWarn: false, fallbackWarn: false })],
    },
  });
  wrappers.push(wrapper);
  await flushPromises();
  return wrapper;
}
function button(wrapper: VueWrapper, text: string) {
  return wrapper.findAll('button').find(button => button.text() === text)!;
}
async function value(wrapper: VueWrapper, selector: string, text: string) {
  const input = wrapper.get(selector);
  await input.setValue(text);
  await input.trigger('blur');
}

describe('color rule dialog', () => {
  it.each(Object.keys(locales) as (keyof typeof locales)[])(
    'restores editable fan speed defaults in %s',
    async locale => {
      const source: MetricColorRule = { id: 'fanSpeed', enabled: false, rules: [] };
      const wrapper = await editor(locale, source);
      await button(wrapper, locales[locale].systemStatus.colorRestore).trigger('click');
      await flushPromises();
      expect(colorMetricUnit('fanSpeed')).toBe('RPM');
      expect(colorMetricLimit('fanSpeed')).toBe(30000);
      await value(wrapper, '#color-from-0', '2500');
      await value(wrapper, '#color-from-1', '2500');
      await button(wrapper, locales[locale].common.save).trigger('click');
      const saved = wrapper.emitted('save')?.[0]?.[0] as MetricColorRule;
      expect(saved).toEqual({
        id: 'fanSpeed',
        enabled: true,
        rules: [
          { condition: 'below', value: 2500, color: 'green' },
          { condition: 'range', from: 2500, to: 4000, color: 'orange' },
          { condition: 'atLeast', value: 4000, color: 'red' },
        ],
      });
      expect(source).toEqual({ id: 'fanSpeed', enabled: false, rules: [] });
      expect(defaultColorRule('fanSpeed').rules[0]).toEqual({ condition: 'below', value: 2000, color: 'green' });
    }
  );

  it('restores enabled disk temperature defaults without changing the saved source', async () => {
    const source: MetricColorRule = { id: 'diskTemperature', enabled: false, rules: [] };
    const wrapper = await editor('en', source);
    await button(wrapper, en.systemStatus.colorRestore).trigger('click');
    await flushPromises();
    await button(wrapper, en.common.save).trigger('click');
    expect(wrapper.emitted('save')?.[0]?.[0]).toEqual({
      id: 'diskTemperature',
      enabled: true,
      rules: [
        { condition: 'below', value: 50, color: 'green' },
        { condition: 'range', from: 50, to: 60, color: 'orange' },
        { condition: 'atLeast', value: 60, color: 'red' },
      ],
    });
    expect(source).toEqual({ id: 'diskTemperature', enabled: false, rules: [] });
  });

  it('restores enabled battery temperature rules with battery-specific thresholds', async () => {
    const wrapper = await editor('en', { id: 'batteryTemperature', enabled: false, rules: [] });
    await button(wrapper, en.systemStatus.colorRestore).trigger('click');
    await flushPromises();
    await button(wrapper, en.common.save).trigger('click');
    expect(wrapper.emitted('save')?.[0]?.[0]).toEqual({
      id: 'batteryTemperature',
      enabled: true,
      rules: [
        { condition: 'below', value: 40, color: 'green' },
        { condition: 'range', from: 40, to: 45, color: 'orange' },
        { condition: 'atLeast', value: 45, color: 'red' },
      ],
    });
  });
  it.each(Object.keys(locales) as (keyof typeof locales)[])('renders translated modal controls in %s', async locale => {
    const wrapper = await editor(locale);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(wrapper.text()).toContain(locales[locale].common.save);
    expect(wrapper.text()).toContain(locales[locale].systemStatus.colorRestore);
    expect(wrapper.text()).not.toMatch(/common\.|systemStatus\./);
    expect(wrapper.find('[id^="color-preview"]').exists()).toBe(false);
  });
  it('commits Number Field input and preserves the source while editing sparse temperature rules', async () => {
    const rule: MetricColorRule = {
      id: 'cpuTemperature',
      enabled: true,
      rules: [
        { condition: 'below', value: 60, color: 'green' },
        { condition: 'atLeast', value: 120, color: 'red' },
      ],
    };
    const wrapper = await editor('en', rule);
    await value(wrapper, '#color-from-0', '65');
    expect(rule.rules[0]).toEqual({ condition: 'below', value: 60, color: 'green' });
    await wrapper.get('#color-from-1').trigger('keydown', { key: 'ArrowUp' });
    await button(wrapper, en.common.save).trigger('click');
    expect(wrapper.emitted('save')?.[0]?.[0]).toEqual({
      ...rule,
      rules: [
        { condition: 'below', value: 65, color: 'green' },
        { condition: 'atLeast', value: 121, color: 'red' },
      ],
    });
  });
  it('deletes every rule, saves an empty set and adds an independent rule without automatic thresholds', async () => {
    const wrapper = await editor();
    for (let index = 0; index < 3; index++) await button(wrapper, en.systemStatus.colorRemove).trigger('click');
    await button(wrapper, en.common.save).trigger('click');
    expect(wrapper.emitted('save')?.[0]?.[0]).toEqual({ id: 'cpuTemperature', enabled: true, rules: [] });
    await button(wrapper, en.systemStatus.colorAddRule).trigger('click');
    expect(wrapper.get('#color-from-3').element).toHaveProperty('value', '');
    expect(button(wrapper, en.common.save).attributes('disabled')).toBeDefined();
    await value(wrapper, '#color-from-3', '100');
    await button(wrapper, en.common.save).trigger('click');
    expect(wrapper.emitted('save')?.[1]?.[0]).toEqual({
      id: 'cpuTemperature',
      enabled: true,
      rules: [{ condition: 'atLeast', value: 100, color: 'red' }],
    });
  });
  it('identifies overlapping rules, permits adjacent boundaries and blocks empty or fractional values', async () => {
    const wrapper = await editor();
    await value(wrapper, '#color-from-0', '61');
    expect(wrapper.get('[role="alert"]').text()).toContain('Rules 1 and 2');
    expect(button(wrapper, en.common.save).attributes('disabled')).toBeDefined();
    await value(wrapper, '#color-from-0', '60');
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    for (const text of ['', '60.5']) {
      await value(wrapper, '#color-from-0', text);
      expect(button(wrapper, en.common.save).attributes('disabled')).toBeDefined();
    }
    await button(wrapper, en.common.cancel).trigger('click');
    expect(wrapper.emitted('cancel')).toHaveLength(1);
    expect(wrapper.emitted('save')).toBeUndefined();
  });
  it('keeps drafts available when saving fails', async () => {
    const wrapper = await editor();
    await wrapper.setProps({ error: 'Save failed' });
    expect(wrapper.get('[role="alert"]').text()).toBe('Save failed');
    expect(button(wrapper, en.common.save).attributes('disabled')).toBeUndefined();
  });
});
