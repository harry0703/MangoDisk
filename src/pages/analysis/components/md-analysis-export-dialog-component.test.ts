// @vitest-environment happy-dom

import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import type { AnalysisExportOptions } from '@/lib/models/storage/analysis-export';
import type { AnalysisResult } from '@/lib/models/storage/analysis';
import MdAnalysisExportDialog from './md-analysis-export-dialog.vue';

const { optionsMock, saveMock, successMock } = vi.hoisted(() => ({
  optionsMock: vi.fn(),
  saveMock: vi.fn(),
  successMock: vi.fn(),
}));
vi.mock('@/lib/services/storage/analysis-export-service', () => ({
  AnalysisExportService: { options: optionsMock, save: saveMock },
}));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('@/lib/services/settings/operating-system-service', () => ({
  OperatingSystemService: { isWindows: () => false },
}));
vi.mock('vue-sonner', () => ({ toast: { success: successMock, error: vi.fn() } }));

const coverage = {
  schemaVersion: 1 as const,
  omissions: { permissionDenied: 2, readErrors: 0, excluded: 0, linkOrMount: 0, other: 0 },
  elevated: false,
};
const options: AnalysisExportOptions = {
  schemaVersion: 1,
  scanId: 7,
  scanMode: 'standard',
  scanRoot: { path: '/fixture', scannedAtMs: 1, totalBytes: 4096, fileCount: 3, coverage },
  currentDirectory: { path: '/fixture/Downloads', scannedAtMs: 1, totalBytes: 4096, fileCount: 3, coverage },
};
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
function mountDialog(open = true) {
  return mount(MdAnalysisExportDialog, {
    props: { open, result },
    global: {
      plugins: [i18n],
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
beforeEach(() => {
  optionsMock.mockReset().mockResolvedValue(structuredClone(options));
  saveMock.mockReset().mockResolvedValue(null);
  successMock.mockReset();
  i18n.global.locale.value = 'en-US';
});
afterEach(() => {
  i18n.global.locale.value = originalLocale;
});

describe('analysis export dialog', () => {
  it('does not request export options when mounted closed', async () => {
    const wrapper = mountDialog(false);
    await flushPromises();
    expect(optionsMock).not.toHaveBeenCalled();
    await wrapper.setProps({ open: true });
    await flushPromises();
    expect(optionsMock).toHaveBeenCalledExactlyOnceWith(7);
    wrapper.unmount();
  });

  it('opens the complete form only after export metadata resolves', async () => {
    let finish!: (value: AnalysisExportOptions) => void;
    optionsMock.mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.find('fieldset').exists()).toBe(false);
    expect(wrapper.getComponent({ name: 'Dialog' }).props('open')).toBe(false);
    expect(wrapper.emitted('loading')).toEqual([[true]]);
    finish(options);
    await flushPromises();
    expect(wrapper.getComponent({ name: 'Dialog' }).props('open')).toBe(true);
    expect(wrapper.find('fieldset').exists()).toBe(true);
    expect(wrapper.emitted('loading')).toEqual([[true], [false]]);
    wrapper.unmount();
  });

  it('releases the button loading state and ignores a response after closing', async () => {
    let finish!: (value: AnalysisExportOptions) => void;
    optionsMock.mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const wrapper = mountDialog();
    await flushPromises();
    await wrapper.setProps({ open: false });
    expect(wrapper.emitted('loading')).toEqual([[true], [false]]);
    finish(options);
    await flushPromises();
    expect(wrapper.find('fieldset').exists()).toBe(false);
    expect(wrapper.getComponent({ name: 'Dialog' }).props('open')).toBe(false);
    wrapper.unmount();
  });

  it('defaults to the full scan and relative paths without a dynamic coverage notice', async () => {
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.get('input[value="scanRoot"]').element).toHaveProperty('checked', true);
    expect(wrapper.get('[role="checkbox"]').attributes('aria-checked')).toBe('false');
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    expect(wrapper.get('[role=combobox][aria-labelledby=analysis-export-detail-label]').text()).toContain(
      'Key directory summary'
    );
    await wrapper.get('input[value="currentDirectory"]').setValue();
    await wrapper.get('[role="checkbox"]').trigger('click');
    await wrapper
      .findAll('button')
      .find(button => button.text() === 'Export JSON…')!
      .trigger('click');
    await flushPromises();
    expect(saveMock).toHaveBeenCalledExactlyOnceWith(
      7,
      'currentDirectory',
      true,
      'Save analysis results',
      'summary',
      0
    );
    expect(successMock).not.toHaveBeenCalled();
    expect(wrapper.emitted('update:open')).toBeUndefined();
    wrapper.unmount();
  });

  it('hides the redundant scope chooser when already at the scan root', async () => {
    optionsMock.mockResolvedValueOnce({ ...options, currentDirectory: options.scanRoot });
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.find('input[type=radio]').exists()).toBe(false);
    expect(wrapper.text()).toContain('/fixture');
    wrapper.unmount();
  });

  it('updates the selected detail label when the language changes', async () => {
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.get('[role=combobox][aria-labelledby=analysis-export-detail-label]').text()).toContain(
      'Key directory summary'
    );
    i18n.global.locale.value = 'zh-CN';
    await flushPromises();
    expect(wrapper.get('[role=combobox][aria-labelledby=analysis-export-detail-label]').text()).toBe(
      '重点目录摘要（推荐）'
    );
    wrapper.unmount();
  });

  it('exports complete directories when selected and keeps the selection after cancellation', async () => {
    const wrapper = mountDialog();
    await flushPromises();
    const select = wrapper.findComponent({ name: 'Select' });
    select.vm.$emit('update:modelValue', 'full');
    await flushPromises();
    expect(wrapper.get('.export-detail-hints [aria-hidden="false"]').text()).toContain('every matching directory');
    const button = () => wrapper.findAll('button').find(button => button.text() === 'Export JSON…')!;
    await button().trigger('click');
    await flushPromises();
    expect(saveMock).toHaveBeenCalledExactlyOnceWith(7, 'scanRoot', false, 'Save analysis results', 'full', 0);
    expect(wrapper.get('[role=combobox][aria-labelledby=analysis-export-detail-label]').text()).toContain(
      'Complete directories'
    );
    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    await flushPromises();
    expect(wrapper.get('[role=combobox][aria-labelledby=analysis-export-detail-label]').text()).toContain(
      'Key directory summary'
    );
    wrapper.unmount();
  });

  it.each(['summary', 'full'] as const)(
    'applies a shared size filter in %s mode and resets it only on reopen',
    async mode => {
      const wrapper = mountDialog();
      await flushPromises();
      const filters = wrapper.findAllComponents({ name: 'Select' });
      expect(wrapper.get('[aria-labelledby=analysis-export-minimum-label]').text()).toBe('No minimum');
      filters[0]!.vm.$emit('update:modelValue', mode);
      filters[1]!.vm.$emit('update:modelValue', '100000000');
      await flushPromises();
      expect(wrapper.get('[aria-labelledby=analysis-export-minimum-label]').text()).toBe('100 MB');
      await wrapper
        .findAll('button')
        .find(button => button.text() === 'Export JSON…')!
        .trigger('click');
      await flushPromises();
      expect(saveMock).toHaveBeenCalledExactlyOnceWith(7, 'scanRoot', false, 'Save analysis results', mode, 100000000);
      expect(wrapper.get('[aria-labelledby=analysis-export-minimum-label]').text()).toBe('100 MB');
      await wrapper.setProps({ open: false });
      await wrapper.setProps({ open: true });
      await flushPromises();
      expect(wrapper.get('[aria-labelledby=analysis-export-minimum-label]').text()).toBe('No minimum');
      wrapper.unmount();
    }
  );

  it('blocks duplicate saves and reports success only after native completion', async () => {
    let finish!: (value: unknown) => void;
    saveMock.mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const wrapper = mountDialog();
    await flushPromises();
    const button = wrapper.findAll('button').find(button => button.text() === 'Export JSON…')!;
    await button.trigger('click');
    await button.trigger('click');
    expect(saveMock).toHaveBeenCalledTimes(1);
    expect(wrapper.getComponent({ name: 'MdOperationDialog' }).props('open')).toBe(true);
    finish({ schemaVersion: 1, path: '/exports/report.json', directoryCount: 12, fileBytes: 500 });
    await flushPromises();
    expect(wrapper.emitted('update:open')).toEqual([[false]]);
    expect(successMock).toHaveBeenCalledWith(
      'Analysis results exported',
      expect.objectContaining({ description: 'report.json' })
    );
    wrapper.unmount();
  });

  it('retains the selection after a failed save so the user can retry', async () => {
    saveMock.mockRejectedValueOnce({ code: 'persistenceFailed', details: {}, retryable: true });
    const wrapper = mountDialog();
    await flushPromises();
    await wrapper.get('input[value="currentDirectory"]').setValue();
    await wrapper
      .findAll('button')
      .find(button => button.text() === 'Export JSON…')!
      .trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain('Could not save');
    expect(wrapper.get('input[value="currentDirectory"]').element).toHaveProperty('checked', true);
    expect(wrapper.emitted('update:open')).toBeUndefined();
    wrapper.unmount();
  });

  it('offers explicit rescan when the matching index is unavailable', async () => {
    optionsMock.mockRejectedValueOnce({
      code: 'invalidInput',
      details: { reason: 'analysisExportUnavailable' },
      retryable: false,
    });
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.text()).toContain('no longer available');
    await wrapper
      .findAll('button')
      .find(button => button.text() === 'Analyze again')!
      .trigger('click');
    expect(wrapper.emitted('rescan')).toEqual([['/fixture/Downloads']]);
    expect(saveMock).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('ignores an old options request after the dialog is closed', async () => {
    let finish!: (value: AnalysisExportOptions) => void;
    optionsMock.mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const wrapper = mountDialog();
    await wrapper.setProps({ open: false });
    finish(options);
    await flushPromises();
    expect(wrapper.find('fieldset').exists()).toBe(false);
    expect(saveMock).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('rescans the selected export scope if its index expires before saving', async () => {
    saveMock.mockRejectedValueOnce({
      code: 'invalidInput',
      details: { reason: 'analysisExportUnavailable' },
      retryable: false,
    });
    const wrapper = mountDialog();
    await flushPromises();
    await wrapper
      .findAll('button')
      .find(button => button.text() === 'Export JSON…')!
      .trigger('click');
    await flushPromises();
    await wrapper
      .findAll('button')
      .find(button => button.text() === 'Analyze again')!
      .trigger('click');
    expect(wrapper.emitted('rescan')).toEqual([['/fixture']]);
    wrapper.unmount();
  });
});
