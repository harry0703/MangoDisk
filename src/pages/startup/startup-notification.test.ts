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
} from '@/lib/models/startup/startup';
import en from '@/locales/en-US.json';
import ja from '@/locales/ja-JP.json';
import pt from '@/locales/pt-BR.json';
import tr from '@/locales/tr-TR.json';
import ko from '@/locales/ko-KR.json';
import ru from '@/locales/ru-RU.json';
import uk from '@/locales/uk-UA.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';
import StartupPage from './index.vue';
import StartupRow from './components/md-startup-row.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));
vi.mock('@/stores/ai-store', () => ({ useAiStore: () => ({ dismissModule: vi.fn() }) }));
vi.mock('vue-sonner', () => ({ toast: { success: vi.fn(), warning: vi.fn() } }));
afterEach(() => {
  vi.clearAllMocks();
  vi.useRealTimers();
});
const messages = {
  'zh-CN': zh,
  'zh-TW': tw,
  'en-US': en,
  'ja-JP': ja,
  'ko-KR': ko,
  'tr-TR': tr,
  'pt-BR': pt,
  'ru-RU': ru,
  'uk-UA': uk,
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

function render(locale: keyof typeof messages, catalog: StartupCatalog | null = null) {
  return shallowMount(StartupPage, {
    props: {
      catalog,
      scanning: false,
      cancelling: false,
      preparingChange: false,
      executingChange: false,
      cancellingChange: false,
      pendingPlan: null,
      lastChangeResult: null,
    },
    global: { plugins: [createI18n({ legacy: false, locale, messages })], renderStubDefaultSlot: true },
  });
}

describe.each(Object.keys(messages) as (keyof typeof messages)[])('startup failure feedback in %s', locale => {
  it.each(['enabled', 'disabled'] as const)('uses the updated row for one verified %s change', async desiredState => {
    vi.useFakeTimers();
    const catalog = result('platformFailure', false).catalog!;
    const item = catalog.artifacts[0]!;
    item.configuredState = desiredState === 'enabled' ? 'disabled' : 'enabled';
    catalog.groups = [
      {
        groupId: 'fixture',
        name: 'Fixture',
        publisher: null,
        summary: null,
        summarySource: 'sourceLabel',
        version: null,
        iconPath: null,
        identityConfidence: 'strong',
        itemIds: [item.itemId],
        sourceKinds: ['launchAgent'],
        triggers: ['userLogon'],
        scopes: ['currentUser'],
        configuredState: desiredState === 'enabled' ? 'allDisabled' : 'allEnabled',
        controlState: 'allToggleable',
        systemItem: false,
      },
    ];
    const wrapper = render(locale, catalog);
    wrapper.getComponent(StartupRow).vm.$emit('toggle-artifact', item);
    await vi.advanceTimersByTimeAsync(350);
    expect(wrapper.emitted('prepareChange')).toHaveLength(1);
    const updated = structuredClone(catalog);
    updated.artifacts[0]!.configuredState = desiredState;
    updated.groups[0]!.configuredState = desiredState === 'enabled' ? 'allEnabled' : 'allDisabled';
    await wrapper.setProps({
      catalog: updated,
      lastChangeResult: {
        planId: 'plan',
        changedCount: 1,
        failedCount: 0,
        catalog: updated,
        items: [
          {
            itemId: item.itemId,
            status: 'changed',
            configuredState: desiredState,
            verified: true,
            failureReason: null,
          },
        ],
      },
    });
    expect(wrapper.getComponent(StartupRow).props('state')).toBe(desiredState);
    expect(wrapper.getComponent(StartupRow).props('busy')).toBe(false);
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.warning).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('retains result feedback when there is no visible toggle confirming the outcome', async () => {
    const wrapper = render(locale);
    const completed = result('platformFailure', false);
    completed.failedCount = 0;
    completed.changedCount = 1;
    completed.items[0] = {
      ...completed.items[0]!,
      status: 'changed',
      configuredState: 'disabled',
      verified: true,
      failureReason: null,
    };
    await wrapper.setProps({ lastChangeResult: completed });
    expect(toast.success).toHaveBeenCalledOnce();
    wrapper.unmount();
  });
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
