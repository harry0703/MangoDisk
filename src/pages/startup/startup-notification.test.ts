// @vitest-environment happy-dom
import { shallowMount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { toast } from 'vue-sonner';
import type {
  StartupArtifact,
  StartupCatalog,
  StartupChangeFailureReason,
  StartupChangeResult,
} from '@/lib/models/startup';
import en from '@/locales/en-US.json';
import ja from '@/locales/ja-JP.json';
import pt from '@/locales/pt-BR.json';
import tr from '@/locales/tr-TR.json';
import ko from '@/locales/ko-KR.json';
import ru from '@/locales/ru-RU.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import StartupPage from './index.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));
vi.mock('@/stores/ai-store', () => ({ useAiStore: () => ({ dismissModule: vi.fn() }) }));
vi.mock('vue-sonner', () => ({ toast: { success: vi.fn(), warning: vi.fn() } }));
afterEach(() => vi.clearAllMocks());
const messages = {
  'zh-CN': zh,
  'zh-TW': tw,
  'en-US': en,
  'ja-JP': ja,
  'ko-KR': ko,
  'tr-TR': tr,
  'pt-BR': pt,
  'ru-RU': ru,
};

function artifact(overrides: Partial<StartupArtifact> = {}): StartupArtifact {
  return {
    itemId: 'a'.repeat(64),
    sourceId: 'macos.launchd.user_agents',
    sourceKind: 'launchAgent',
    scope: 'currentUser',
    triggers: ['userLogon'],
    displayName: 'Fixture',
    configurationPath: null,
    target: { kind: 'executable', path: null, executableName: null, arguments: [] },
    ownerName: 'Fixture',
    publisher: null,
    summary: null,
    summarySource: 'sourceLabel',
    version: null,
    iconPath: null,
    identityConfidence: 'strong',
    configuredState: 'enabled',
    runtimeState: 'unknown',
    controlCapability: 'toggleable',
    trust: 'unknown',
    modifiedAtMs: null,
    diagnostics: [],
    removalSupported: false,
    removableOrphan: false,
    ...overrides,
  };
}

function result(reason: StartupChangeFailureReason, service: boolean): StartupChangeResult {
  const item = artifact(service ? { sourceKind: 'service', sourceId: 'windows.services' } : {});
  const catalog: StartupCatalog = {
    schemaVersion: 1,
    scanId: 'scan',
    catalogRevision: 'revision',
    scannedAtMs: 0,
    complete: true,
    artifacts: [item],
    groups: [],
    coverage: [],
    elapsedMs: 0,
    summary: {
      itemCount: 1,
      groupCount: 0,
      enabledCount: 1,
      disabledCount: 0,
      unknownStateCount: 0,
      elevationRequiredCount: 0,
      systemItemCount: 0,
    },
  };
  return {
    planId: 'plan',
    changedCount: 0,
    failedCount: 1,
    catalog,
    items: [
      { itemId: item.itemId, status: 'failed', configuredState: 'enabled', verified: false, failureReason: reason },
    ],
  };
}

function render(locale: keyof typeof messages) {
  return shallowMount(StartupPage, {
    props: {
      catalog: null,
      scanning: false,
      cancelling: false,
      preparingChange: false,
      executingChange: false,
      cancellingChange: false,
      pendingPlan: null,
      lastChangeResult: null,
    },
    global: { plugins: [createI18n({ legacy: false, locale, messages })] },
  });
}

describe.each(Object.keys(messages) as (keyof typeof messages)[])('startup failure feedback in %s', locale => {
  it('explains a service permission denial without requesting another authorization', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastChangeResult: result('permissionDenied', true) });
    expect(toast.warning).toHaveBeenLastCalledWith(expect.any(String), {
      description: messages[locale].startup.change.serviceAccessDenied,
    });
    expect(wrapper.emitted('executeChange')).toBeUndefined();
    expect(toast.success).not.toHaveBeenCalled();
    wrapper.unmount();
  });
  it('uses general permission feedback for macOS launch agents', async () => {
    const wrapper = render(locale);
    await wrapper.setProps({ lastChangeResult: result('permissionDenied', false) });
    expect(toast.warning).toHaveBeenLastCalledWith(expect.any(String), {
      description: messages[locale].startup.change.accessDenied,
    });
    wrapper.unmount();
  });
  it('does not mislabel cancellation or another failure as a permission denial', async () => {
    const wrapper = render(locale);
    for (const reason of ['userCancelled', 'platformFailure', 'verificationFailed'] as const) {
      await wrapper.setProps({ lastChangeResult: result(reason, true) });
      expect(vi.mocked(toast.warning).mock.lastCall?.[1]?.description).toBeUndefined();
    }
    wrapper.unmount();
  });
});
