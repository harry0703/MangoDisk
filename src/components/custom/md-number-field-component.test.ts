// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { i18n } from '@/i18n';
import MdNumberField from './md-number-field.vue';

describe('optional number field', () => {
  it('supports decimal buttons and keyboard input without changing a value on scroll', async () => {
    const wrapper = mount(MdNumberField, {
      props: {
        id: 'temperature',
        label: 'Temperature',
        modelValue: 0.75,
        min: 0,
        max: 2,
        step: 0.1,
        stepSnapping: false,
        'onUpdate:modelValue': value => wrapper.setProps({ modelValue: value }),
      },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    try {
      await flushPromises();
      const input = wrapper.get('input');
      expect(input.element.value).toBe('0.75');
      const increase = wrapper.get('[data-slot="increment"]');
      await increase.trigger('pointerdown', { button: 0 });
      window.dispatchEvent(new PointerEvent('pointerup'));
      await flushPromises();
      expect(input.element.value).toBe('0.85');
      await input.trigger('keydown', { key: 'ArrowDown' });
      expect(input.element.value).toBe('0.75');
      input.element.focus();
      await input.trigger('wheel', { deltaY: 100 });
      expect(input.element.value).toBe('0.75');
      await input.setValue('2');
      await input.trigger('blur');
      expect(increase.attributes('disabled')).toBeDefined();
      await input.setValue('');
      await input.trigger('blur');
      expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([null]);
      expect(input.element.value).toBe('');
      await wrapper.setProps({ disabled: true });
      expect(input.attributes('disabled')).toBeDefined();
      expect(increase.attributes('disabled')).toBeDefined();
    } finally {
      wrapper.unmount();
    }
  });
});
