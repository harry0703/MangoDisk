// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { beforeEach, expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import MdAiAction from '@/layouts/components/md-ai-action.vue';
import MdAiWorkspace from '@/layouts/components/md-ai-workspace.vue';
import { i18n } from '@/i18n';
import { AiService } from '@/lib/services/ai-service';
import { useAiStore } from '@/stores/ai-store';
import MdAiFeatureToggle from './md-ai-feature-toggle.vue';

vi.mock('@/lib/services/ai-service', () => ({
  AiService: { preferences: vi.fn(), setEnabled: vi.fn() },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(AiService.setEnabled).mockImplementation(async enabled => ({ schemaVersion: 1, enabled }));
});

it.each(['zh-CN', 'zh-TW', 'en-US', 'ja-JP', 'ko-KR', 'tr-TR', 'pt-BR', 'ru-RU'] as const)(
  'keeps only the toggle and removes AI controls, panels and launchers in %s',
  async locale => {
    const previousLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const pinia = createPinia();
    const store = useAiStore(pinia);
    store.$patch({ enabled: true, preferencesLoaded: true });
    const wrapper = mount(
      {
        components: { TooltipProvider, MdAiFeatureToggle, MdAiAction, MdAiWorkspace },
        template:
          '<TooltipProvider><MdAiFeatureToggle /><MdAiAction name="Fixture" /><MdAiWorkspace module="cleanup" /></TooltipProvider>',
      },
      { global: { plugins: [pinia, i18n], stubs: { MdAiSettingsDialog: true } } }
    );
    try {
      store.workspaces.cleanup.open = true;
      store.workspaces.cleanup.minimized = true;
      await flushPromises();
      expect(wrapper.find('.md-ai-action').exists()).toBe(true);
      expect(wrapper.find('.floating-panel-launcher').exists()).toBe(true);
      const toggle = wrapper.getComponent(MdAiFeatureToggle);
      expect(toggle.get('.setting-copy strong').text()).toBe(i18n.global.t('ai.providerTitle'));
      expect(toggle.get('#ai-enabled-hint').text()).toBe(i18n.global.t('ai.settingsDescription'));
      expect(toggle.get('#ai-enabled').attributes('aria-label')).toBe(i18n.global.t('ai.enableFeature'));
      await toggle.get('.setting-copy').trigger('click');
      expect(toggle.emitted('configure')).toBeUndefined();
      expect(AiService.setEnabled).not.toHaveBeenCalled();
      expect(toggle.get('button[aria-haspopup="dialog"]').text()).toBe(i18n.global.t('ai.configureAction'));
      await toggle.get('button[aria-haspopup="dialog"]').trigger('click');
      expect(toggle.emitted('configure')).toHaveLength(1);
      expect(AiService.setEnabled).not.toHaveBeenCalled();
      expect(store.enabled).toBe(true);
      await wrapper.get('#ai-enabled').trigger('click');
      await flushPromises();
      expect(wrapper.get('#ai-enabled').attributes('aria-checked')).toBe('false');
      expect(toggle.get('.setting-copy strong').text()).toBe(i18n.global.t('ai.providerTitle'));
      expect(toggle.get('#ai-enabled-hint').text()).toBe(i18n.global.t('ai.settingsDescription'));
      expect(toggle.get('#ai-enabled').attributes('aria-label')).toBe(i18n.global.t('ai.enableFeature'));
      expect(toggle.find('button[aria-haspopup="dialog"]').exists()).toBe(false);
      expect(toggle.emitted('configure')).toHaveLength(1);
      expect(wrapper.find('.md-ai-action').exists()).toBe(false);
      expect(wrapper.find('.floating-panel-launcher').exists()).toBe(false);
      expect(wrapper.find('[role="region"]').exists()).toBe(false);
      expect(wrapper.findComponent({ name: 'MdAiSettingsDialog' }).exists()).toBe(false);
      await wrapper.get('#ai-enabled').trigger('click');
      await flushPromises();
      expect(wrapper.find('.md-ai-action').exists()).toBe(true);
      expect(toggle.find('button[aria-haspopup="dialog"]').exists()).toBe(true);
      expect(toggle.get('#ai-enabled-hint').text()).toBe(i18n.global.t('ai.settingsDescription'));
      expect(store.open).toBe(false);
      expect(wrapper.get('.floating-panel-launcher').isVisible()).toBe(false);
    } finally {
      wrapper.unmount();
      i18n.global.locale.value = previousLocale;
    }
  }
);

it('keeps the confirmed switch state during saving and offers read retry only when needed', async () => {
  const pinia = createPinia();
  const store = useAiStore(pinia);
  const wrapper = mount(MdAiFeatureToggle, { global: { plugins: [pinia, i18n] } });
  try {
    expect(wrapper.get('#ai-enabled').attributes('disabled')).toBeDefined();
    vi.mocked(AiService.preferences).mockRejectedValueOnce('configurationUnavailable');
    await store.loadPreferences();
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain(i18n.global.t('ai.featureLoadFailed'));
    vi.mocked(AiService.preferences).mockResolvedValue({ schemaVersion: 1, enabled: true });
    await wrapper.get('[role="alert"] button').trigger('click');
    await flushPromises();
    let fail!: (reason: string) => void;
    vi.mocked(AiService.setEnabled).mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          fail = reject;
        })
    );
    await wrapper.get('#ai-enabled').trigger('click');
    expect(wrapper.get('#ai-enabled').attributes('aria-checked')).toBe('true');
    expect(wrapper.get('#ai-enabled').attributes('disabled')).toBeDefined();
    fail('configurationUnavailable');
    await flushPromises();
    expect(wrapper.get('#ai-enabled').attributes('aria-checked')).toBe('true');
    expect(wrapper.get('#ai-enabled').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('[role="alert"]').text()).toContain(i18n.global.t('ai.featureSaveFailed'));
    await wrapper.get('#ai-enabled').trigger('click');
    await flushPromises();
    expect(wrapper.get('#ai-enabled').attributes('aria-checked')).toBe('false');
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  } finally {
    wrapper.unmount();
  }
});
