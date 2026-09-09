// @vitest-environment happy-dom

import { shallowMount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { toast } from 'vue-sonner';

import type { ApplicationUninstallBatchResult, ApplicationUninstallResult } from '@/lib/models/application';
import en from '@/locales/en-US.json';
import ja from '@/locales/ja-JP.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import ApplicationUninstallPage from './index.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));
vi.mock('vue-sonner', () => ({ toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn(), info: vi.fn() } }));
afterEach(() => vi.clearAllMocks());
const messages = { 'zh-CN': zh, 'zh-TW': tw, 'en-US': en, 'ja-JP': ja };

type Outcome = 'completed' | 'failed' | 'cancelled' | 'continuing' | 'removedWithFailure';
function result(outcomes: Outcome[], restartRequired = false): ApplicationUninstallBatchResult {
  const results: ApplicationUninstallResult[] = outcomes.map((outcome, index) => {
    const status = outcome === 'continuing' ? 'cancelled' : outcome === 'removedWithFailure' ? 'failed' : outcome;
    return {
      planId: `plan-${index}`,
      applicationId: `application-${index}`,
      applicationName: 'Example',
      expectedBytes: 0,
      previewedBytes: 0,
      releasedBytes: 0,
      previewedItemCount: 0,
      affectedItemCount: Number(status === 'completed'),
      failedItemCount: Number(status === 'failed'),
      releasedBytesIsEstimate: false,
      restartRequired: false,
      dryRun: false,
      historySaved: true,
      actions: [
        {
          componentId: 'native',
          kind: 'nativeInstaller',
          status,
          reason:
            outcome === 'continuing'
              ? 'externalUninstallerContinuing'
              : outcome === 'removedWithFailure'
                ? 'nativeInstallerFailedAfterRemoval'
                : status === 'failed'
                  ? 'nativeInstallerFailed'
                  : null,
          expectedBytes: 0,
          releasedBytes: 0,
        },
      ],
    };
  });
  const failed = results.filter(item => item.failedItemCount).length;
  const completed = results.filter(item => item.affectedItemCount).length;
  return {
    batchId: 'batch',
    expectedBytes: 0,
    previewedBytes: 0,
    releasedBytes: 0,
    selectedApplicationCount: outcomes.length,
    previewedApplicationCount: 0,
    affectedApplicationCount: completed,
    failedApplicationCount: failed,
    previewedItemCount: 0,
    affectedItemCount: completed,
    failedItemCount: failed,
    releasedBytesIsEstimate: false,
    restartRequired,
    dryRun: false,
    results,
  };
}

function render(locale: keyof typeof messages) {
  return shallowMount(ApplicationUninstallPage, {
    props: {
      catalog: null,
      scanning: false,
      cancelling: false,
      progress: null,
      executionProgress: null,
      plan: null,
      preview: null,
      lastResult: null,
      preparing: false,
      executing: false,
      cancellingExecution: false,
      cancellationRevision: 0,
      closingApplications: false,
      closeResult: null,
    },
    global: { plugins: [createI18n({ legacy: false, locale, messages })] },
  });
}

describe.each(Object.keys(messages) as (keyof typeof messages)[])('uninstall result messages in %s', locale => {
  it('uses a single clear message for confirmed cancellation and preparation cancellation', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastResult: result(['cancelled']) });
    expect(toast.info).toHaveBeenLastCalledWith(
      messages[locale].applicationUninstall.cancelledTitle,
      expect.not.objectContaining({ description: expect.anything() })
    );
    expect(toast.warning).not.toHaveBeenCalled();
    await wrapper.setProps({ cancellationRevision: 1 });
    expect(toast.info).toHaveBeenLastCalledWith(
      messages[locale].applicationUninstall.cancelledTitle,
      expect.not.objectContaining({ description: expect.anything() })
    );
    wrapper.unmount();
  });

  it('preserves failures and restart instructions when the same batch contains cancellations', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastResult: result(['completed', 'failed', 'cancelled'], true) });
    const text = messages[locale].applicationUninstall;
    const call = vi.mocked(toast.warning).mock.lastCall!;
    expect(call[0]).toBe(text.completedWithWarnings);
    for (const part of [text.resultCompleted, text.resultFailed, text.resultCancelled]) {
      expect(call[1]?.description).toContain(part.replace('{count}', '1'));
    }
    expect(call[1]?.description).toContain(text.restartRequired);
    expect(toast.info).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('reports successful and cancelled items without losing a restart request', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastResult: result(['completed', 'cancelled'], true) });
    const text = messages[locale].applicationUninstall;
    const call = vi.mocked(toast.info).mock.lastCall!;
    expect(call[0]).toBe(text.executionFinishedTitle);
    expect(call[1]?.description).toContain(text.resultCompleted.replace('{count}', '1'));
    expect(call[1]?.description).toContain(text.resultCancelled.replace('{count}', '1'));
    expect(call[1]?.description).toContain(text.restartRequired);
    wrapper.unmount();
  });

  it('keeps a useful count when an entire batch is cancelled', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastResult: result(['cancelled', 'cancelled']) });
    expect(toast.info).toHaveBeenLastCalledWith(
      messages[locale].applicationUninstall.cancelledTitle,
      expect.objectContaining({
        description: messages[locale].applicationUninstall.resultCancelled.replace('{count}', '2'),
      })
    );
    wrapper.unmount();
  });

  it('distinguishes a running external uninstaller from a confirmed cancellation', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastResult: result(['continuing']) });
    expect(toast.info).toHaveBeenLastCalledWith(
      messages[locale].applicationUninstall.executionStoppedTitle,
      expect.objectContaining({
        description: messages[locale].applicationUninstall.resultContinuing.replace('{count}', '1'),
      })
    );
    wrapper.unmount();
  });

  it('keeps the verified partial-removal explanation', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastResult: result(['removedWithFailure']) });
    expect(vi.mocked(toast.warning).mock.lastCall?.[1]?.description).toContain(
      messages[locale].history.applicationUninstallReasons.nativeInstallerFailedAfterRemoval
    );
    wrapper.unmount();
  });
});
