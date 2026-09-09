// @vitest-environment happy-dom

import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

import { i18n } from '@/i18n';
import type { PresentedScanRuleResult } from '@/lib/models/cleanup';

import MdCleanupPlanDialog from './md-cleanup-plan-dialog.vue';
import cleanupPlanDialogSource from './md-cleanup-plan-dialog.vue?raw';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));

const passthroughStub = { template: '<div><slot /></div>' };
const dialogContentStub = {
  props: {
    height: {
      type: String,
      default: 'auto',
    },
  },
  template: '<section class="dialog-content-stub" :data-dialog-height="height"><slot /></section>',
};
const applicationClosePanelStub = {
  props: ['items'],
  emits: ['update:selectedIds'],
  template:
    '<section class="application-close-panel-stub"><button @click="$emit(\'update:selectedIds\', [items[0].id])">Select app</button></section>',
};

function createRule(index: number, requiresAppClose = false): PresentedScanRuleResult {
  return {
    ruleId: `fixture.rule-${index}`,
    category: requiresAppClose ? 'application' : 'system',
    group: requiresAppClose ? 'application' : 'system',
    risk: 'safe',
    defaultSelected: true,
    recommendedSelected: true,
    bytes: index * 1024,
    fileCount: 1,
    available: true,
    selectable: true,
    status: requiresAppClose ? 'requiresClose' : 'found',
    runningProcesses: requiresAppClose ? [`fixture-${index}.exe`] : [],
    requiresAppClose,
    sources: [],
    sourceCount: 0,
    sourcesTruncated: false,
    scanElapsedMs: 1,
    name: `Cleanup rule ${index}`,
    categoryLabel: 'System',
    description: `Description ${index}`,
    impact: `Impact ${index}`,
  };
}

function mountDialog(ruleCount: number, requiresAppClose = false) {
  const rules = Array.from({ length: ruleCount }, (_, index) => createRule(index + 1, requiresAppClose));
  return mount(MdCleanupPlanDialog, {
    props: {
      modelValue: true,
      busy: false,
      rules,
      selectedBytes: rules.reduce((total, rule) => total + rule.bytes, 0),
      selectedItemCount: rules.length,
      leftoverApplicationCount: 0,
      leftoverBytes: 0,
      leftoverItemCount: 0,
      closingApplications: false,
      closeResult: null,
      applicationIcons: [],
    },
    global: {
      plugins: [i18n],
      stubs: {
        Button: { template: '<button><slot /></button>' },
        Dialog: passthroughStub,
        DialogDescription: passthroughStub,
        DialogTitle: passthroughStub,
        MdApplicationClosePanel: applicationClosePanelStub,
        MdDialogContent: dialogContentStub,
        MdDialogFooter: passthroughStub,
        MdDialogHeader: passthroughStub,
      },
    },
  });
}

describe('cleanup plan dialog component', () => {
  it('offers optional app closure for projects through the shared close panel', async () => {
    const wrapper = mountDialog(1);
    await wrapper.setProps({
      rules: [{ ...createRule(1, true), category: 'project', runningProcesses: ['Codex', 'ChatGPT'] }],
    });
    expect(wrapper.find('.manual-close-warning').exists()).toBe(false);
    expect(wrapper.find('.application-close-panel-stub').exists()).toBe(true);
    await wrapper.findAll('button').at(-1)!.trigger('click');
    expect(wrapper.emitted('closeApplications')).toBeUndefined();
    expect(wrapper.emitted('execute')).toHaveLength(1);
    await wrapper.get('.application-close-panel-stub button').trigger('click');
    await wrapper.findAll('button').at(-1)!.trigger('click');
    expect(wrapper.emitted('closeApplications')).toEqual([[['fixture.rule-1'], 'graceful']]);
    expect(wrapper.emitted('execute')).toHaveLength(1);
    wrapper.unmount();
  });
  it('keeps a long cleanup plan inside the bounded dialog content scroller', () => {
    const wrapper = mountDialog(12);

    expect(wrapper.get('.dialog-content-stub').attributes('data-dialog-height')).toBe('tall');
    expect(cleanupPlanDialogSource).toContain(':global(.cleanup-plan-dialog)');
    expect(wrapper.get('.plan-dialog-body').classes()).toContain('scrollbar-stable');
    expect(wrapper.get('.plan-dialog-body').findAll('.confirmation-item-list > div')).toHaveLength(12);
    wrapper.unmount();
  });

  it('expands the running applications and cleanup rules inside one shared content scroller', () => {
    const wrapper = mountDialog(12, true);
    const contentScroller = wrapper.get('.plan-dialog-body');

    expect(contentScroller.find('.application-close-panel-stub').exists()).toBe(true);
    expect(contentScroller.findAll('.confirmation-item-list > div')).toHaveLength(12);
    wrapper.unmount();
  });

  it('keeps a short cleanup plan content-sized', () => {
    const wrapper = mountDialog(2);

    expect(wrapper.get('.dialog-content-stub').attributes('data-dialog-height')).toBe('auto');
    wrapper.unmount();
  });
});
