// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { afterEach, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import MdCopyButton from './md-copy-button.vue';

const mocks = vi.hoisted(() => ({ writeText: vi.fn(), error: vi.fn(), warn: vi.fn() }));
vi.mock('@/lib/services/clipboard-service', () => ({ ClipboardService: { writeText: mocks.writeText } }));
vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: mocks.warn } }));
vi.mock('vue-sonner', () => ({ toast: { error: mocks.error } }));
function mountButton(text = 'Visible answer') {
  return mount(MdCopyButton, {
    props: { text },
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
