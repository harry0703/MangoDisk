// @vitest-environment happy-dom
import { shallowMount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import type { PresentedCleanupScanResult } from '@/lib/models/cleanup';
import { useAiStore } from '@/stores/ai-store';
import { useCustomCleanupStore } from '@/stores/custom-cleanup-store';
import CleanupPage from './index.vue';

type PageProps = InstanceType<typeof CleanupPage>['$props'];
const scan: PresentedCleanupScanResult = {
  schemaVersion: '1.9',
  customScanId: null,
  scannedAtMs: 1,
  disk: { name: 'Fixture', mountPoint: '/fixture', totalBytes: 100, usedBytes: 50, availableBytes: 50 },
  rules: [],
  applicationIcons: [],
  warningCount: 0,
  safeBytes: 0,
  reclaimableBytes: 0,
  applicabilityElapsedMs: 0,
  applicableRuleCount: 0,
  filteredRuleCount: 0,
  inventoryApplicationCount: 0,
  inventoryProcessCount: 0,
  elapsedMs: 0,
};
const nativeChanges: Record<string, Partial<PageProps>> = {
  'scan replacement': { scan: { ...scan, scannedAtMs: 2 } },
  rescan: { busy: true, operation: 'scanning' },
  execution: { busy: true, operation: 'cleaning' },
  'application close': { closingApplications: true },
  'privileged scan': { privilegedScanRuleId: 'fixture-rule' },
  'leftover scan': { scanningLeftovers: true },
};

it.each(Object.keys(nativeChanges))('invalidates only cleanup AI after %s', async change => {
  const pinia = createPinia();
  useCustomCleanupStore(pinia).initialized = true;
  const ai = useAiStore(pinia);
  const stop = vi.spyOn(ai, 'stop').mockResolvedValue(undefined);
  ai.workspaces.cleanup.open = true;
  ai.workspaces.cleanup.minimized = true;
  ai.workspaces.startup.open = true;
  const wrapper = shallowMount(CleanupPage, {
    props: {
      busy: false,
      disk: scan.disk,
      disks: [],
      leftovers: null,
      leftoverResult: null,
      scanningLeftovers: false,
      deletingLeftovers: false,
      loadingMessage: '',
      operation: 'idle',
      progress: null,
      result: null,
      scan,
      scanScope: { mode: 'standard' },
      selectedBytes: 0,
      selectedRuleIds: [],
      sourceSelections: [],
      closingApplications: false,
      closeResult: null,
      privilegedScanRuleId: null,
    },
    global: { plugins: [pinia, i18n] },
  });
  try {
    // Pure selection edits do not make the observed scan snapshot stale.
    await wrapper.setProps({ selectedRuleIds: ['fixture-rule'], selectedBytes: 10 });
    expect(ai.workspaces.cleanup.open).toBe(true);
    expect(stop).not.toHaveBeenCalled();
    await wrapper.setProps(nativeChanges[change]!);
    expect(ai.workspaces.cleanup.open).toBe(false);
    expect(stop).toHaveBeenCalledExactlyOnceWith('cleanup');
    expect(ai.workspaces.startup.open).toBe(true);
  } finally {
    wrapper.unmount();
    stop.mockRestore();
  }
});
