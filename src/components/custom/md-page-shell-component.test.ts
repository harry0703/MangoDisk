// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { defineComponent, h, ref } from 'vue';
import { expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import { i18n } from '@/i18n';
import MdFloatingPanel from './md-floating-panel.vue';
import MdPageShell from './md-page-shell.vue';

vi.mock('@/lib/services/operating-system-service', () => ({
  OperatingSystemService: { isWindows: () => false, isMacOs: () => true },
}));

it('updates content spacing with the footer without remounting the floating response', async () => {
  const footer = ref(false);
  const wrapper = mount(
    defineComponent({
      setup: () => () =>
        h(TooltipProvider, {}, () =>
          h(
            MdPageShell,
            { title: 'Page', contentMode: 'workspace' },
            {
              default: () => h('section', 'Results'),
              overlay: () => h(MdFloatingPanel, { open: true, minimized: false, title: 'AI' }, () => 'Answer'),
              ...(footer.value ? { footer: () => h('button', 'Apply') } : {}),
            }
          )
        ),
    }),
    { global: { plugins: [i18n] } }
  );
  const panel = wrapper.get('[role="region"]').element;
  expect(wrapper.find('.md-page-content-stage--with-footer').exists()).toBe(false);
  footer.value = true;
  await flushPromises();
  expect(wrapper.find('.md-page-content-stage--with-footer').exists()).toBe(true);
  expect(wrapper.get('[role="region"]').element).toBe(panel);
  expect(wrapper.get('.md-page-footer').text()).toBe('Apply');
  footer.value = false;
  await flushPromises();
  expect(wrapper.find('.md-page-content-stage--with-footer').exists()).toBe(false);
  expect(wrapper.get('[role="region"]').element).toBe(panel);
  wrapper.unmount();
});
