// @vitest-environment happy-dom

import { flushPromises, mount, shallowMount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import { AnalysisService } from '@/lib/services/analysis-service';
import { useAnalysisStore } from '@/stores/analysis-store';
import { useStorageScanPreferencesStore } from '@/stores/storage-scan-preferences-store';
import MdAnalysisSunburst from './components/md-analysis-sunburst.vue';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import MdDelayedOperationWorkspace from '@/components/custom/md-delayed-operation-workspace.vue';
import MdOperationProgress from '@/components/custom/md-operation-progress.vue';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import MdAnalysisBrowserToolbar from './components/md-analysis-browser-toolbar.vue';
import MdAnalysisFolderPane from './components/md-analysis-folder-pane.vue';
import MdAnalysisVisualPane from './components/md-analysis-visual-pane.vue';
import MdAnalysisScanButton from './components/md-analysis-scan-button.vue';
import { i18n } from '@/i18n';
import { Select } from '@/components/ui/select';
import type { AnalysisDeleteResult, AnalysisResult } from '@/lib/models/analysis';

import AnalysisPage from './index.vue';
import { useAppStore } from '@/stores/app-store';
import { PreferenceStorageService } from '@/lib/services/preference-storage-service';

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
  vi.spyOn(PreferenceStorageService, 'saveAnalysisViewPreferences').mockResolvedValue();
});

describe('analysis page', () => {
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
