// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { nextTick, reactive } from 'vue';
import { beforeEach, expect, it, vi } from 'vitest';
import App from './App.vue';

const state = vi.hoisted(() => ({
  app: { settings: { theme: 'light' } },
  ai: { open: false },
}));
vi.mock('@/stores/app-store', () => ({ useAppStore: () => state.app }));
vi.mock('@/stores/ai-store', () => ({ useAiStore: () => state.ai }));
vi.mock('@/layouts/md-app-shell.vue', () => ({ default: { template: '<main />' } }));

beforeEach(() => {
  state.app = reactive({ settings: { theme: 'light' } });
  state.ai = reactive({ open: false });
});

it.each(['light', 'dark', 'system'])('keeps one bottom-right toaster when AI visibility changes (%s)', async theme => {
  state.app.settings.theme = theme;
  const wrapper = mount(App, {
    global: {
      stubs: {
        TooltipProvider: { template: '<div><slot /></div>' },
        Toaster: { name: 'ToastProbe', props: ['position', 'theme'], template: '<aside />' },
      },
    },
  });
  const toaster = wrapper.getComponent({ name: 'ToastProbe' });
  for (const open of [false, true, false]) {
    state.ai.open = open;
    await nextTick();
    expect(wrapper.findAllComponents({ name: 'ToastProbe' })).toHaveLength(1);
    expect(toaster.props('position')).toBe('bottom-right');
    expect(toaster.props('theme')).toBe(theme);
  }
  wrapper.unmount();
});
