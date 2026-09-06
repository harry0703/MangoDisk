// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { defineComponent, h, ref } from 'vue';
import { afterEach, expect, it } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import { i18n } from '@/i18n';
import MdFloatingPanel from './md-floating-panel.vue';

afterEach(() => {
  document.body.innerHTML = '';
});

it('is non-modal and supports minimize, restore and close without hiding the page', async () => {
  const minimized = ref(false);
  const open = ref(true);
  const wrapper = mount(
    defineComponent({
      setup: () => () =>
        h(TooltipProvider, {}, () =>
          h(
            MdFloatingPanel,
            {
              open: open.value,
              minimized: minimized.value,
              onMinimize: () => {
                minimized.value = true;
              },
              onRestore: () => {
                minimized.value = false;
              },
              onClose: () => {
                open.value = false;
              },
              title: 'AI',
              subtitle: 'Cache',
            },
            () => 'Streaming answer'
          )
        ),
    }),
    { attachTo: document.body, global: { plugins: [i18n] } }
  );
  await flushPromises();
  const panel = wrapper.findComponent(MdFloatingPanel);
  const region = document.querySelector('[role="region"]')!;
  expect(region.textContent).toContain('Streaming answer');
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(region.getAttribute('aria-modal')).toBeNull();
  (document.querySelector('button[aria-label="' + i18n.global.t('ai.minimize') + '"]') as HTMLButtonElement).click();
  expect(panel.emitted('minimize')).toHaveLength(1);
  await flushPromises();
  expect((document.querySelector('[role="region"]') as HTMLElement).style.display).toBe('none');
  expect(region.hasAttribute('inert')).toBe(true);
  expect(region.getAttribute('aria-hidden')).toBe('true');
  (document.querySelector('button[aria-label="' + i18n.global.t('ai.restore') + '"]') as HTMLButtonElement).click();
  await flushPromises();
  expect(panel.emitted('restore')).toHaveLength(1);
  expect(document.querySelector('[role="region"]')?.textContent).toContain('Streaming answer');
  expect(document.querySelector('[role="region"]')).toBe(region);
  expect(region.hasAttribute('inert')).toBe(false);
  expect(document.querySelector('.floating-panel-launcher')?.hasAttribute('inert')).toBe(true);
  (document.querySelector('button[aria-label="' + i18n.global.t('common.close') + '"]') as HTMLButtonElement).click();
  expect(panel.emitted('close')).toHaveLength(1);
  await flushPromises();
  expect((region as HTMLElement).style.display).toBe('none');
  expect(region.hasAttribute('inert')).toBe(true);
  open.value = true;
  await flushPromises();
  expect(document.querySelector('[role="region"]')).toBe(region);
  expect(region.textContent).toContain('Streaming answer');
  wrapper.unmount();
});
