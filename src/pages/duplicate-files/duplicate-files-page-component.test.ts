// @vitest-environment happy-dom

import { flushPromises, shallowMount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { i18n } from '@/i18n';
import {
  DUPLICATE_ENTRY_DELETE_POLICIES,
  DUPLICATE_GROUP_KINDS,
  DUPLICATE_KEEPER_RULE_IDS,
  type DuplicateFilesResult,
} from '@/lib/models/storage/duplicate-file';
import { STORAGE_SCOPE_IDS, type StorageScopePreferences } from '@/lib/models/storage/storage-scope';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { useDuplicateFilesStore } from '@/stores/duplicate-files-store';
import { useAiStore } from '@/stores/ai-store';
import { Button } from '@/components/ui/button';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';

import MdStorageScopeSelect from '@/components/custom/md-storage-scope-select.vue';
import MdScanExclusionLink from '@/components/custom/md-scan-exclusion-link.vue';
import MdDuplicateFileGroups from './components/md-duplicate-file-groups.vue';
import MdDuplicateSmartSelectButton from './components/md-duplicate-smart-select-button.vue';
import DuplicateFilesPage from './index.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));

const result: DuplicateFilesResult = {
  scanId: 7,
  roots: ['/scan'],
  protectedRoots: [],
  scannedAtMs: 1,
  scannedFileCount: 2,
  skippedCount: 0,
  duplicateFileCount: 2,
  totalDuplicateBytes: 2048,
  reclaimableBytes: 1024,
  totalGroupCount: 1,
  returnedGroupCount: 1,
  truncated: false,
  groups: [
    {
      id: 'group-1',
      hash: 'hash-1',
      kind: DUPLICATE_GROUP_KINDS.file,
      bytesPerFile: 1024,
      fileCountPerEntry: 1,
      reclaimableBytes: 1024,
      entries: ['/scan/work/report.pdf', '/scan/chat/report.pdf'].map(path => ({
        name: 'report.pdf',
        path,
        parentPath: path.slice(0, path.lastIndexOf('/')),
        bytes: 1024,
        allocatedBytes: 1024,
        modifiedAtMs: 1,
        deletePolicy: DUPLICATE_ENTRY_DELETE_POLICIES.cleanable,
      })),
    },
  ],
};

function mountPage(pageResult: DuplicateFilesResult = result) {
  return shallowMount(DuplicateFilesPage, {
    props: {
      disk: null,
      disks: [],
      result: pageResult,
      resultComplete: true,
      hasMore: false,
      loadingMore: false,
      progress: null,
      busy: false,
      cancelling: false,
      deleting: false,
      minimumBytes: 200 * 1024,
      keeperRule: DUPLICATE_KEEPER_RULE_IDS.shortestPath,
    },
    global: {
      plugins: [i18n],
      stubs: {
        MdPageShell: {
          template: '<div><slot name="actions" /><slot name="footer" /><slot /></div>',
        },
        MdResultWorkspace: {
          template: '<div><slot name="summary" /><slot name="header" /><slot /></div>',
        },
        MdResultSummary: { template: '<div><slot name="status" /><slot name="actions" /></div>' },
        MdResultFilterToolbar: { template: '<div><slot /></div>' },
        MdDelayedOperationWorkspace: { template: '<div><slot /></div>' },
      },
    },
  });
}

beforeEach(() => {
  const pinia = createPinia();
  setActivePinia(pinia);
  useStorageScopeStore().$patch({
    selectedPaths: { [STORAGE_SCOPE_IDS.duplicateFiles]: ['/scan'] },
    duplicateFileProtectedPaths: [],
  });
  vi.spyOn(FolderSelectionService, 'resolveSelectedDirectories').mockImplementation(async paths =>
    paths.map(path => ({ requestedPath: path, path }))
  );
  vi.spyOn(PreferenceStorageService, 'saveStorageScopePreferences').mockResolvedValue();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('duplicate files page', () => {
  it('resolves aliases only on scan and retains protection when targets converge', async () => {
    useStorageScanPreferencesStore().initialized = true;
    const scope = useStorageScopeStore();
    const alias = 'C:\\Mac\\Home\\Documents';
    const target = '\\\\psf\\Home\\Documents';
    const savedPreferences: StorageScopePreferences[] = [];
    vi.mocked(PreferenceStorageService.saveStorageScopePreferences).mockImplementation(async preferences => {
      savedPreferences.push(JSON.parse(JSON.stringify(preferences)) as StorageScopePreferences);
    });
    scope.$patch({ selectedPaths: { 'duplicate-files': [alias, target] }, duplicateFileProtectedPaths: [alias] });
    const resolve = vi.mocked(FolderSelectionService.resolveSelectedDirectories);
    resolve.mockResolvedValueOnce([
      { requestedPath: alias, path: target },
      { requestedPath: target, path: target },
    ]);
    const wrapper = mountPage();
    try {
      await flushPromises();
      expect(resolve).not.toHaveBeenCalled();
      await wrapper.getComponent(Button).trigger('click');
      await flushPromises();
      expect(wrapper.emitted('find')).toEqual([[[{ path: target, mode: 'protected' }]]]);
      expect(scope.selectedPaths['duplicate-files']).toEqual([target]);
      expect(scope.duplicateFileProtectedPaths).toEqual([target]);
      expect(savedPreferences.length).toBeGreaterThan(0);
      for (const saved of savedPreferences) {
        expect(saved.selectedPaths['duplicate-files']).toEqual([target]);
        expect(saved.duplicateFileProtectedPaths).toEqual([target]);
      }
    } finally {
      wrapper.unmount();
    }
  });

  it('keeps saved selection and protection when explicit access is rejected', async () => {
    useStorageScanPreferencesStore().initialized = true;
    const scope = useStorageScopeStore();
    scope.duplicateFileProtectedPaths = ['/scan'];
    const wrapper = mountPage();
    const error = { code: 'invalidInput', details: { reason: 'folderUnavailable' } };
    vi.mocked(FolderSelectionService.resolveSelectedDirectories).mockRejectedValueOnce(error);
    try {
      await wrapper.getComponent(Button).trigger('click');
      await flushPromises();
      expect(wrapper.emitted('find')).toBeUndefined();
      expect(wrapper.emitted('error')).toEqual([[error]]);
      expect(scope.selectedPaths['duplicate-files']).toEqual(['/scan']);
      expect(scope.duplicateFileProtectedPaths).toEqual(['/scan']);
    } finally {
      wrapper.unmount();
    }
  });

  it('does not start scanning after cancelling pending directory validation', async () => {
    useStorageScanPreferencesStore().initialized = true;
    let finish!: (value: Array<{ requestedPath: string; path: string }>) => void;
    vi.mocked(FolderSelectionService.resolveSelectedDirectories).mockImplementationOnce(
      () =>
        new Promise(resolve => {
          finish = resolve;
        })
    );
    const wrapper = mountPage();
    try {
      await wrapper.getComponent(Button).trigger('click');
      wrapper.getComponent(MdOperationProgress).vm.$emit('cancel');
      finish([{ requestedPath: '/scan', path: '/scan' }]);
      await flushPromises();
      expect(wrapper.emitted('find')).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it('explains the clicked copy without changing selections and invalidates it on a rescan', async () => {
    const ai = useAiStore();
    const show = vi.spyOn(ai, 'show').mockResolvedValue();
    const dismiss = vi.spyOn(ai, 'dismissModule').mockImplementation(() => {});
    const wrapper = mountPage();
    const group = result.groups[0]!;
    const target = group.entries[1]!;
    wrapper.getComponent(MdDuplicateFileGroups).vm.$emit('explain', group, target);
    await flushPromises();
    expect(show).toHaveBeenCalledWith(
      expect.objectContaining({
        subject: expect.objectContaining({
          module: 'duplicateFiles',
          target: expect.objectContaining({
            file: expect.objectContaining({ path: target.path }),
          }),
        }),
      }),
      i18n.global.locale.value
    );
    expect(wrapper.getComponent(MdDuplicateFileGroups).props('selectedPaths')).toEqual([]);
    await wrapper.setProps({ busy: true });
    expect(dismiss).toHaveBeenCalledWith('duplicateFiles');
    wrapper.unmount();
  });

  it('shows the exclusion link only for a completed scan that used exclusions', async () => {
    const scanStore = useDuplicateFilesStore();
    const wrapper = mountPage();
    expect(wrapper.findComponent(MdScanExclusionLink).exists()).toBe(false);

    scanStore.resultExcludedFolders = ['/other/private'];
    await flushPromises();
    expect(wrapper.findComponent(MdScanExclusionLink).exists()).toBe(false);

    scanStore.resultExcludedFolders = ['/scan/private'];
    await flushPromises();
    const link = wrapper.getComponent(MdScanExclusionLink);
    link.vm.$emit('open');
    expect(wrapper.emitted('openExclusions')).toHaveLength(1);

    await wrapper.setProps({ resultComplete: false });
    expect(wrapper.findComponent(MdScanExclusionLink).exists()).toBe(false);
  });

  it('invalidates destructive actions when protection changes after the scan', async () => {
    const ai = useAiStore();
    const show = vi.spyOn(ai, 'show').mockResolvedValue();
    const dismiss = vi.spyOn(ai, 'dismissModule').mockImplementation(() => {});
    const wrapper = mountPage();
    const groups = wrapper.getComponent(MdDuplicateFileGroups);
    groups.vm.$emit('update:selectedPaths', ['/scan/chat/report.pdf']);
    await flushPromises();
    expect(wrapper.getComponent(MdDuplicateSmartSelectButton).props('selectedCount')).toBe(1);
    expect(groups.props('deleteDisabled')).toBe(false);

    wrapper.getComponent(MdStorageScopeSelect).vm.$emit('update:protectedPaths', ['/scan']);
    await flushPromises();

    expect(groups.props('selectedPaths')).toEqual([]);
    expect(groups.props('selectionDisabled')).toBe(true);
    expect(groups.props('deleteDisabled')).toBe(true);
    expect(wrapper.getComponent(MdDuplicateSmartSelectButton).props('selectedCount')).toBe(0);
    expect(wrapper.getComponent(MdDuplicateSmartSelectButton).props('disabled')).toBe(true);
    expect(wrapper.emitted('delete')).toBeUndefined();
    expect(dismiss).toHaveBeenCalledWith('duplicateFiles');
    groups.vm.$emit('explain', result.groups[0]!, result.groups[0]!.entries[0]!);
    await flushPromises();
    expect(show).not.toHaveBeenCalled();
  });

  it('preserves an explicitly empty protected scope instead of restoring the previous scan', () => {
    const wrapper = mountPage({ ...result, protectedRoots: ['/scan'] });

    expect(wrapper.getComponent(MdStorageScopeSelect).props('protectedPaths')).toEqual([]);
    expect(wrapper.getComponent(MdDuplicateFileGroups).props('deleteDisabled')).toBe(true);
  });
});
