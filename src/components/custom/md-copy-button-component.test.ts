// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { afterEach, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import MdCopyButton from './md-copy-button.vue';

const mocks = vi.hoisted(() => ({ writeText: vi.fn(), error: vi.fn(), warn: vi.fn() }));
vi.mock('@/lib/services/shell/clipboard-service', () => ({ ClipboardService: { writeText: mocks.writeText } }));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: mocks.warn } }));
vi.mock('vue-sonner', () => ({ toast: { error: mocks.error } }));
function mountButton(text = 'Visible answer', showText = false) {
  return mount(MdCopyButton, {
    props: { text, showText },
    global: {
      plugins: [i18n],
      stubs: {
        MdIconAction: {
          props: ['label', 'disabled'],
          emits: ['click'],
          template: '<button :aria-label="label" :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
        },
      },
    },
  });
}
afterEach(() => {
  vi.useRealTimers();
  vi.resetAllMocks();
});

it('copies the current text and restores the copy icon after one second', async () => {
  vi.useFakeTimers();
  mocks.writeText.mockResolvedValue(undefined);
  const wrapper = mountButton();
  await wrapper.get('button').trigger('click');
  await flushPromises();
  expect(mocks.writeText).toHaveBeenCalledWith('Visible answer');
  expect(wrapper.get('button').attributes('aria-label')).toBe(i18n.global.t('common.copied'));
  await wrapper.setProps({ text: 'Visible answer continued' });
  await vi.advanceTimersByTimeAsync(999);
  expect(wrapper.get('button').attributes('aria-label')).toBe(i18n.global.t('common.copied'));
  await vi.advanceTimersByTimeAsync(1);
  expect(wrapper.get('button').attributes('aria-label')).toBe(i18n.global.t('common.copy'));
  wrapper.unmount();
});

it('copies complete long addresses and restores text feedback after one second', async () => {
  vi.useFakeTimers();
  let finish!: () => void;
  mocks.writeText.mockImplementation(() => new Promise<void>(resolve => (finish = resolve)));
  const addresses = ['2001:db8:1234:5678:90ab:cdef:1234:5678', '2001:db8:abcd:ef01:2345:6789:abcd:ef01'].join(', ');
  const wrapper = mountButton(addresses, true);
  await wrapper.setProps({ truncate: true });
  const button = wrapper.get('button');
  expect(button.text()).toBe(addresses);
  expect(wrapper.find('svg').exists()).toBe(false);
  expect(button.attributes('title')).toBeUndefined();

  await button.trigger('click');
  await button.trigger('click');
  expect(mocks.writeText).toHaveBeenCalledOnce();
  expect(mocks.writeText).toHaveBeenCalledWith(addresses);
  expect(button.attributes('disabled')).toBeDefined();
  finish();
  await flushPromises();
  expect(button.classes()).toContain('copied');
  expect(button.attributes('aria-label')).toBe(`${i18n.global.t('common.copied')}: ${addresses}`);
  expect(wrapper.find('svg').exists()).toBe(false);
  await vi.advanceTimersByTimeAsync(1000);
  expect(button.classes()).not.toContain('copied');
  expect(button.attributes('aria-label')).toBe(`${i18n.global.t('common.copy')}: ${addresses}`);
  wrapper.unmount();
});

it('does not report success when clipboard access fails or text is empty', async () => {
  mocks.writeText.mockRejectedValue(new Error('private error'));
  const wrapper = mountButton('');
  expect(wrapper.get('button').attributes('disabled')).toBeDefined();
  await wrapper.setProps({ text: 'Answer' });
  await wrapper.get('button').trigger('click');
  await flushPromises();
  expect(wrapper.get('button').attributes('aria-label')).toBe(i18n.global.t('common.copy'));
  expect(mocks.warn).toHaveBeenCalledWith('clipboard', 'write_failed');
  expect(mocks.error).toHaveBeenCalledWith(i18n.global.t('common.copyFailed'));
  wrapper.unmount();
});

it('does not create feedback timers when copying completes after unmount', async () => {
  vi.useFakeTimers();
  let resolve!: () => void;
  mocks.writeText.mockImplementation(
    () =>
      new Promise<void>(r => {
        resolve = r;
      })
  );
  const wrapper = mountButton();
  await wrapper.get('button').trigger('click');
  expect(wrapper.get('button').attributes('disabled')).toBeDefined();
  wrapper.unmount();
  resolve();
  await flushPromises();
  expect(vi.getTimerCount()).toBe(0);
});
