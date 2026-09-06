// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import MdAiSettingsDialog from './md-ai-settings-dialog.vue';
import MdSpinner from './md-spinner.vue';

const mocks = vi.hoisted(() => ({
  configuration: vi.fn().mockResolvedValue(null),
  save: vi.fn(),
  delete: vi.fn(),
  run: vi.fn(),
  success: vi.fn(),
}));
vi.mock('vue-sonner', () => ({ toast: { success: mocks.success } }));
vi.mock('@/lib/services/ai-service', () => ({
  AiService: mocks,
  AiSession: class {
    run = mocks.run;
    async cancel() {}
  },
}));
afterEach(() => {
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

describe('AI configuration dialog', () => {
  it('opens, masks the key and saves the entered configuration', async () => {
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: false },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await wrapper.setProps({ open: true });
    await flushPromises();
    const dialog = document.querySelector('[role="dialog"]');
    expect(dialog).not.toBeNull();
    expect(document.querySelector('#ai-key')?.getAttribute('type')).toBe('password');
    expect(mocks.configuration).toHaveBeenCalledTimes(1);
    const configuration = {
      endpoint: 'https://api.example.com/v1',
      model: 'example-model',
      apiKey: 'synthetic-test-key',
      reasoning: 'default',
    };
    let finishSave!: () => void;
    mocks.save.mockImplementationOnce(
      () =>
        new Promise<void>(resolve => {
          finishSave = resolve;
        })
    );
    for (const [id, value] of Object.entries({
      'ai-endpoint': configuration.endpoint,
      'ai-model': configuration.model,
      'ai-key': configuration.apiKey,
    })) {
      const input = document.getElementById(id) as HTMLInputElement;
      input.value = value;
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }
    await flushPromises();
    const save = [...document.querySelectorAll('button')].find(
      button => button.textContent?.trim() === i18n.global.t('ai.save')
    );
    expect(save).toBeDefined();
    save!.click();
    await flushPromises();
    expect(save!.disabled).toBe(true);
    expect(wrapper.findComponent(MdSpinner).exists()).toBe(false);
    expect(
      [...document.querySelectorAll('button')].some(
        button => button.textContent?.trim() === i18n.global.t('common.close')
      )
    ).toBe(true);
    save!.click();
    expect(mocks.save).toHaveBeenCalledTimes(1);
    finishSave();
    await flushPromises();
    expect(mocks.save).toHaveBeenCalledWith(configuration);
    expect(wrapper.emitted('configured')).toHaveLength(1);
    expect(mocks.success).toHaveBeenCalledWith(i18n.global.t('ai.saved'));
    expect(dialog?.textContent).not.toContain(i18n.global.t('ai.saved'));
    expect((document.getElementById('ai-key') as HTMLInputElement).value).toBe(configuration.apiKey);
    expect(wrapper.emitted('update:open')).toBeUndefined();
    wrapper.unmount();
  });

  it('reports connection success only after the test, keeps failures inline and notifies deletion', async () => {
    mocks.configuration.mockResolvedValueOnce({
      schemaVersion: 1,
      endpoint: 'http://example.com/v1',
      model: 'fixture',
      apiKey: 'synthetic-key',
      reasoning: 'default',
    });
    let finish!: () => void;
    mocks.run.mockImplementationOnce(
      () =>
        new Promise<void>(resolve => {
          finish = resolve;
        })
    );
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    const button = (label: string) =>
      [...document.querySelectorAll('button')].find(button => button.textContent?.trim() === i18n.global.t(label))!;
    try {
      button('ai.saveAndTest').click();
      await flushPromises();
      expect(mocks.success).not.toHaveBeenCalled();
      expect(wrapper.findComponent(MdSpinner).exists()).toBe(true);
      finish();
      await flushPromises();
      expect(mocks.success).toHaveBeenLastCalledWith(i18n.global.t('ai.connected'));
      expect(document.querySelector('[role="dialog"]')?.textContent).not.toContain(i18n.global.t('ai.connected'));
      mocks.success.mockClear();
      mocks.run.mockRejectedValueOnce('unauthorized');
      button('ai.saveAndTest').click();
      await flushPromises();
      expect(mocks.success).not.toHaveBeenCalled();
      expect(document.querySelector('[role="alert"]')?.textContent).toBe(i18n.global.t('ai.errors.unauthorized'));
      button('ai.deleteConfiguration').click();
      await flushPromises();
      expect(mocks.success).toHaveBeenLastCalledWith(i18n.global.t('ai.deleted'));
      expect(document.querySelector('[role="alert"]')).toBeNull();
      expect(document.getElementById('ai-reasoning')?.textContent).toContain(i18n.global.t('ai.reasoningDefault'));
    } finally {
      wrapper.unmount();
    }
  });

  it.each(['default', 'disabled'])(
    'preserves saved reasoning mode %s and masks the key when opened',
    async reasoning => {
      mocks.configuration.mockResolvedValueOnce({
        schemaVersion: 1,
        endpoint: 'http://example.com/v1',
        model: 'fixture',
        apiKey: 'synthetic-key',
        reasoning,
      });
      const wrapper = mount(MdAiSettingsDialog, {
        props: { open: true },
        attachTo: document.body,
        global: { plugins: [i18n] },
      });
      await flushPromises();
      const input = document.getElementById('ai-key') as HTMLInputElement;
      expect(document.getElementById('ai-reasoning')?.textContent).toContain(
        i18n.global.t(reasoning === 'default' ? 'ai.reasoningDefault' : 'ai.reasoningDisabled')
      );
      expect(input.value).toBe('synthetic-key');
      expect(input.type).toBe('password');
      (document.querySelector(`[aria-label="${i18n.global.t('ai.showKey')}"]`) as HTMLButtonElement).click();
      await flushPromises();
      expect(input.type).toBe('text');
      (document.querySelector(`[aria-label="${i18n.global.t('ai.hideKey')}"]`) as HTMLButtonElement).click();
      await flushPromises();
      expect(input.type).toBe('password');
      await wrapper.setProps({ open: false });
      expect(input.value).toBe('');
      wrapper.unmount();
    }
  );

  it('does not restore a secret after the editor closes during loading', async () => {
    let resolve!: (value: unknown) => void;
    mocks.configuration.mockImplementationOnce(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await wrapper.setProps({ open: false });
    resolve({
      schemaVersion: 1,
      endpoint: 'http://example.com/v1',
      model: 'fixture',
      apiKey: 'late-secret',
      reasoning: 'default',
    });
    await flushPromises();
    expect((document.getElementById('ai-key') as HTMLInputElement | null)?.value ?? '').toBe('');
    wrapper.unmount();
  });
});
