// @vitest-environment jsdom
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { defineComponent, h, KeepAlive, ref } from 'vue';
import { expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import { i18n } from '@/i18n';
import type { AiContext, AiDelta, AiSubject } from '@/lib/models/ai';
import { useAiStore } from '@/stores/ai-store';
import fixtures from '../../../tests/fixtures/ai-context-v2.json';
import MdAiWorkspace from './md-ai-workspace.vue';
import MdAiReasoning from '@/components/custom/md-ai-reasoning.vue';

const streams = vi.hoisted(
  () =>
    new Map<
      string,
      {
        delta: (value: AiDelta) => void;
        finish: () => void;
        fail: (error: string) => void;
      }
    >()
);
vi.mock('@/lib/services/ai-service', () => ({
  AiService: {
    settings: async () => ({
      schemaVersion: 1,
      endpoint: 'http://localhost/v1',
      model: 'test',
      hasKey: false,
      reasoning: 'default',
    }),
  },
  AiSession: class {
    module = '';
    run(context: AiContext, _language: string, delta: (value: AiDelta) => void) {
      this.module = context.subject.module;
      return new Promise<void>((finish, fail) => streams.set(this.module, { delta, finish, fail }));
    }
    async cancel() {
      streams.get(this.module)?.fail('cancelled');
    }
  },
}));

it.each([true, false])('retains independent responses across navigation (keepAlive=%s)', async keepAlive => {
  streams.clear();
  const module = ref<AiSubject['module']>('startup');
  const pinia = createPinia();
  const wrapper = mount(
    defineComponent({
      setup: () => () =>
        h(TooltipProvider, {}, () => {
          const panel = () => h(MdAiWorkspace, { key: module.value, module: module.value });
          return keepAlive ? h(KeepAlive, {}, panel) : panel();
        }),
    }),
    { global: { plugins: [pinia, i18n], stubs: { MdAiSettingsDialog: true } } }
  );
  const store = useAiStore(pinia);
  const contexts = Object.fromEntries((fixtures as AiContext[]).map(context => [context.subject.module, context]));
  try {
    const first = store.show(contexts.startup!, 'en-US');
    await flushPromises();
    streams.get('startup')!.delta({ kind: 'text', text: 'Startup answer' });
    module.value = 'systemOptimization';
    await flushPromises();
    expect(wrapper.get('[role="region"]').isVisible()).toBe(false);
    expect(store.workspaces.startup.status).toBe('generating');
    const second = store.show(contexts.systemOptimization!, 'en-US');
    await flushPromises();
    streams.get('systemOptimization')!.delta({ kind: 'text', text: 'Optimization answer' });
    streams.get('startup')!.delta({ kind: 'text', text: ' from background' });
    await flushPromises();
    expect(wrapper.findAll('[role="region"]')).toHaveLength(1);
    expect(wrapper.get('[role="region"]').text()).toContain('Optimization answer');
    expect(wrapper.get('[role="region"]').text()).not.toContain('Startup answer');
    streams.get('startup')!.finish();
    await first;
    module.value = 'startup';
    await flushPromises();
    expect(wrapper.get('[role="region"]').text()).toContain('Startup answer from background');
    store.minimize('startup');
    module.value = 'systemOptimization';
    await flushPromises();
    expect(wrapper.get('[role="region"]').isVisible()).toBe(true);
    streams.get('systemOptimization')!.finish();
    await second;
    module.value = 'startup';
    await flushPromises();
    expect(wrapper.get('[role="region"]').isVisible()).toBe(false);
    expect(wrapper.find('.floating-panel-launcher').exists()).toBe(true);
    store.restore('startup');
    await flushPromises();
    expect(wrapper.get('[role="region"]').text()).toContain('Startup answer from background');
    expect(store.workspaces.systemOptimization.text).toBe('Optimization answer');
  } finally {
    for (const stream of streams.values()) stream.finish();
    wrapper.unmount();
  }
});

it.each(['completed', 'cancelled', 'failed'] as const)(
  'keeps answer progress in the footer and removes it when %s',
  async outcome => {
    streams.clear();
    const pinia = createPinia();
    const wrapper = mount(TooltipProvider, {
      slots: { default: () => h(MdAiWorkspace, { module: 'startup' }) },
      global: { plugins: [pinia, i18n], stubs: { MdAiSettingsDialog: true } },
    });
    const store = useAiStore(pinia);
    const context = (fixtures as AiContext[]).find(item => item.subject.module === 'startup')!;
    const request = store.show(context, 'en-US');
    try {
      await flushPromises();
      const body = wrapper.get('.overflow-y-auto');
      const footer = wrapper.get('footer');
      expect(body.get('[role="status"]').text()).toBe(i18n.global.t('ai.waiting'));
      expect(footer.find('[role="status"]').exists()).toBe(false);

      streams.get('startup')!.delta({ kind: 'reasoning', text: 'Checking the supplied startup metadata.' });
      await flushPromises();
      expect(wrapper.getComponent(MdAiReasoning).props('active')).toBe(true);
      expect(body.text()).not.toContain(i18n.global.t('ai.waiting'));

      streams.get('startup')!.delta({ kind: 'text', text: '**Docker Desktop** starts the container environment.' });
      await flushPromises();
      expect(wrapper.getComponent(MdAiReasoning).props('active')).toBe(false);
      expect(body.text()).not.toContain(i18n.global.t('ai.generating'));
      expect(body.get('.ai-response').text()).toContain('Docker Desktop');
      expect(footer.get('[role="status"]').text()).toBe(i18n.global.t('ai.generating'));

      store.minimize('startup');
      await flushPromises();
      streams.get('startup')!.delta({ kind: 'text', text: ' More detail.' });
      store.restore('startup');
      await flushPromises();
      expect(body.get('.ai-response').text()).toContain('More detail.');
      expect(footer.get('[role="status"]').text()).toBe(i18n.global.t('ai.generating'));

      if (outcome === 'completed') streams.get('startup')!.finish();
      else streams.get('startup')!.fail(outcome === 'cancelled' ? 'cancelled' : 'connectionFailed');
      await request;
      await flushPromises();
      expect(store.workspaces.startup.status).toBe(outcome);
      expect(footer.find('[role="status"]').exists()).toBe(false);
      expect(footer.text()).toContain(i18n.global.t('ai.disclaimer'));
      expect(body.get('.ai-response').text()).toContain('Docker Desktop');
    } finally {
      streams.get('startup')?.finish();
      await request;
      wrapper.unmount();
    }
  }
);
