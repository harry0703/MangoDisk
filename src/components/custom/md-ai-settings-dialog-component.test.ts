// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import MdAiSettingsDialog from './md-ai-settings-dialog.vue';
import MdSpinner from './md-spinner.vue';
import type { AiQuota } from '@/lib/models/ai';
import { formatAiQuotaResetAt } from '@/lib/utils/ai-quota';

const mocks = vi.hoisted(() => {
  const reads = {
    settings: vi.fn().mockResolvedValue({ freeAvailable: false }),
    configuration: vi.fn().mockResolvedValue(null),
  };
  return {
    ...reads,
    editorState: vi.fn(async () => ({
      configuration: await reads.configuration(),
      freeAvailable: (await reads.settings()).freeAvailable,
    })),
    save: vi.fn(),
    delete: vi.fn(),
    run: vi.fn(),
    success: vi.fn(),
    error: vi.fn(),
    openLink: vi.fn().mockResolvedValue(undefined),
  };
});
vi.mock('vue-sonner', () => ({ toast: { success: mocks.success, error: mocks.error } }));
vi.mock('@/lib/services/link-service', () => ({ LinkService: { open: mocks.openLink } }));
vi.mock('@/lib/services/ai-service', () => ({
  AiService: mocks,
  AiSession: class {
    run = mocks.run;
    async cancel() {}
  },
}));
function modeRadio(value: 'free' | 'custom'): HTMLInputElement {
  return document.querySelector(`input[name="ai-service-mode"][value="${value}"]`)!;
}
afterEach(() => {
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

describe('AI configuration dialog', () => {
  it('opens help without selecting custom mode, saving, closing or losing unsaved fields', async () => {
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    try {
      await flushPromises();
      const link = document.querySelector('a[href$="/docs/ai#custom-service"]') as HTMLAnchorElement;
      expect(link.closest('label')).toBeNull();
      link.click();
      await flushPromises();
      expect(modeRadio('free').checked).toBe(true);
      expect(mocks.openLink).toHaveBeenLastCalledWith(link.href);
      modeRadio('custom').click();
      await flushPromises();
      const input = document.querySelector('#ai-model') as HTMLInputElement;
      input.value = 'unsaved-model';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      link.click();
      await flushPromises();
      expect(modeRadio('custom').checked).toBe(true);
      expect(input.value).toBe('unsaved-model');
      expect(mocks.save).not.toHaveBeenCalled();
      expect(mocks.run).not.toHaveBeenCalled();
      expect(wrapper.emitted('update:open')).toBeUndefined();
      mocks.openLink.mockRejectedValueOnce(new Error('native opener unavailable'));
      link.click();
      await flushPromises();
      expect(mocks.error).toHaveBeenCalledWith(i18n.global.t('ai.guideOpenFailed'));
      expect(input.value).toBe('unsaved-model');
      expect(document.querySelector('[role="alert"]')).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });

  it('renders quota independently of opening and only requests updates for the selected free service', async () => {
    mocks.settings.mockResolvedValueOnce({ freeAvailable: true });
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    try {
      await flushPromises();
      const dialog = () => document.querySelector('[role="dialog"]')!;
      expect(dialog()).not.toBeNull();
      expect(dialog().textContent).toContain(i18n.global.t('ai.freeDailyAllowance'));
      expect(wrapper.findComponent(MdSpinner).exists()).toBe(false);
      expect(wrapper.emitted('refreshQuota')).toEqual([[i18n.global.locale.value, false]]);
      const quota: AiQuota = {
        available: true,
        unavailableReason: null,
        remaining: 13,
        dailyLimit: 20,
        cooldownSeconds: 60,
        nextAllowedAt: '2026-09-07T00:01:00Z',
        resetAt: '2026-09-08T00:00:00Z',
        serverTime: '2026-09-07T00:00:00Z',
        activeRequests: 0,
        maxConcurrentRequests: 2,
        policyVersion: 'test',
        promptVersion: 'test',
      };
      await wrapper.setProps({ quota });
      const remaining = i18n.global.t('ai.freeRemaining', { remaining: 13, limit: 20 });
      expect(dialog().textContent).toContain(remaining);
      expect(dialog().textContent).not.toContain(i18n.global.t('ai.freeDailyAllowance'));
      await wrapper.setProps({ quota: { ...quota, remaining: 3, dailyLimit: 10, cooldownSeconds: 120 } });
      expect(dialog().textContent).toContain(i18n.global.t('ai.freeRemaining', { remaining: 3, limit: 10 }));
      expect(dialog().textContent).not.toContain(remaining);
      expect(dialog().querySelector('[role="status"]')?.parentElement?.children).toHaveLength(1);
      await wrapper.setProps({ quota });
      modeRadio('custom').click();
      await flushPromises();
      window.dispatchEvent(new Event('focus'));
      await flushPromises();
      expect(wrapper.emitted('refreshQuota')).toHaveLength(1);
      expect(dialog().textContent).not.toContain(remaining);
      modeRadio('free').click();
      await flushPromises();
      expect(wrapper.emitted('refreshQuota')).toHaveLength(2);
      expect(dialog().textContent).toContain(remaining);
      // The owner retains the last successful snapshot when a refresh fails.
      window.dispatchEvent(new Event('focus'));
      await flushPromises();
      expect(wrapper.emitted('refreshQuota')?.at(-1)).toEqual([i18n.global.locale.value, true]);
      expect(dialog().textContent).toContain(remaining);
      await wrapper.setProps({ quota: { ...quota, remaining: 0 } });
      expect(dialog().textContent).toContain(
        i18n.global.t('ai.freeResetsAt', {
          time: formatAiQuotaResetAt(
            quota.resetAt,
            i18n.global.locale.value,
            Intl.DateTimeFormat().resolvedOptions().timeZone
          ),
        })
      );
      await wrapper.setProps({ quota: { ...quota, unavailableReason: 'AI_SERVICE_DISABLED' } });
      expect(dialog().textContent).toContain(i18n.global.t('ai.freeTemporarilyUnavailable'));
      expect(dialog().textContent).toContain(i18n.global.t('ai.freeUseCustom'));
      expect(dialog().textContent).not.toContain(remaining);
      await wrapper.setProps({ quota: { ...quota, unavailableReason: 'AI_RATE_LIMITED' } });
      expect(dialog().textContent).not.toContain(i18n.global.t('ai.freeTemporarilyUnavailable'));
      await wrapper.setProps({ quota: { ...quota, remaining: 20 } });
      expect(dialog().textContent).toContain(i18n.global.t('ai.freeRemaining', { remaining: 20, limit: 20 }));
      await wrapper.setProps({ open: false });
      window.dispatchEvent(new Event('focus'));
      await flushPromises();
      expect(wrapper.emitted('refreshQuota')).toHaveLength(3);
    } finally {
      wrapper.unmount();
    }
  });

  it('keeps advanced settings collapsed, validates overrides and restores provider defaults', async () => {
    mocks.configuration.mockResolvedValueOnce({
      mode: 'custom',
      endpoint: 'https://example.com/v1',
      model: 'fixture',
      apiKey: 'synthetic-key',
      reasoning: 'disabled',
      temperature: 0.7,
      maxTokens: 4096,
    });
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    const advanced = document.querySelector('[aria-controls="ai-advanced-settings"]') as HTMLButtonElement;
    const save = [...document.querySelectorAll('button')].find(
      button => button.textContent?.trim() === i18n.global.t('ai.save')
    )!;
    expect(document.querySelector('#ai-temperature')).toBeNull();
    advanced.click();
    await flushPromises();
    const temperature = document.querySelector('#ai-temperature') as HTMLInputElement;
    const maxTokens = document.querySelector('#ai-max-tokens') as HTMLInputElement;
    expect(temperature.value).toBe('0.7');
    expect(maxTokens.value).toBe('4096');
    const input = async (element: HTMLInputElement, value: string) => {
      element.value = value;
      element.dispatchEvent(new Event('input', { bubbles: true }));
      element.dispatchEvent(new FocusEvent('blur'));
      await flushPromises();
    };
    await input(temperature, '3');
    expect(temperature.value).toBe('2');
    expect(save.disabled).toBe(false);
    await input(temperature, '0');
    await input(maxTokens, '1.5');
    expect(maxTokens.value).toBe('2');
    expect(save.disabled).toBe(false);
    await input(maxTokens, '2048');
    advanced.click();
    await flushPromises();
    save.click();
    await flushPromises();
    expect(mocks.save).toHaveBeenLastCalledWith(
      expect.objectContaining({ temperature: 0, maxTokens: 2048, reasoning: 'disabled' })
    );
    advanced.click();
    await flushPromises();
    await input(document.querySelector('#ai-temperature')!, '');
    await input(document.querySelector('#ai-max-tokens')!, '');
    save.click();
    await flushPromises();
    expect(mocks.save).toHaveBeenLastCalledWith(expect.objectContaining({ temperature: null, maxTokens: null }));
    wrapper.unmount();
  });

  it('keeps free mode simple and retains custom fields when switching back', async () => {
    mocks.settings.mockResolvedValueOnce({ freeAvailable: true });
    mocks.configuration.mockResolvedValueOnce({
      schemaVersion: 2,
      mode: 'custom',
      freeConsent: false,
      endpoint: 'https://example.com/v1',
      model: 'fixture',
      apiKey: 'synthetic-key',
      reasoning: 'default',
    });
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    const button = (label: string) =>
      [...document.querySelectorAll('button')].find(item => item.textContent?.trim() === i18n.global.t(label))!;
    expect(modeRadio('custom').checked).toBe(true);
    expect(document.querySelector('[role="dialog"]')?.textContent).not.toContain(
      i18n.global.t('ai.freeDailyAllowance')
    );
    modeRadio('free').click();
    await flushPromises();
    expect(mocks.editorState).toHaveBeenCalledTimes(1);
    expect(modeRadio('free').checked).toBe(true);
    expect(modeRadio('custom').checked).toBe(false);
    expect(mocks.save).not.toHaveBeenCalled();
    expect(document.querySelector('#ai-key')).toBeNull();
    expect(button('ai.saveAndTest')).toBeUndefined();
    for (const label of ['ai.freeNoKey', 'ai.freeDailyAllowance']) {
      expect(i18n.global.te(label)).toBe(true);
    }
    for (const label of ['ai.freeNoKey', 'ai.freeDailyAllowance']) {
      expect(document.querySelector('[role="dialog"]')?.textContent).toContain(i18n.global.t(label));
    }
    expect(document.querySelector('[role="dialog"]')?.textContent).not.toContain(i18n.global.t('ai.freeDisclosure'));
    expect(document.querySelector('[role="dialog"]')?.textContent).not.toContain(
      i18n.global.t('ai.freeBuildUnavailable')
    );
    button('ai.save').click();
    await flushPromises();
    expect(mocks.run).not.toHaveBeenCalled();
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({ mode: 'free', freeConsent: false, apiKey: 'synthetic-key' })
    );
    modeRadio('custom').click();
    await flushPromises();
    expect((document.querySelector('#ai-key') as HTMLInputElement).value).toBe('synthetic-key');
    expect(document.querySelector('#ai-key')?.getAttribute('type')).toBe('password');
    wrapper.unmount();
  });

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
    modeRadio('custom').click();
    await flushPromises();
    expect(document.querySelector('#ai-key')?.getAttribute('type')).toBe('password');
    expect(mocks.configuration).toHaveBeenCalledTimes(1);
    const configuration = {
      mode: 'custom',
      freeConsent: false,
      endpoint: 'https://api.example.com/v1',
      model: 'example-model',
      apiKey: 'synthetic-test-key',
      reasoning: 'default',
      temperature: null,
      maxTokens: null,
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
      schemaVersion: 2,
      mode: 'custom',
      freeConsent: false,
      freeAvailable: false,
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
      expect(document.getElementById('ai-reasoning')).toBeNull();
      expect(modeRadio('free').checked).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });

  it.each(['default', 'disabled'])(
    'preserves saved reasoning mode %s and masks the key when opened',
    async reasoning => {
      mocks.configuration.mockResolvedValueOnce({
        schemaVersion: 2,
        mode: 'custom',
        freeConsent: false,
        freeAvailable: false,
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
      expect(document.getElementById('ai-reasoning')).toBeNull();
      (document.querySelector('[aria-controls="ai-advanced-settings"]') as HTMLButtonElement).click();
      await flushPromises();
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
    await flushPromises();
    // The entrance must never start on an undersized loading shell.
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.querySelector('input[type="radio"]')).toBeNull();
    await wrapper.setProps({ open: false });
    resolve({
      schemaVersion: 2,
      mode: 'custom',
      freeConsent: false,
      freeAvailable: false,
      endpoint: 'http://example.com/v1',
      model: 'fixture',
      apiKey: 'late-secret',
      reasoning: 'default',
    });
    await flushPromises();
    expect((document.getElementById('ai-key') as HTMLInputElement | null)?.value ?? '').toBe('');
    wrapper.unmount();
  });

  it('does not allow a failed read to overwrite the existing configuration', async () => {
    mocks.editorState.mockRejectedValueOnce('configurationUnavailable');
    const wrapper = mount(MdAiSettingsDialog, {
      props: { open: true },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe(
      i18n.global.t('ai.errors.configurationUnavailable')
    );
    const save = [...document.querySelectorAll('button')].find(
      button => button.textContent?.trim() === i18n.global.t('ai.save')
    )!;
    expect(save.disabled).toBe(true);
    expect(document.querySelector('input[type="radio"]')).toBeNull();
    save.click();
    expect(mocks.save).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});
