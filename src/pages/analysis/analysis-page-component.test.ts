// @vitest-environment happy-dom

import { flushPromises, mount, shallowMount } from '@vue/test-utils';
import { defineComponent, h, KeepAlive, ref } from 'vue';
import { AnalysisService } from '@/lib/services/storage/analysis-service';
import { useAnalysisStore } from '@/stores/analysis-store';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';
import MdAnalysisSunburst from './components/md-analysis-sunburst.vue';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import MdDelayedOperationWorkspace from '@/components/custom/md-delayed-operation-workspace.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import MdAnalysisBrowserToolbar from './components/md-analysis-browser-toolbar.vue';
import MdAnalysisFolderPane from './components/md-analysis-folder-pane.vue';
import MdAnalysisVisualPane from './components/md-analysis-visual-pane.vue';
import MdAnalysisScanButton from './components/md-analysis-scan-button.vue';
import MdAnalysisExportDialog from './components/md-analysis-export-dialog.vue';
import MdDestructiveActionDialog from '@/components/custom/md-destructive-action-dialog.vue';
import { ByteSizeService } from '@/lib/services/settings/byte-size-service';
import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { i18n } from '@/i18n';
import { Select } from '@/components/ui/select';
import type { AnalysisDeleteResult, AnalysisResult } from '@/lib/models/storage/analysis';

import AnalysisPage from './index.vue';
import { useAppStore } from '@/stores/app-store';
import { useStorageScopeStore } from '@/stores/storage-scope-store';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));

const result: AnalysisResult = {
  scanId: 7,
  root: '/fixture',
  scannedAtMs: 1,
  totalBytes: 64,
  skippedCount: 0,
  truncated: false,
  entries: [],
};

beforeEach(() => {
  setActivePinia(createPinia());
  vi.restoreAllMocks();
  useAnalysisStore().viewPreferencesInitialized = true;
  vi.spyOn(FolderSelectionService, 'filterExistingDirectories').mockImplementation(async paths => paths);
  vi.spyOn(PreferenceStorageService, 'saveAnalysisViewPreferences').mockResolvedValue();
});

