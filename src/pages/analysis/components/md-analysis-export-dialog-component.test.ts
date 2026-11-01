// @vitest-environment happy-dom

import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia, type Pinia } from 'pinia';
import { i18n } from '@/i18n';
import type { AnalysisResult } from '@/lib/models/storage/analysis';
import { PreferenceStorageService } from '@/lib/services/settings/preference-storage-service';
import { useAnalysisExportPreferencesStore } from '@/stores/analysis-export-preferences-store';
import MdAnalysisExportDialog from './md-analysis-export-dialog.vue';

const { saveMock, successMock } = vi.hoisted(() => ({ saveMock: vi.fn(), successMock: vi.fn() }));
vi.mock('@/lib/services/storage/analysis-export-service', () => ({ AnalysisExportService: { save: saveMock } }));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('@/lib/services/settings/operating-system-service', () => ({
  OperatingSystemService: { isWindows: () => false },
}));
vi.mock('vue-sonner', () => ({ toast: { success: successMock, error: vi.fn() } }));

const result: AnalysisResult = {
  scanId: 7,
  root: '/fixture/Downloads',
  scannedAtMs: 1,
  totalBytes: 4096,
  skippedCount: 2,
  truncated: false,
  entries: [],
};
const passthrough = { template: '<div><slot/></div>' };
let pinia: Pinia;
function mountDialog(open = true) {
  return mount(MdAnalysisExportDialog, {
    props: { open, result },
    global: {
      plugins: [pinia, i18n],
      stubs: {
        Dialog: { name: 'Dialog', props: ['open'], template: '<div v-if="open"><slot/></div>' },
        DialogTitle: passthrough,
        DialogDescription: passthrough,
        MdDialogContent: passthrough,
        MdDialogHeader: passthrough,
        MdDialogFooter: passthrough,
        MdOperationDialog: true,
      },
    },
  });
}
const originalLocale = i18n.global.locale.value;
beforeEach(async () => {
  pinia = createPinia();
  setActivePinia(pinia);
  vi.spyOn(PreferenceStorageService, 'loadAnalysisExportPreferences').mockResolvedValue(null);
  vi.spyOn(PreferenceStorageService, 'saveAnalysisExportPreferences').mockResolvedValue();
  await useAnalysisExportPreferencesStore().initialize();
  vi.mocked(PreferenceStorageService.loadAnalysisExportPreferences).mockClear();
  saveMock.mockReset().mockResolvedValue(null);
  successMock.mockReset();
  i18n.global.locale.value = 'en-US';
});
afterEach(() => {
  i18n.global.locale.value = originalLocale;
  vi.restoreAllMocks();
});
const exportButton = (wrapper: ReturnType<typeof mountDialog>) =>
  wrapper.findAll('button').find(b => b.text() === 'Export…')!;

