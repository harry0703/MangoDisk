// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { expect, it } from 'vitest';
import { i18n } from '@/i18n';
import MdAiReasoning from './md-ai-reasoning.vue';

it('shows thinking live, collapses at the first answer and supports keyboard-accessible expansion', async () => {
  const wrapper = mount(MdAiReasoning, {
    props: { text: 'Inspecting metadata', active: true, hasAnswer: false },
    global: { plugins: [i18n] },
  });
  expect(wrapper.get('button').attributes('aria-expanded')).toBe('true');
  expect(wrapper.get('button').text()).toBe(i18n.global.t('ai.thinking'));
  expect(wrapper.get('button').attributes('aria-label')).toBe(i18n.global.t('ai.thinking'));
  await wrapper.setProps({ active: false, hasAnswer: true });
  expect(wrapper.get('button').attributes('aria-expanded')).toBe('false');
  expect(wrapper.get('button').text()).toBe(i18n.global.t('ai.reasoningDetails'));
  await wrapper.get('button').trigger('click');
  expect(wrapper.get('.reasoning-content').isVisible()).toBe(true);
  wrapper.unmount();
});

it('does not interrupt a user who is reading or selecting reasoning', async () => {
  const wrapper = mount(MdAiReasoning, {
    props: { text: 'Earlier thought', active: true, hasAnswer: false },
    global: { plugins: [i18n] },
  });
  const el = wrapper.get('.reasoning-content').element;
  Object.defineProperties(el, { scrollHeight: { value: 600 }, clientHeight: { value: 144 } });
  el.scrollTop = 80;
  await wrapper.get('.reasoning-content').trigger('scroll');
  expect(wrapper.emitted('interact')).toHaveLength(1);
  await wrapper.setProps({ text: 'Earlier thought plus new material' });
  await flushPromises();
  expect(el.scrollTop).toBe(80);
  await wrapper.setProps({ active: false, hasAnswer: true });
  expect(wrapper.get('button').attributes('aria-expanded')).toBe('true');
  wrapper.unmount();
});

it('keeps cached reasoning collapsed and renders provider markup as inert text', () => {
  const wrapper = mount(MdAiReasoning, {
    props: { text: '<img src=x onerror=alert(1)>', active: false, hasAnswer: true },
    global: { plugins: [i18n] },
  });
  expect(wrapper.get('button').attributes('aria-expanded')).toBe('false');
  expect(wrapper.find('img').exists()).toBe(false);
  expect(wrapper.get('.reasoning-content').text()).toContain('<img');
  wrapper.unmount();
});
