// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { h } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TooltipProvider } from '@/components/ui/tooltip';
import { i18n } from '@/i18n';
import MdWindowTitlebar from './md-window-titlebar.vue';

const { close, minimize, observeMaximized, stopObserving, toggleMaximize } = vi.hoisted(() => ({
  close: vi.fn(),
  minimize: vi.fn(),
  observeMaximized: vi.fn(),
  stopObserving: vi.fn(),
  toggleMaximize: vi.fn(),
}));

vi.mock('@/lib/services/application-window-service', () => ({
  ApplicationWindowService: {
    close,
    minimize,
    observeMaximized,
    toggleMaximize,
  },
}));

afterEach(() => {
  vi.clearAllMocks();
});

describe('MdWindowTitlebar', () => {
  it.each(['linux', 'windows'] as const)('provides native window controls on %s', async platform => {
    observeMaximized.mockImplementationOnce(async (onChange: (value: boolean) => void) => {
      onChange(false);
      return stopObserving;
    });
    const wrapper = mount(TooltipProvider, {
      slots: { default: () => h(MdWindowTitlebar, { platform }) },
      global: { plugins: [i18n] },
    });
    await flushPromises();

    await wrapper.get(`[aria-label="${i18n.global.t('common.minimize')}"]`).trigger('click');
    await wrapper.get(`[aria-label="${i18n.global.t('common.maximize')}"]`).trigger('click');
    await wrapper.get(`[aria-label="${i18n.global.t('common.close')}"]`).trigger('click');

    expect(observeMaximized).toHaveBeenCalledOnce();
    expect(minimize).toHaveBeenCalledOnce();
    expect(toggleMaximize).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
    wrapper.unmount();
    expect(stopObserving).toHaveBeenCalledOnce();
  });

  it('leaves window controls to the native macOS traffic lights', async () => {
    const wrapper = mount(TooltipProvider, {
      slots: { default: () => h(MdWindowTitlebar, { platform: 'macos' }) },
      global: { plugins: [i18n] },
    });
    await flushPromises();

    expect(wrapper.find('.window-controls').exists()).toBe(false);
    expect(observeMaximized).not.toHaveBeenCalled();
  });
});
