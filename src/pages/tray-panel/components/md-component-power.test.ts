// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { describe, expect, it } from 'vitest';
import MdComponentPower from './md-component-power.vue';
import enUS from '@/locales/en-US.json';
import type { MetricReading, ComponentPower } from '@/lib/models/system-resources/system-resources';

describe('component power presentation', () => {
  it('distinguishes a real zero from unavailable, invalid, and expired power', async () => {
    const reading: MetricReading<ComponentPower> = {
      status: 'ready',
      sampledAtMs: 1000,
      value: { watts: 0, source: 'ioReport' },
    };
    const wrapper = mount(MdComponentPower, {
      props: { metric: 'cpu', reading, observedAtMs: 1000, active: true },
      global: { plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': enUS } })] },
    });
    expect(wrapper.get('[data-component-power]').attributes('aria-label')).toContain('CPU power');
    expect(wrapper.text()).toContain('0.0W');
    for (const status of ['loading', 'failed', 'stale', 'unsupported'] as const) {
      await wrapper.setProps({ reading: { ...reading, status } });
      expect(wrapper.find('[data-component-power]').exists()).toBe(false);
    }
    await wrapper.setProps({ reading, observedAtMs: 6001 });
    expect(wrapper.find('[data-component-power]').exists()).toBe(false);
    await wrapper.setProps({ observedAtMs: 999 });
    expect(wrapper.find('[data-component-power]').exists()).toBe(false);
    for (const watts of [-1, NaN, Infinity, 1001]) {
      await wrapper.setProps({ observedAtMs: 1000, reading: { ...reading, value: { watts, source: 'ioReport' } } });
      expect(wrapper.find('[data-component-power]').exists()).toBe(false);
    }
    await wrapper.setProps({
      metric: 'gpu',
      observedAtMs: 1000,
      reading: { ...reading, value: { watts: 50.5, source: 'ioReport' } },
    });
    expect(wrapper.get('[data-component-power]').attributes('aria-label')).toContain('GPU power');
    expect(wrapper.text()).toContain('50.5W');
    wrapper.unmount();
  });
});