describe('analysis page', () => {
  it.each(['navigate', 'scan'])('opens export immediately and closes it on %s', async action => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        renderStubDefaultSlot: true,
        stubs: { MdPageShell: { template: '<div><slot name="actions"/><slot/></div>' } },
      },
    });
    try {
      await wrapper.get('.analysis-export-button').trigger('click');
      const dialog = wrapper.getComponent(MdAnalysisExportDialog);
      expect(dialog.props('open')).toBe(true);
      expect(dialog.props('result')).toEqual(result);
      expect(wrapper.get('.analysis-export-button').attributes('aria-busy')).toBeUndefined();
      if (action === 'scan') await wrapper.setProps({ busy: true });
      else await wrapper.setProps({ result: { ...result, scanId: 8, root: '/fixture/child' } });
      expect(dialog.props('open')).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it('shows cancellable progress while validating a rescan with existing results', async () => {
    let completeResolution!: (paths: string[]) => void;
    vi.mocked(FolderSelectionService.filterExistingDirectories).mockImplementationOnce(
      () => new Promise(resolve => (completeResolution = resolve))
    );
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        renderStubDefaultSlot: true,
        stubs: { MdPageShell: { template: '<div><slot name="actions"/><slot/></div>' } },
      },
    });
    try {
      wrapper.getComponent(MdAnalysisScanButton).vm.$emit('scan', 'standard');
      await flushPromises();
      expect(wrapper.getComponent(MdDelayedOperationWorkspace).props('active')).toBe(true);
      expect(wrapper.getComponent(MdAnalysisScanButton).props('scanning')).toBe(true);
      wrapper.getComponent(MdOperationProgress).vm.$emit('cancel');
      await flushPromises();
      expect(wrapper.getComponent(MdDelayedOperationWorkspace).props('active')).toBe(false);
      completeResolution(['/fixture']);
      await flushPromises();
      expect(wrapper.emitted('analyze')).toBeUndefined();
      expect(wrapper.emitted('cancel')).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it('resolves a saved scope only when explicitly starting analysis', async () => {
    const scope = useStorageScopeStore();
    scope.selectedPaths.analysis = 'C:\\Mac\\Home\\Documents';
    const resolve = vi.mocked(FolderSelectionService.filterExistingDirectories);
    resolve.mockResolvedValueOnce(['\\\\psf\\Home\\Documents']);
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: { plugins: [i18n], stubs: { MdPageShell: { template: '<div><slot name="actions"/><slot/></div>' } } },
    });
    try {
      await flushPromises();
      expect(resolve).not.toHaveBeenCalled();
      wrapper.getComponent(MdAnalysisScanButton).vm.$emit('scan', 'standard');
      await flushPromises();
      expect(wrapper.emitted('analyze')?.at(-1)?.[0]).toBe('\\\\psf\\Home\\Documents');
      expect(scope.selectedPath('analysis')).toBe('\\\\psf\\Home\\Documents');
    } finally {
      wrapper.unmount();
    }
  });

  it('rejects a protected entry even when a child emits a delete event', async () => {
    const entry = {
      name: 'hiberfil.sys',
      path: 'C:\\hiberfil.sys',
      bytes: 64,
      fileCount: 1,
      isDirectory: false,
      modifiedAtMs: null,
      contentFingerprint: null,
      deletionProtection: 'systemManagedFile' as const,
    };
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result: { ...result, entries: [entry] },
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: { plugins: [i18n], stubs: { MdPageShell: { template: '<div><slot /></div>' } } },
    });
    wrapper.getComponent(MdAnalysisFolderPane).vm.$emit('delete', entry);
    await flushPromises();
    const dialog = wrapper.getComponent(MdDestructiveActionDialog);
    expect(dialog.props('open')).toBe(false);
    dialog.vm.$emit('confirm');
    expect(wrapper.emitted('delete')).toBeUndefined();
    wrapper.unmount();
  });
  it.each(['macos', 'windows'])('keeps the %s browser mounted during recovery progress and cancellation', async os => {
    vi.useFakeTimers();
    vi.spyOn(OperatingSystemService, 'isWindows').mockReturnValue(os === 'windows');
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: {
          MdPageShell: { template: '<div><slot /></div>' },
          MdDelayedOperationWorkspace: false,
          MdOperationWorkspace: { template: '<div><slot /></div>' },
        },
      },
    });
    try {
      const listId = wrapper.getComponent(MdAnalysisFolderPane).vm.$.uid;
      const chartId = wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid;
      await wrapper.setProps({
        busy: true,
        recovering: true,
        recoveryRequired: true,
        progress: {
          operationId: 5,
          currentStage: 'analyzing',
          currentPath: '/fixture',
          itemsScanned: 12,
          bytesScanned: 100,
          completedSteps: 0,
          totalSteps: 1,
          foundItems: 12,
          foundBytes: 100,
          elapsedMs: 100,
        },
      });
      await vi.advanceTimersByTimeAsync(30_000);
      expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(false);
      expect(wrapper.getComponent(MdAnalysisFolderPane).vm.$.uid).toBe(listId);
      expect(wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid).toBe(chartId);
      expect(wrapper.find('.browser-content').attributes('inert')).toBeUndefined();
      expect(wrapper.getComponent(MdAnalysisFolderPane).props('deleteDisabled')).toBe(true);
      await wrapper.get('.recovery-status button').trigger('click');
      expect(wrapper.emitted('cancel')).toHaveLength(1);
      await wrapper.setProps({ busy: false, recovering: false, progress: null });
      expect(wrapper.getComponent(MdAnalysisFolderPane).vm.$.uid).toBe(listId);
      expect(wrapper.getComponent(MdAnalysisFolderPane).props('deleteDisabled')).toBe(true);
      expect(wrapper.get('.recovery-status').text()).toContain(i18n.global.t('analysis.recoveryRequired'));
      await wrapper.setProps({ recoveryRequired: false, result: { ...result, scanId: 9 } });
      expect(wrapper.find('.recovery-status').exists()).toBe(false);
      expect(wrapper.getComponent(MdAnalysisFolderPane).props('deleteDisabled')).toBe(false);
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it.each([
    ['macos', 501],
    ['macos', 30_000],
    ['linux', 501],
    ['linux', 30_000],
    ['windows', 501],
    ['windows', 30_000],
  ] as const)('does not show scan progress for a %s cached navigation taking %i ms', async (os, delay) => {
    vi.useFakeTimers();
    vi.spyOn(OperatingSystemService, 'isWindows').mockReturnValue(os === 'windows');
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: {
          MdPageShell: { template: '<div><slot /></div>' },
          MdDelayedOperationWorkspace: false,
          MdOperationWorkspace: { template: '<div><slot /></div>' },
        },
      },
    });
    try {
      const chart = wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid;
      wrapper.getComponent(MdAnalysisBrowserToolbar).vm.$emit('navigate', '/fixture/child');
      await wrapper.setProps({ busy: true });
      await vi.advanceTimersByTimeAsync(delay);
      expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(false);
      expect(wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid).toBe(chart);
      expect(wrapper.getComponent(MdAnalysisBrowserToolbar).props('busy')).toBe(true);

      await wrapper.setProps({ busy: false, result: { ...result, root: '/fixture/child' } });
      expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(false);
      expect(wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid).toBe(chart);

      // A cache miss must still expose actual scan progress after the same wait.
      wrapper.getComponent(MdAnalysisBrowserToolbar).vm.$emit('navigate', '/unscanned');
      await wrapper.setProps({ busy: true });
      await vi.advanceTimersByTimeAsync(delay);
      expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(false);
      await wrapper.setProps({
        progress: {
          operationId: 8,
          currentStage: 'analyzing',
          currentPath: '/unscanned',
          itemsScanned: 0,
          bytesScanned: 0,
          completedSteps: 0,
          totalSteps: 0,
          foundItems: 0,
          foundBytes: 0,
          elapsedMs: 0,
        },
      });
      await vi.advanceTimersByTimeAsync(0);
      expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(true);
      expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(false);
      await wrapper.setProps({ busy: false, progress: null });
      expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(false);
    } finally {
      wrapper.unmount();
      vi.useRealTimers();
    }
  });

  it.each(['standard', 'fast'] as const)('prefixes Windows scan progress with the active %s mode', async mode => {
    vi.spyOn(OperatingSystemService, 'isWindows').mockReturnValue(true);
    useAnalysisStore().scanMode = mode;
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result: null,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: true,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: {
          MdPageShell: { template: '<div><slot /></div>' },
          MdDelayedOperationWorkspace: { template: '<div><slot /></div>' },
        },
      },
    });
    try {
      expect(wrapper.getComponent(MdOperationProgress).props('hint')).toBe(
        `${i18n.global.t(`analysis.scanMode.${mode}`)} · ${i18n.global.t('loading.cancelHint')}`
      );
      await wrapper.setProps({ cancelling: true });
      expect(wrapper.getComponent(MdOperationProgress).props('hint')).toContain(
        i18n.global.t(`analysis.scanMode.${mode}`)
      );
    } finally {
      wrapper.unmount();
    }
  });

  it('keeps the Unix progress hint unchanged', () => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result: null,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: true,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: {
          MdPageShell: { template: '<div><slot /></div>' },
          MdDelayedOperationWorkspace: { template: '<div><slot /></div>' },
        },
      },
    });
    try {
      expect(wrapper.getComponent(MdOperationProgress).props('hint')).toBe(i18n.global.t('loading.cancelHint'));
    } finally {
      wrapper.unmount();
    }
  });

  it('restores mode and independent depths on page entry without waiting for saves', async () => {
    const store = useAnalysisStore();
    store.viewPreferences = { schemaVersion: 1, viewMode: 'sunburst', treemapDepth: 2, sunburstDepth: 5 };
    let finish: () => void = () => undefined;
    vi.mocked(PreferenceStorageService.saveAnalysisViewPreferences).mockImplementation(
      () =>
        new Promise(resolve => {
          finish = resolve;
        })
    );
    const props = {
      result,
      excludedFolders: [],
      homePath: '/fixture',
      disk: null,
      disks: [],
      progress: null,
      busy: false,
      cancelling: false,
      deleting: false,
    };
    const options = { props, global: { plugins: [i18n], stubs: { MdPageShell: { template: '<div><slot /></div>' } } } };
    const wrapper = shallowMount(AnalysisPage, options);
    try {
      const chart = wrapper.getComponent(MdAnalysisVisualPane);
      expect(chart.props('viewMode')).toBe('sunburst');
      expect(chart.props('sunburstDepth')).toBe(5);
      chart.vm.$emit('update:viewMode', 'treemap');
      await wrapper.vm.$nextTick();
      expect(chart.props('viewMode')).toBe('treemap');
      expect(chart.props('treemapDepth')).toBe(2);
      expect(chart.vm.$.uid).toBe(wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid);
      finish();
    } finally {
      wrapper.unmount();
    }
    const reentered = shallowMount(AnalysisPage, options);
    try {
      expect(reentered.getComponent(MdAnalysisVisualPane).props('viewMode')).toBe('treemap');
      expect(reentered.getComponent(MdAnalysisVisualPane).props('sunburstDepth')).toBe(5);
    } finally {
      reentered.unmount();
    }
  });

  it('preserves the sunburst depth selection while an unvisited child is resolved from the backend cache', async () => {
    const store = useAnalysisStore();
    store.result = result;
    useStorageScanPreferencesStore().initialized = true;
    const unlisten = vi.fn();
    vi.spyOn(AnalysisService, 'listenProgress').mockResolvedValue(unlisten);
    let complete: (value: AnalysisResult) => void = () => undefined;
    const analyze = vi.spyOn(AnalysisService, 'analyze').mockImplementation(
      () =>
        new Promise(resolve => {
          complete = resolve;
        })
    );
    const wrapper = mount(
      defineComponent({
        setup: () => () =>
          h(AnalysisPage, {
            result: store.result,
            excludedFolders: [],
            homePath: '/fixture',
            disk: null,
            disks: [],
            progress: store.progress,
            busy: store.pending,
            cancelling: store.cancelling,
            deleting: false,
          }),
      }),
      {
        global: {
          plugins: [i18n],
          stubs: {
            MdPageShell: { template: '<div><slot /></div>' },
            MdAnalysisBrowserToolbar: true,
            MdAnalysisFolderPane: true,
            MdAnalysisTreemap: true,
            MdIconSunburst: true,
            MdDestructiveActionDialog: true,
            MdTooltip: { template: '<span><slot /></span>' },
          },
        },
      }
    );
    try {
      wrapper.getComponent(MdAnalysisVisualPane).vm.$emit('update:viewMode', 'sunburst');
      await flushPromises();
      const table = wrapper.getComponent(MdAnalysisSunburst);
      wrapper.getComponent(MdAnalysisVisualPane).getComponent(Select).vm.$emit('update:modelValue', '4');
      await flushPromises();
      expect(wrapper.getComponent(MdAnalysisVisualPane).getComponent(Select).props('modelValue')).toBe('4');
      const request = store.analyze('/fixture/child');
      await flushPromises();
      expect(analyze).toHaveBeenCalledOnce();
      expect(store.scanStarted).toBe(true);
      expect(store.progress).toBeNull();
      const retainedWhilePending = wrapper.findComponent(MdAnalysisSunburst).exists();
      const fullProgressWhilePending = wrapper.find('.analysis-overlay--full').exists();
      complete({ ...result, root: '/fixture/child' });
      await request;
      await flushPromises();
      expect(unlisten).toHaveBeenCalledOnce();
      expect.soft(retainedWhilePending).toBe(true);
      expect.soft(fullProgressWhilePending).toBe(false);
      expect.soft(wrapper.getComponent(MdAnalysisSunburst).vm.$.uid).toBe(table.vm.$.uid);
      expect(wrapper.getComponent(Select).props('modelValue')).toBe('4');
    } finally {
      wrapper.unmount();
    }
  });

  it('expands the chart without losing the file pane or resetting layout on mode and folder changes', async () => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: { MdPageShell: { template: '<div><slot /></div>' } },
      },
    });
    try {
      const pane = wrapper.getComponent(MdAnalysisFolderPane);
      const toolbar = wrapper.getComponent(MdAnalysisBrowserToolbar);
      const chart = wrapper.getComponent(MdAnalysisVisualPane);
      expect(toolbar.props('listCollapsed')).toBe(false);
      expect(pane.attributes('style') ?? '').not.toContain('display: none');
      pane.vm.$emit('hoverEntry', '/fixture/item');
      await flushPromises();
      expect(chart.props('hoveredEntryPath')).toBe('/fixture/item');
      toolbar.vm.$emit('toggleList');
      await flushPromises();
      expect(pane.attributes('style')).toContain('display: none');
      expect(chart.props('hoveredEntryPath')).toBeNull();
      expect(wrapper.get('.browser-content').classes()).toContain('browser-content--list-collapsed');
      chart.vm.$emit('update:viewMode', 'sunburst');
      await wrapper.setProps({ result: { ...result, root: '/fixture/child' } });
      expect(toolbar.props('listCollapsed')).toBe(true);
      expect(chart.props('viewMode')).toBe('sunburst');
      toolbar.vm.$emit('toggleList');
      await flushPromises();
      expect(wrapper.getComponent(MdAnalysisFolderPane).vm.$.uid).toBe(pane.vm.$.uid);
      expect(pane.attributes('style') ?? '').not.toContain('display: none');
      expect(wrapper.get('.browser-content').classes()).not.toContain('browser-content--list-collapsed');
    } finally {
      wrapper.unmount();
    }
  });

  it.each(['treemap', 'sunburst'] as const)(
    'keeps the %s pane mounted through a linked deletion without scan UI',
    async viewMode => {
      const store = useAnalysisStore();
      useStorageScanPreferencesStore().initialized = true;
      const owner = {
        name: 'owner.bin',
        path: '/fixture/owner.bin',
        bytes: 64,
        fileCount: 1,
        isDirectory: false,
        modifiedAtMs: null,
        contentFingerprint: null,
      };
      const alias = { ...owner, name: 'alias.bin', path: '/fixture/alias.bin', bytes: 0 };
      store.result = { ...result, entries: [owner, alias] };
      store.cache = { '/fixture': store.result };
      store.homePath = '/fixture';
      store.viewPreferences = { schemaVersion: 1, viewMode, treemapDepth: 3, sunburstDepth: 4 };
      let complete: (value: AnalysisDeleteResult) => void = () => undefined;
      vi.spyOn(AnalysisService, 'deletePermanently').mockImplementation(
        () =>
          new Promise(resolve => {
            complete = resolve;
          })
      );
      const analyze = vi.spyOn(AnalysisService, 'analyze');
      vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
      const wrapper = mount(
        () =>
          h(AnalysisPage, {
            result: store.result,
            excludedFolders: [],
            homePath: store.homePath,
            disk: null,
            disks: [],
            progress: store.progress,
            busy: store.pending,
            cancelling: store.cancelling,
            deleting: store.deleting,
            deletingPath: store.deletingPath,
          }),
        {
          global: {
            plugins: [i18n],
            stubs: {
              MdPageShell: { template: '<div><slot /></div>' },
              MdAnalysisBrowserToolbar: true,
              MdAnalysisFolderPane: true,
              MdAnalysisTreemap: true,
              MdAnalysisSunburst: true,
              MdDestructiveActionDialog: true,
              MdTooltip: { template: '<span><slot /></span>' },
            },
          },
        }
      );
      try {
        const pane = wrapper.getComponent(MdAnalysisVisualPane);
        const uid = pane.vm.$.uid;
        const request = store.deletePermanently(owner);
        await flushPromises();
        expect(store.pending).toBe(false);
        expect(wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid).toBe(uid);
        expect(wrapper.find('.analysis-overlay--full').exists()).toBe(false);
        expect(wrapper.findComponent(MdOperationProgress).exists()).toBe(false);
        const updated = { ...result, entries: [{ ...alias, bytes: 64 }] };
        complete({
          schemaVersion: 1,
          requiresRescan: false,
          removedPath: owner.path,
          releasedBytes: 64,
          removedFileCount: 1,
          updatedResults: [updated],
          invalidatedScanIds: [],
        });
        await request;
        await flushPromises();
        expect(wrapper.getComponent(MdAnalysisVisualPane).vm.$.uid).toBe(uid);
        expect(pane.props('viewMode')).toBe(viewMode);
        expect(store.viewPreferences.treemapDepth).toBe(3);
        expect(store.viewPreferences.sunburstDepth).toBe(4);
        expect(store.result).toEqual(updated);
        expect(store.homePath).toBe('/fixture');
        expect(analyze).not.toHaveBeenCalled();
        expect(wrapper.find('.analysis-overlay--full').exists()).toBe(false);
      } finally {
        wrapper.unmount();
      }
    }
  );

  it('retains chart depth preferences when a new scan replaces the visual pane', async () => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: { plugins: [i18n], stubs: { MdPageShell: { template: '<div><slot /></div>' } } },
    });
    try {
      const chart = wrapper.getComponent(MdAnalysisVisualPane);
      chart.vm.$emit('update:treemapDepth', 6);
      chart.vm.$emit('update:sunburstDepth', 5);
      await flushPromises();
      await wrapper.setProps({ result: null, busy: true });
      expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(false);
      await wrapper.setProps({ result: { ...result, scanId: 8 }, busy: false });
      expect(wrapper.getComponent(MdAnalysisVisualPane).props('treemapDepth')).toBe(6);
      expect(wrapper.getComponent(MdAnalysisVisualPane).props('sunburstDepth')).toBe(5);
    } finally {
      wrapper.unmount();
    }
  });

  it('exposes the entry limit help only for a truncated result', async () => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: { MdPageShell: { template: '<div><slot /></div>' } },
      },
    });
    expect(wrapper.getComponent(MdAnalysisFolderPane).props('truncated')).toBe(false);
    await wrapper.setProps({ result: { ...result, truncated: true } });
    expect(wrapper.getComponent(MdAnalysisFolderPane).props('truncated')).toBe(true);
  });

  it('replaces stale browser data immediately during a primary rescan', async () => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: {
          MdPageShell: { template: '<div><slot name="actions" /><slot /></div>' },
        },
      },
    });

    expect(wrapper.findComponent(MdAnalysisBrowserToolbar).exists()).toBe(true);
    expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(true);

    wrapper.findComponent(MdAnalysisScanButton).vm.$emit('scan', 'standard');
    await wrapper.setProps({ busy: true });

    expect(wrapper.findComponent(MdAnalysisBrowserToolbar).exists()).toBe(false);
    expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(false);
    const progress = wrapper.getComponent(MdDelayedOperationWorkspace);
    expect(progress.props('delay')).toBe(0);
    expect(progress.classes()).toContain('analysis-overlay--full');

    await wrapper.setProps({ busy: false });
    expect(wrapper.findComponent(MdAnalysisBrowserToolbar).exists()).toBe(true);
    expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(true);
  });

  it('hides stale path and results when parent navigation starts a real scan', async () => {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result,
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: {
        plugins: [i18n],
        stubs: { MdPageShell: { template: '<div><slot /></div>' } },
      },
    });

    wrapper.getComponent(MdAnalysisBrowserToolbar).vm.$emit('navigate', '/');
    expect(wrapper.emitted('analyze')?.at(-1)).toEqual(['/', false, false]);
    await wrapper.setProps({ busy: true });
    expect(wrapper.findComponent(MdAnalysisBrowserToolbar).exists()).toBe(true);
    expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(true);
    const waitingOverlay = wrapper.getComponent(MdDelayedOperationWorkspace).vm.$.uid;

    await wrapper.setProps({
      progress: {
        operationId: 8,
        currentStage: 'analyzing',
        currentPath: '/',
        itemsScanned: 0,
        bytesScanned: 0,
        completedSteps: 0,
        totalSteps: 0,
        foundItems: 0,
        foundBytes: 0,
        elapsedMs: 0,
      },
    });
    expect(wrapper.findComponent(MdAnalysisBrowserToolbar).exists()).toBe(false);
    expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(false);
    const progress = wrapper.getComponent(MdDelayedOperationWorkspace);
    expect(progress.vm.$.uid).not.toBe(waitingOverlay);
    expect(progress.classes()).toContain('analysis-overlay--full');
    expect(progress.props('delay')).toBe(0);

    await wrapper.setProps({ busy: false, progress: null });
    expect(wrapper.findComponent(MdAnalysisBrowserToolbar).exists()).toBe(true);
    expect(wrapper.findComponent(MdAnalysisVisualPane).exists()).toBe(true);
  });
});

