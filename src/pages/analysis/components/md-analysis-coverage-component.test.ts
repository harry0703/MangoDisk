// @vitest-environment happy-dom
import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { h, KeepAlive, nextTick, ref } from 'vue';
import { i18n } from '@/i18n';
import type { AnalysisResult } from '@/lib/models/storage/analysis';
import { OperatingSystemService } from '@/lib/services/settings/operating-system-service';
import MdAnalysisCoverage from './md-analysis-coverage.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));
const locales = Object.keys(i18n.global.messages.value);
function result(): AnalysisResult {
  return {
    policyVersion: 2,
    scanId: 7,
    root: '/fixture',
    scannedAtMs: 1,
    totalBytes: 64,
    skippedCount: 4,
    truncated: false,
    entries: [],
    coverage: {
      schemaVersion: 1,
      elevated: false,
      omissions: { permissionDenied: 1, readErrors: 1, excluded: 1, linkOrMount: 1, other: 0 },
    },
  };
}
const stubs = {
  MdTooltip: { template: '<span><slot /></span>' },
  Dialog: { props: ['open'], template: '<div v-if="open"><slot /></div>' },
  MdDialogContent: { template: '<div><slot /></div>' },
  MdDialogHeader: { template: '<div><slot /></div>' },
  MdDialogFooter: { template: '<div><slot /></div>' },
  DialogTitle: { template: '<h2><slot /></h2>' },
  DialogDescription: { template: '<p><slot /></p>' },
};
function render(value = result(), disabled = false) {
  return mount(MdAnalysisCoverage, { props: { result: value, disabled }, global: { plugins: [i18n], stubs } });
}
afterEach(() => {
  vi.restoreAllMocks();
  i18n.global.locale.value = 'zh-CN';
});
describe('analysis coverage', () => {
  it.each(locales)('explains omissions and carries only native scan authority in %s', async locale => {
    i18n.global.locale.value = locale as typeof i18n.global.locale.value;
    vi.spyOn(OperatingSystemService, 'isWindows').mockReturnValue(true);
    const wrapper = render();
    expect(wrapper.get('[role="status"]').text()).toContain('4');
    expect(wrapper.text()).not.toContain(i18n.global.t('analysis.coverage.restart'));
    await wrapper.get('button[aria-haspopup="dialog"]').trigger('click');
    for (const reason of ['permissionDenied', 'readErrors', 'excluded', 'linkOrMount']) {
      expect(wrapper.text()).toContain(i18n.global.t(`analysis.coverage.${reason}`));
    }
    const restart = wrapper
      .findAll('button')
      .find(button => button.text() === i18n.global.t('analysis.coverage.restart'))!;
    await restart.trigger('click');
    expect(wrapper.emitted('restartAsAdministrator')).toEqual([[7]]);
    wrapper.unmount();
  });
  it.each(['macos', 'linux', 'elevated', 'elevatedWithoutDenial', 'unknown', 'noPermissionFailure'])(
    'does not offer administrator restart for %s',
    async context => {
      vi.spyOn(OperatingSystemService, 'isWindows').mockReturnValue(!['macos', 'linux'].includes(context));
      const value = result();
      if (context === 'elevated' || context === 'elevatedWithoutDenial') value.coverage!.elevated = true;
      if (context === 'unknown') value.coverage!.elevated = null;
      if (context === 'noPermissionFailure' || context === 'elevatedWithoutDenial') {
        value.coverage!.omissions.permissionDenied = 0;
      }
      const wrapper = render(value);
      await wrapper.get('button[aria-haspopup="dialog"]').trigger('click');
      expect(wrapper.text()).not.toContain(i18n.global.t('analysis.coverage.restart'));
      expect(
        wrapper.find(`button[aria-label="${i18n.global.t('analysis.coverage.administratorHint')}"]`).exists()
      ).toBe(context === 'elevated');
      wrapper.unmount();
    }
  );
  it('shows no notice for a complete scan and treats legacy skips as unclassified', async () => {
    const value = result();
    value.skippedCount = 0;
    value.coverage = undefined;
    const wrapper = render(value);
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    await wrapper.setProps({ result: { ...value, skippedCount: 3 } });
    expect(wrapper.get('[role="status"]').text()).toContain('3');
    expect(wrapper.text()).not.toContain(i18n.global.t('analysis.coverage.restart'));
    wrapper.unmount();
  });
  it('closes details when navigation replaces the result', async () => {
    const wrapper = render();
    await wrapper.get('button[aria-haspopup="dialog"]').trigger('click');
    expect(wrapper.get('h2').text()).toBe(i18n.global.t('analysis.coverage.title'));
    await wrapper.setProps({ result: { ...result(), scanId: 8 } });
    expect(wrapper.find('h2').exists()).toBe(false);
    wrapper.unmount();
  });
  it('closes details when leaving and returning to a cached page', async () => {
    const active = ref(true);
    const host = {
      render: () =>
        h(KeepAlive, null, {
          default: () => (active.value ? h(MdAnalysisCoverage, { result: result() }) : h('section', 'alternate page')),
        }),
    };
    const wrapper = mount(host, { global: { plugins: [i18n], stubs } });
    await wrapper.get('button[aria-haspopup="dialog"]').trigger('click');
    expect(wrapper.find('h2').exists()).toBe(true);
    active.value = false;
    await nextTick();
    expect(wrapper.text()).toContain('alternate page');
    active.value = true;
    await nextTick();
    expect(wrapper.find('h2').exists()).toBe(false);
    wrapper.unmount();
  });
  it('blocks a second restart while scanning or restarting', async () => {
    vi.spyOn(OperatingSystemService, 'isWindows').mockReturnValue(true);
    const wrapper = render(result(), true);
    expect(wrapper.findAll('button').every(button => button.attributes('disabled') !== undefined)).toBe(true);
    await wrapper.setProps({ disabled: false });
    await wrapper.get('button[aria-haspopup="dialog"]').trigger('click');
    await wrapper.setProps({ restarting: true });
    const button = wrapper
      .findAll('button')
      .find(button => button.text() === i18n.global.t('analysis.coverage.restarting'))!;
    await button.trigger('click');
    expect(wrapper.emitted('restartAsAdministrator')).toBeUndefined();
    wrapper.unmount();
  });
});