describe('analysis list export dialog', () => {
  it('opens immediately from local list metadata without invoking a native service', async () => {
    const wrapper = mountDialog(false);
    await wrapper.setProps({ open: true });
    expect(wrapper.getComponent({ name: 'Dialog' }).props('open')).toBe(true);
    expect(wrapper.text()).toContain('Current folder');
    expect(wrapper.text()).toContain(result.root);
    expect(wrapper.findAll('[role=combobox]')).toHaveLength(3);
    expect(wrapper.get('[aria-labelledby=analysis-export-format]').text()).toBe('JSON');
    expect(wrapper.get('[aria-labelledby=analysis-export-depth]').text()).toBe('Depth 2');
    expect(wrapper.get('[aria-labelledby=analysis-export-limit]').text()).toBe('500 items');
    expect(wrapper.text()).toContain('Up to 500 items across all levels combined');
    expect(wrapper.find('input[type=radio]').exists()).toBe(false);
    expect(wrapper.find('[role=alert]').exists()).toBe(false);
    expect(saveMock).not.toHaveBeenCalled();
    expect(PreferenceStorageService.loadAnalysisExportPreferences).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('defaults to relative paths and preserves the preference after picker cancellation', async () => {
    const wrapper = mountDialog();
    expect(wrapper.get('[role=checkbox]').attributes('aria-checked')).toBe('false');
    await wrapper.get('[role=checkbox]').trigger('click');
    await exportButton(wrapper).trigger('click');
    await flushPromises();
    expect(saveMock).toHaveBeenCalledExactlyOnceWith(
      { scanId: 7, includeFullPaths: true, maxDepth: 2, maxEntries: 500, format: 'json' },
      'Save analysis results'
    );
    expect(wrapper.get('[role=checkbox]').attributes('aria-checked')).toBe('true');
    expect(successMock).not.toHaveBeenCalled();
    expect(wrapper.emitted('update:open')).toBeUndefined();
    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    expect(wrapper.get('[role=checkbox]').attributes('aria-checked')).toBe('true');
    wrapper.unmount();
  });

  it('retains six levels and all available details across reopening, new scans and page remounts', async () => {
    const wrapper = mountDialog();
    const selects = wrapper.findAllComponents({ name: 'Select' });
    selects[0]!.vm.$emit('update:modelValue', 'markdown');
    selects[1]!.vm.$emit('update:modelValue', '6');
    selects[2]!.vm.$emit('update:modelValue', '1000');
    await flushPromises();
    expect(wrapper.text()).toContain('Up to 1,000 items across all levels combined');
    selects[2]!.vm.$emit('update:modelValue', '0');
    await flushPromises();
    expect(wrapper.text()).toContain('Export all available details within the selected depth');
    expect(wrapper.text()).not.toContain('Up to 0 items');
    await exportButton(wrapper).trigger('click');
    await flushPromises();
    expect(saveMock).toHaveBeenCalledExactlyOnceWith(
      { scanId: 7, includeFullPaths: false, maxDepth: 6, maxEntries: 0, format: 'markdown' },
      'Save analysis results'
    );
    expect(wrapper.get('[aria-labelledby=analysis-export-limit]').text()).toBe('All available details');
    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    expect(wrapper.get('[aria-labelledby=analysis-export-depth]').text()).toBe('Depth 6');
    expect(wrapper.get('[aria-labelledby=analysis-export-limit]').text()).toBe('All available details');
    await wrapper.setProps({ result: { ...result, scanId: 8, root: '/fixture/Other' } });
    expect(wrapper.get('[aria-labelledby=analysis-export-depth]').text()).toBe('Depth 6');
    wrapper.unmount();
    const reentered = mountDialog();
    expect(reentered.get('[aria-labelledby=analysis-export-depth]').text()).toBe('Depth 6');
    expect(reentered.get('[aria-labelledby=analysis-export-format]').text()).toBe('Markdown');
    expect(reentered.get('[aria-labelledby=analysis-export-limit]').text()).toBe('All available details');
    reentered.unmount();
  });

  it('prevents duplicate exports while the native picker is active', async () => {
    let finish!: (value: null) => void;
    saveMock.mockImplementation(
      () =>
        new Promise(resolve => {
          finish = resolve;
        })
    );
    const wrapper = mountDialog();
    const button = exportButton(wrapper);
    await button.trigger('click');
    expect(wrapper.getComponent({ name: 'Dialog' }).props('open')).toBe(false);
    expect(wrapper.getComponent({ name: 'MdOperationDialog' }).props('open')).toBe(true);
    expect(saveMock).toHaveBeenCalledTimes(1);
    finish(null);
    await flushPromises();
    expect(wrapper.getComponent({ name: 'Dialog' }).props('open')).toBe(true);
    wrapper.unmount();
  });

  it('offers a rescan only when the retained list has expired', async () => {
    saveMock.mockRejectedValueOnce({
      code: 'invalidInput',
      details: { reason: 'analysisExportUnavailable' },
      retryable: false,
    });
    const wrapper = mountDialog();
    await exportButton(wrapper).trigger('click');
    await flushPromises();
    expect(wrapper.get('[role=alert]').text()).toContain('scan data is no longer available');
    await wrapper
      .findAll('button')
      .find(b => b.text() === 'Analyze again')!
      .trigger('click');
    expect(wrapper.emitted('rescan')).toEqual([[result.root]]);
    expect(wrapper.emitted('update:open')).toEqual([[false]]);
    wrapper.unmount();
  });

  it('keeps retry available on save failure and closes only after a successful save', async () => {
    saveMock.mockRejectedValueOnce(new Error('permission denied'));
    const wrapper = mountDialog();
    await exportButton(wrapper).trigger('click');
    await flushPromises();
    expect(wrapper.get('[role=alert]').text()).toContain('Could not save');
    saveMock.mockResolvedValueOnce({ schemaVersion: 2, path: '/exports/list.json', entryCount: 0, fileBytes: 100 });
    await exportButton(wrapper).trigger('click');
    await flushPromises();
    expect(wrapper.emitted('update:open')).toEqual([[false]]);
    expect(successMock).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it('updates local labels when switching languages', async () => {
    const wrapper = mountDialog();
    i18n.global.locale.value = 'zh-CN';
    await flushPromises();
    expect(wrapper.text()).toContain('当前目录');
    expect(wrapper.get('[aria-labelledby=analysis-export-depth]').text()).toBe('2 级');
    expect(wrapper.get('[aria-labelledby=analysis-export-limit]').text()).toBe('500 条');
    expect(wrapper.text()).toContain('所有层级合计最多 500 条');
    wrapper.unmount();
  });
});