describe('analysis page batch selection', () => {
  const file = {
    name: 'file.bin',
    path: '/fixture/file.bin',
    bytes: 32,
    fileCount: 1,
    isDirectory: false,
    modifiedAtMs: null,
    contentFingerprint: null,
  };
  const folder = { ...file, name: 'folder', path: '/fixture/folder', isDirectory: true };
  const protectedEntry = {
    ...folder,
    name: 'reserved',
    path: '/fixture/reserved',
    deletionProtection: 'protectedPath' as const,
  };
  function createPage(selecting = true) {
    const wrapper = shallowMount(AnalysisPage, {
      props: {
        result: { ...result, entries: [file, folder, protectedEntry] },
        excludedFolders: [],
        homePath: '/fixture',
        disk: null,
        disks: [],
        progress: null,
        busy: false,
        cancelling: false,
        deleting: false,
      },
      global: { plugins: [i18n], stubs: { MdPageShell: { template: '<div><slot /></div>' } } },
    });
    if (selecting) wrapper.getComponent(MdAnalysisFolderPane).vm.$emit('update:selectionMode', true);
    return wrapper;
  }
  it('requires explicit selection mode before accepting a selection', async () => {
    const wrapper = createPage(false);
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    expect(pane.props('selectionMode')).toBe(false);
    pane.vm.$emit('update:selectedPaths', [file.path]);
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    expect(pane.props('selectedPaths')).toEqual([]);
    expect(wrapper.getComponent(MdDestructiveActionDialog).props('open')).toBe(false);
    pane.vm.$emit('update:selectionMode', true);
    await flushPromises();
    expect(pane.props('selectionMode')).toBe(true);
    wrapper.unmount();
  });
  it.each(['cancel', 'Escape'])('clears selection when exiting with %s', async exit => {
    const wrapper = createPage();
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    pane.vm.$emit('update:selectedPaths', [file.path]);
    await flushPromises();
    if (exit === 'cancel') pane.vm.$emit('update:selectionMode', false);
    else await wrapper.trigger('keydown', { key: 'Escape' });
    await flushPromises();
    expect(pane.props('selectionMode')).toBe(false);
    expect(pane.props('selectedPaths')).toEqual([]);
    pane.vm.$emit('update:selectionMode', true);
    await flushPromises();
    expect(pane.props('selectedPaths')).toEqual([]);
    wrapper.unmount();
  });
  it.each([false, true])('exits from body focus with selected=%s', async selected => {
    const wrapper = createPage();
    try {
      const pane = wrapper.getComponent(MdAnalysisFolderPane);
      if (selected) pane.vm.$emit('update:selectedPaths', [file.path]);
      await flushPromises();
      const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      await flushPromises();
      expect(pane.props('selectionMode')).toBe(false);
      expect(pane.props('selectedPaths')).toEqual([]);
      expect(event.defaultPrevented).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });
  it('scopes body Escape to the active cached page and removes it on unmount', async () => {
    const visible = ref(true);
    const host = mount(
      () =>
        h(KeepAlive, {}, () =>
          visible.value
            ? h(AnalysisPage, {
                result: { ...result, entries: [file] },
                excludedFolders: [],
                homePath: '/fixture',
                disk: null,
                disks: [],
                progress: null,
                busy: false,
                cancelling: false,
                deleting: false,
              })
            : h('div')
        ),
      {
        attachTo: document.body,
        global: {
          plugins: [i18n],
          stubs: {
            MdPageShell: { template: '<div><slot /></div>' },
            MdStorageScopeSelect: true,
            MdAnalysisScanButton: true,
            MdAnalysisBrowserToolbar: true,
            MdDelayedOperationWorkspace: true,
            MdOperationProgress: true,
            MdAnalysisFolderPane: true,
            MdAnalysisVisualPane: true,
            MdDestructiveActionDialog: true,
            MdAnalysisDeleteResultDialog: true,
          },
        },
      }
    );
    try {
      let pane = host.getComponent(MdAnalysisFolderPane);
      pane.vm.$emit('update:selectionMode', true);
      await flushPromises();
      visible.value = false;
      await flushPromises();
      const inactive = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
      document.body.dispatchEvent(inactive);
      expect(inactive.defaultPrevented).toBe(false);
      visible.value = true;
      await flushPromises();
      pane = host.getComponent(MdAnalysisFolderPane);
      expect(pane.props('selectionMode')).toBe(true);
      document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      await flushPromises();
      expect(pane.props('selectionMode')).toBe(false);
      pane.vm.$emit('update:selectionMode', true);
      await flushPromises();
    } finally {
      host.unmount();
    }
    const afterUnmount = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    document.body.dispatchEvent(afterUnmount);
    expect(afterUnmount.defaultPrevented).toBe(false);
  });
  it('keeps selection mode during deletion and while its confirmation is open', async () => {
    const wrapper = createPage();
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    pane.vm.$emit('update:selectedPaths', [file.path]);
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await wrapper.trigger('keydown', { key: 'Escape' });
    expect(pane.props('selectionMode')).toBe(true);
    expect(pane.props('selectedPaths')).toEqual([file.path]);
    wrapper.getComponent(MdDestructiveActionDialog).vm.$emit('update:open', false);
    await wrapper.setProps({ deleting: true });
    pane.vm.$emit('update:selectionMode', false);
    await wrapper.trigger('keydown', { key: 'Escape' });
    await flushPromises();
    expect(pane.props('selectionMode')).toBe(true);
    await wrapper.setProps({ deleting: false });
    await wrapper.trigger('keydown', { key: 'Escape' });
    expect(pane.props('selectionMode')).toBe(false);
    wrapper.unmount();
  });
  it('confirms the current files and folders together and rejects protected or foreign selections', async () => {
    const wrapper = createPage();
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    pane.vm.$emit('update:selectedPaths', [file.path, folder.path, protectedEntry.path, '/elsewhere']);
    await flushPromises();
    expect(pane.props('selectedPaths')).toEqual([file.path, folder.path]);
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    const dialog = wrapper.getComponent(MdDestructiveActionDialog);
    expect(dialog.props('open')).toBe(true);
    expect(dialog.props('title')).toBe('Permanently delete the selected items?');
    dialog.vm.$emit('confirm');
    expect(wrapper.emitted('deleteMany')).toEqual([[[file, folder]]]);
    expect(wrapper.emitted('delete')).toBeUndefined();
    wrapper.unmount();
  });
  it.each([100, 500])('confirms all %i entries with the full total after cancelling and reopening', async count => {
    const selected = Array.from({ length: count }, (_, index) => ({
      ...file,
      name: `${index}-${'long-name-'.repeat(20)}.bin`,
      path: `/fixture/${index}-${'long-name-'.repeat(20)}.bin`,
      bytes: index * 1024,
      isDirectory: index % 2 === 0,
    }));
    const wrapper = createPage();
    await wrapper.setProps({ result: { ...result, entries: [...selected, protectedEntry] } });
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    pane.vm.$emit('update:selectedPaths', [...selected.map(entry => entry.path), protectedEntry.path]);
    await flushPromises();
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    const dialog = wrapper.getComponent(MdDestructiveActionDialog);
    expect(dialog.props('summaryLabel')).toBe(`${count} selected`);
    expect(dialog.props('summaryValue')).toBe(
      ByteSizeService.bytes(selected.reduce((sum, entry) => sum + entry.bytes, 0))
    );
    expect(wrapper.emitted('deleteMany')).toBeUndefined();
    dialog.vm.$emit('update:open', false);
    await flushPromises();
    expect(dialog.props('open')).toBe(false);
    expect(wrapper.emitted('deleteMany')).toBeUndefined();
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    expect(dialog.props('open')).toBe(true);
    dialog.vm.$emit('confirm');
    await flushPromises();
    expect(wrapper.emitted('deleteMany')).toEqual([[[...selected].sort((left, right) => right.bytes - left.bytes)]]);
    expect(dialog.props('open')).toBe(false);
    wrapper.unmount();
  });
  it('clears selection and confirmation when scan authority changes', async () => {
    const wrapper = createPage();
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    pane.vm.$emit('update:selectedPaths', [file.path]);
    await flushPromises();
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    await wrapper.setProps({ result: { ...result, scanId: 8, entries: [file] } });
    expect(pane.props('selectedPaths')).toEqual([]);
    expect(pane.props('selectionMode')).toBe(false);
    const dialog = wrapper.getComponent(MdDestructiveActionDialog);
    expect(dialog.props('open')).toBe(false);
    dialog.vm.$emit('confirm');
    expect(wrapper.emitted('deleteMany')).toBeUndefined();
    wrapper.unmount();
  });
  it('blocks selection and batch confirmation while result recovery is required', async () => {
    const wrapper = createPage();
    await wrapper.setProps({ recoveryRequired: true });
    const pane = wrapper.getComponent(MdAnalysisFolderPane);
    pane.vm.$emit('update:selectedPaths', [file.path]);
    pane.vm.$emit('deleteSelected');
    await flushPromises();
    expect(pane.props('selectedPaths')).toEqual([]);
    expect(wrapper.getComponent(MdDestructiveActionDialog).props('open')).toBe(false);
    wrapper.unmount();
  });
});
