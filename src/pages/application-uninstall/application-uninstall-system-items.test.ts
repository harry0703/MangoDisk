// @vitest-environment happy-dom

import { flushPromises, shallowMount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Checkbox } from '@/components/ui/checkbox';
import MdResultSummary from '@/components/custom/md-result-summary.vue';
import MdResultCheckbox from '@/components/custom/md-result-checkbox.vue';
import MdSelectionActionBar from '@/components/custom/md-selection-action-bar.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import MdResultSearch from '@/components/custom/md-result-search.vue';
import MdCategoryFilter from '@/components/custom/md-category-filter.vue';
import { ApplicationService } from '@/lib/services/application-service';
import { LoggerService } from '@/lib/services/logger-service';
import type { ApplicationUninstallCandidate } from '@/lib/models/application';
import en from '@/locales/en-US.json';
import Page from './index.vue';
import Row from './components/md-application-uninstall-row.vue';
import { displayedApplications, applicationCanStartUninstall } from './application-uninstall-catalog';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));
vi.mock('vue-sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));
afterEach(() => vi.restoreAllMocks());

function candidate(id: string, systemKind: ApplicationUninstallCandidate['systemKind']): ApplicationUninstallCandidate {
  return {
    applicationId: id,
    primaryIdentifier: `private-${id}`,
    sourceIdentities: [],
    systemKind,
    name: id,
    version: null,
    publisher: 'Private publisher',
    estimatedBytes: 100,
    lastUsedAtMs: null,
    installedAtMs: null,
    platform: 'windowsRegistry',
    installerKind: 'windowsAppx',
    executionMode: 'silent',
    capability: 'ready',
    recordState: 'installed',
    uninstallDiagnostic: null,
    applicationPath: null,
    possibleRelatedPaths: [],
    iconPath: null,
    runningProcesses: [],
    totalBytes: 100,
    defaultSelectedBytes: 100,
    associatedDataComplete: false,
    components: [
      {
        componentId: `${id}-installer`,
        kind: 'nativeInstaller',
        risk: 'required',
        path: null,
        bytes: 100,
        fileCount: 1,
        defaultSelected: true,
      },
    ],
  };
}
const regular = candidate('regular', 'unclassified');
const system = candidate('system', 'windowsBuiltinApp');
const orphan = {
  ...candidate('orphan', 'sharedRuntime'),
  capability: 'viewOnly' as const,
  recordState: 'orphanedRegistration' as const,
  components: [],
};

function render() {
  vi.spyOn(LoggerService, 'info').mockImplementation(() => {});
  return shallowMount(Page, {
    props: {
      catalog: {
        schemaVersion: 11,
        scannedAtMs: 123,
        supported: true,
        executionSupported: true,
        catalogActionable: true,
        inventoryComplete: true,
        catalogRevision: 'private-revision',
        candidates: [regular, system, orphan],
        readyCount: 2,
        blockedCount: 1,
        hiddenCount: 0,
        relatedDirectoryCount: 0,
        relatedPathScanElapsedMs: 0,
        elapsedMs: 0,
      },
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
    global: {
      plugins: [createI18n({ legacy: false, locale: 'en-US', messages: { 'en-US': en } })],
      renderStubDefaultSlot: true,
      stubs: {
        MdPageShell: { template: '<div><slot/><slot name="footer"/></div>' },
        MdResultWorkspace: { template: '<div><slot name="summary"/><slot name="header"/><slot/></div>' },
        MdResultSummary: false,
        MdResultTable: { methods: { scrollTo() {} }, template: '<div><slot name="header"/><slot/></div>' },
        MdResultFilterToolbar: { template: '<div><slot/><slot name="aside"/></div>' },
      },
    },
  });
}

async function showSystem(wrapper: ReturnType<typeof render>, show: boolean) {
  wrapper.getComponent(Checkbox).vm.$emit('update:modelValue', show);
  await flushPromises();
}

describe('system application visibility', () => {
  it('hides only positive Windows classifications and preserves explicit uninstall capability', () => {
    expect(displayedApplications([regular, system, orphan], false)).toEqual([regular]);
    expect(displayedApplications([regular, system, orphan], true)).toEqual([regular, system, orphan]);
    expect(displayedApplications([{ ...system, platform: 'macosBundle' }], false)).toHaveLength(1);
    expect(applicationCanStartUninstall(system)).toBe(true);
    const codec = { ...system, systemKind: 'windowsSharedPackage' as const };
    expect(displayedApplications([codec], false)).toHaveLength(0);
    expect(displayedApplications([codec], true)).toEqual([codec]);
  });

  it('keeps summary, status counts, search and select-all within the chosen scope', async () => {
    const wrapper = render();
    expect(wrapper.findAllComponents(Row)).toHaveLength(1);
    expect(wrapper.getComponent(MdResultSummary).props('title')).toContain('1');
    wrapper.getComponent(MdResultCheckbox).vm.$emit('update:checked', true);
    await flushPromises();
    expect(wrapper.getComponent(MdSelectionActionBar).props('selectedValue')).toBe('1');
    await showSystem(wrapper, true);
    expect(wrapper.findAllComponents(Row)).toHaveLength(3);
    expect(wrapper.getComponent(MdResultSummary).props('title')).toContain('3');
    const options = wrapper.getComponent(MdCategoryFilter).props('options');
    expect(options.find((option: { value: string }) => option.value === 'ready')?.count).toBe(2);
    wrapper.getComponent(MdResultSearch).vm.$emit('update:modelValue', 'system');
    await flushPromises();
    expect(wrapper.findAllComponents(Row)).toHaveLength(1);
    await showSystem(wrapper, false);
    expect(wrapper.findAllComponents(Row)).toHaveLength(0);
    wrapper.unmount();
  });

  it('removes hidden applications and components from the next explicit batch', async () => {
    const wrapper = render();
    await showSystem(wrapper, true);
    wrapper.getComponent(MdResultCheckbox).vm.$emit('update:checked', true);
    await flushPromises();
    expect(wrapper.getComponent(MdSelectionActionBar).props('selectedValue')).toBe('2');
    await showSystem(wrapper, false);
    expect(wrapper.getComponent(MdSelectionActionBar).props('selectedValue')).toBe('1');
    wrapper.getComponent(MdSelectionActionBar).vm.$emit('action');
    await flushPromises();
    expect(wrapper.emitted('prepare')).toEqual([
      [[{ applicationId: regular.applicationId, componentIds: ['regular-installer'] }]],
    ]);
    expect(wrapper.getComponent(Checkbox).props('disabled')).toBe(true);
    const events = vi
      .mocked(LoggerService.info)
      .mock.calls.map(call => call[1])
      .join('\n');
    expect(events).toContain('hidden_system_count=2');
    expect(events).toContain('deselected_count=1');
    expect(events).not.toContain('private-');
    expect(events).not.toContain('Private publisher');
    wrapper.unmount();
  });

  it('keeps explicit system record removal independent of classification and catalog revision', async () => {
    const remove = vi.spyOn(ApplicationService, 'removeRecord').mockResolvedValue(undefined);
    const wrapper = render();
    await showSystem(wrapper, true);
    const row = wrapper
      .findAllComponents(Row)
      .find(row => row.props('candidate').applicationId === orphan.applicationId)!;
    row.vm.$emit('removeRecord');
    await flushPromises();
    wrapper.getComponent(MdDestructiveActionDialog).vm.$emit('confirm');
    await flushPromises();
    expect(remove).toHaveBeenCalledExactlyOnceWith(orphan.applicationId);
    expect(wrapper.emitted('scan')).toBeUndefined();
    expect(wrapper.emitted('recordRemoved')).toEqual([[orphan.applicationId]]);
    wrapper.unmount();
  });
});
