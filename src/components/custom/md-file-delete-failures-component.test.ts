// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import { ClipboardService } from '@/lib/services/shell/clipboard-service';
import MdFileDeleteFailures from './md-file-delete-failures.vue';

afterEach(() => {
  vi.restoreAllMocks();
  i18n.global.locale.value = 'zh-CN';
});
describe('file deletion failures', () => {
  it.each(Object.keys(i18n.global.messages.value))(
    'shows a simple summary and copies full diagnostics in %s',
    async locale => {
      i18n.global.locale.value = locale as typeof i18n.global.locale.value;
      const copy = vi.spyOn(ClipboardService, 'writeText').mockResolvedValue();
      const wrapper = mount(MdFileDeleteFailures, {
        props: {
          failures: [
            {
              path: '/fixture/program.dll',
              message: 'raw diagnostic',
              reason: 'accessDenied',
              stage: 'createStaging',
              nativeCode: 5,
              partial: false,
            },
            {
              path: '/fixture/folder',
              message: 'raw partial',
              reason: 'resourceBusy',
              stage: 'remove',
              partial: true,
              remainingRestored: true,
            },
          ],
        },
        global: { plugins: [i18n] },
      });
      expect(wrapper.text()).toContain(i18n.global.t('fileDeletion.reasons.accessDenied'));
      expect(wrapper.text()).not.toContain('raw diagnostic');
      const diagnostics = wrapper.get('details');
      (diagnostics.element as HTMLDetailsElement).open = true;
      await diagnostics.trigger('toggle');
      expect(diagnostics.text()).toContain('program.dll');
      expect(diagnostics.text()).not.toContain('raw diagnostic');
      expect(diagnostics.text()).not.toContain('createStaging');
      expect(wrapper.text()).toContain(i18n.global.t('fileDeletion.partialRestored'));
      await wrapper.get('button').trigger('click');
      await flushPromises();
      expect(copy).toHaveBeenCalledWith(expect.stringContaining('"nativeCode": 5'));
      expect(wrapper.get('[role="status"]').text()).toBe(i18n.global.t('fileDeletion.copied'));
      wrapper.unmount();
    }
  );
  it('does not claim legacy failures left files untouched and bounds expanded lists', async () => {
    const wrapper = mount(MdFileDeleteFailures, {
      props: { failures: Array.from({ length: 55 }, (_, index) => ({ path: `/fixture/${index}`, message: '' })) },
      global: { plugins: [i18n] },
    });
    expect(wrapper.findAll('li')).toHaveLength(0);
    const files = wrapper.get('details');
    (files.element as HTMLDetailsElement).open = true;
    await files.trigger('toggle');
    expect(wrapper.findAll('li')).toHaveLength(50);
    const more = wrapper.findAll('button').find(button => button.text() === i18n.global.t('fileDeletion.showMore'))!;
    await more.trigger('click');
    expect(wrapper.findAll('li')).toHaveLength(55);
    vi.spyOn(ClipboardService, 'writeText').mockRejectedValue(new Error('unavailable'));
    await wrapper.findAll('button').at(-1)!.trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe(i18n.global.t('fileDeletion.failed'));
    wrapper.unmount();
  });
});
