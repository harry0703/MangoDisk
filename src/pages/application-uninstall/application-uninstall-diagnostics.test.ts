// @vitest-environment happy-dom

import { flushPromises, mount, shallowMount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { toast } from 'vue-sonner';

import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import { ApplicationService } from '@/lib/services/application-service';
import type { CommandError } from '@/lib/utils/error';

import type { ApplicationUninstallCandidate } from '@/lib/models/application';
import en from '@/locales/en-US.json';
import ja from '@/locales/ja-JP.json';
import zh from '@/locales/zh-CN.json';
import tw from '@/locales/zh-TW.json';

import MdApplicationUninstallRow from './components/md-application-uninstall-row.vue';
import ApplicationUninstallPage from './index.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));
vi.mock('vue-sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));
afterEach(() => vi.restoreAllMocks());

const candidate: ApplicationUninstallCandidate = {
  applicationId: 'application-0123456789abcdef01234567',
  primaryIdentifier: 'private-registration',
  sourceIdentities: [],
  name: 'Example',
  version: null,
  publisher: 'Example',
  estimatedBytes: 0,
  lastUsedAtMs: null,
  installedAtMs: null,
  platform: 'windowsRegistry',
  installerKind: null,
  executionMode: null,
  capability: 'viewOnly',
  recordState: 'installed',
  uninstallDiagnostic: 'executableAccessDenied',
  applicationPath: null,
  possibleRelatedPaths: [],
  iconPath: null,
  runningProcesses: [],
  totalBytes: 0,
  defaultSelectedBytes: 0,
  associatedDataComplete: false,
  components: [],
};

const messages = { 'en-US': en, 'ja-JP': ja, 'zh-CN': zh, 'zh-TW': tw };

function render(locale: keyof typeof messages, overrides: Partial<ApplicationUninstallCandidate> = {}) {
  return mount(MdApplicationUninstallRow, {
    props: {
      candidate: { ...candidate, ...overrides },
      selected: false,
      selectedComponentIds: [],
      expanded: true,
      busy: false,
      uninstallEnabled: true,
    },
    global: {
      plugins: [createI18n({ legacy: false, locale, messages })],
      stubs: { MdApplicationIcon: true, MdIcon: true, MdIconAction: true },
    },
  });
}

describe('simple uninstall actions', () => {
  it.each(Object.keys(messages) as (keyof typeof messages)[])(
    'keeps technical diagnostics out of the interface in %s',
    async locale => {
      const wrapper = render(locale);
      const text = messages[locale].applicationUninstall;
      expect(wrapper.text()).not.toContain(candidate.applicationId);
      expect(wrapper.text()).not.toContain('executableAccessDenied');
      expect(wrapper.findAll('.application-record-actions')).toHaveLength(1);
      const button = wrapper.findAll('button').find(item => item.text() === text.openWindowsInstalledApps)!;
      await button.trigger('click');
      expect(wrapper.emitted('openWindowsSettings')).toHaveLength(1);
      expect(wrapper.emitted('uninstall')).toBeUndefined();
      const remove = wrapper.get(`.application-actions md-icon-action-stub[label="${text.removeRecord}"]`);
      await remove.trigger('click');
      expect(wrapper.emitted('removeRecord')).toHaveLength(1);
      await wrapper.setProps({ busy: true });
      expect(button.attributes('disabled')).toBeDefined();
      wrapper.unmount();
    }
  );

  it('keeps orphan details and folder navigation independent of record removal', async () => {
    const wrapper = render('en-US', {
      recordState: 'orphanedRegistration',
      uninstallDiagnostic: 'executableMissing',
      possibleRelatedPaths: ['C:\\Users\\fixture\\AppData\\shared-folder'],
    });
    expect(wrapper.find('.application-details').exists()).toBe(true);
    expect(wrapper.find('.application-expand').exists()).toBe(true);
    expect(wrapper.get('.application-disclosure').attributes('disabled')).toBeUndefined();
    expect(wrapper.text()).toContain('shared-folder');
    expect(wrapper.text()).not.toContain(candidate.applicationId);
    expect(wrapper.find('[role="checkbox"]').attributes('disabled')).toBeDefined();
    expect(wrapper.text()).not.toContain(en.applicationUninstall.openWindowsInstalledApps);
    const button = wrapper.get(
      `.application-actions md-icon-action-stub[label="${en.applicationUninstall.removeRecord}"]`
    );
    await button.trigger('click');
    expect(wrapper.emitted('removeRecord')).toHaveLength(1);
    expect(wrapper.emitted('toggleExpanded')).toBeUndefined();
    expect(wrapper.emitted('uninstall')).toBeUndefined();
    await wrapper.get('.possible-related-location md-icon-action-stub').trigger('click');
    expect(wrapper.emitted('open')).toEqual([['C:\\Users\\fixture\\AppData\\shared-folder']]);
    expect(wrapper.emitted('removeRecord')).toHaveLength(1);
    await wrapper.setProps({ expanded: false });
    expect(wrapper.find('.application-details').exists()).toBe(false);
    await wrapper.get('.application-disclosure').trigger('click');
    expect(wrapper.emitted('toggleExpanded')).toHaveLength(1);
    await wrapper.get('.application-main').trigger('click');
    expect(wrapper.emitted('toggleExpanded')).toHaveLength(2);
    await wrapper.get('.application-expand').trigger('click');
    expect(wrapper.emitted('toggleExpanded')).toHaveLength(3);
    await wrapper.setProps({ expanded: true });
    expect(wrapper.text()).toContain('shared-folder');
    await wrapper.setProps({ busy: true });
    expect(button.attributes('disabled')).toBeDefined();
    wrapper.unmount();
  });

  it('shows no diagnostic banner for an ordinary uninstallable application', () => {
    const wrapper = render('en-US', { capability: 'ready', uninstallDiagnostic: null });
    expect(wrapper.text()).not.toContain(candidate.applicationId);
    expect(wrapper.find('.application-record-actions').exists()).toBe(false);
    wrapper.unmount();
  });
});

describe('explicit record removal', () => {
  async function confirmRemoval(failure?: CommandError) {
    vi.mocked(toast.error).mockClear();
    vi.mocked(toast.info).mockClear();
    vi.mocked(toast.success).mockClear();
    const remove = vi.spyOn(ApplicationService, 'removeRecord');
    if (failure) remove.mockRejectedValueOnce(failure);
    else remove.mockResolvedValueOnce(undefined);
    const wrapper = shallowMount(ApplicationUninstallPage, {
      props: {
        catalog: {
          schemaVersion: 10,
          scannedAtMs: 0,
          supported: true,
          executionSupported: true,
          catalogActionable: true,
          inventoryComplete: true,
          catalogRevision: null,
          candidates: [{ ...candidate, name: 'A very long application name '.repeat(20) }],
          readyCount: 0,
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
        plugins: [createI18n({ legacy: false, locale: 'en-US', messages })],
        renderStubDefaultSlot: true,
      },
    });
    wrapper.getComponent(MdApplicationUninstallRow).vm.$emit('removeRecord');
    await flushPromises();
    const dialog = wrapper.getComponent(MdDestructiveActionDialog);
    expect(dialog.props('title')).toBe(en.applicationUninstall.removeRecordTitle);
    expect(dialog.props('summaryLabel')).toContain('A very long application name');
    dialog.vm.$emit('confirm');
    await flushPromises();
    // Explicit removal depends only on the selected source ID, even without a catalog revision.
    expect(remove).toHaveBeenCalledExactlyOnceWith(candidate.applicationId);
    return { wrapper, dialog };
  }

  it('closes the dialog and refreshes after successful removal', async () => {
    const { wrapper, dialog } = await confirmRemoval();
    expect(dialog.props('open')).toBe(false);
    expect(wrapper.emitted('scan')).toHaveLength(1);
    expect(toast.success).toHaveBeenCalledWith(en.applicationUninstall.removeRecordSuccess);
    wrapper.unmount();
  });

  it('quietly closes the dialog when elevation is cancelled', async () => {
    const { wrapper, dialog } = await confirmRemoval({ code: 'operationCancelled', retryable: false, details: {} });
    expect(dialog.props('open')).toBe(false);
    expect(toast.error).not.toHaveBeenCalled();
    expect(wrapper.emitted('scan')).toBeUndefined();
    wrapper.unmount();
  });

  it.each<CommandError['details']>([{ mutationState: 'mayHaveChanged' }, { reason: 'itemChanged' }])(
    'refreshes uncertain or changed records instead of retrying a stale entry: %j',
    async details => {
      const { wrapper, dialog } = await confirmRemoval({ code: 'operationFailed', retryable: true, details });
      expect(dialog.props('open')).toBe(false);
      expect(wrapper.emitted('scan')).toHaveLength(1);
      expect(toast.error).not.toHaveBeenCalled();
      expect(toast.info).toHaveBeenCalledWith(en.applicationUninstall.removeRecordRefreshing);
      wrapper.unmount();
    }
  );

  it('explains a permission failure without claiming the record was removed', async () => {
    const { wrapper, dialog } = await confirmRemoval({ code: 'permissionDenied', retryable: false, details: {} });
    expect(dialog.props('open')).toBe(true);
    expect(toast.error).toHaveBeenCalledWith(en.applicationUninstall.removeRecordPermissionDenied);
    expect(wrapper.emitted('scan')).toBeUndefined();
    wrapper.unmount();
  });
});
