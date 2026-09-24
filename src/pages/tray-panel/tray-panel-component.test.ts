vi.mock('@/lib/services/operating-system-service', () => ({
  OperatingSystemService: { isLinux: vi.fn(() => false), isWindows: () => false },
}));
vi.mock('@/lib/services/memory-release-service', () => ({
  MemoryReleaseService: {
    onPreferences: vi.fn().mockResolvedValue(() => {}),
    preferences: vi.fn().mockResolvedValue({ revision: 0, automatic: false, exclusions: [] }),
  },
}));
import { emptyReadings } from '@/lib/utils/system-resources';
// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import type { Component } from 'vue';
import { createI18n } from 'vue-i18n';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import enUS from '@/locales/en-US.json';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';
import jaJP from '@/locales/ja-JP.json';
import koKR from '@/locales/ko-KR.json';

import TrayPanelPage from './index.vue';
import MemoryOverview from './components/md-memory-overview.vue';
import ResourceOverview from './components/md-resource-overview.vue';
import ApplicationList from './components/md-application-memory-list.vue';
import { FileManagerService } from '@/lib/services/file-manager-service';
import { FileIconService } from '@/lib/services/file-icon-service';
import { ResidentService } from '@/lib/services/resident-service';
import { OperatingSystemService } from '@/lib/services/operating-system-service';
import { ICON_NAMES } from '@/lib/models/ui';
import { useTrayPanelStore } from '@/stores/tray-panel-store';
import type { ResidentReading } from '@/lib/models/resident';

vi.mock('@/lib/services/resident-service', () => ({
  ResidentService: {
    onReading: vi.fn(),
    onFocusChanged: vi.fn(),
    onPanelMetric: vi.fn(),
    panelMetric: vi.fn(),
    selectMetric: vi.fn(),
    panelReady: vi.fn(),
    hidePanel: vi.fn(),
    openMain: vi.fn(),
    quit: vi.fn(),
    reading: vi.fn(),
    refresh: vi.fn(),
    releaseMemory: vi.fn(),
    quitApplication: vi.fn(),
  },
}));
vi.mock('@/stores/app-store', () => ({ useAppStore: () => ({ loadSettings: vi.fn().mockResolvedValue(undefined) }) }));
vi.mock('@/lib/services/file-icon-service', () => ({
  FileIconService: { peek: vi.fn(() => 'cached'), resolve: vi.fn() },
}));
vi.mock('@/lib/services/file-manager-service', () => ({ FileManagerService: { reveal: vi.fn() } }));
vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('@/lib/services/byte-size-service', () => ({ ByteSizeService: { memory: (bytes: number) => `${bytes} B` } }));

const memory = { totalBytes: 100, usedBytes: 40, freeBytes: 60, swapUsedBytes: 2, usedPercent: 40 };
const snapshot: ResidentReading = {
  revision: 1,
  ...emptyReadings(),
  memory: {
    status: 'ready',
    sampledAtMs: 1,
    value: {
      schemaVersion: 1,
      sampledAtMs: 1,
      memory,
      processes: {
        applications: [
          {
            id: 'browser',
            name: 'Browser',
            residentBytes: 20,
            processCount: 3,
            iconPath: '/Browser.app',
            isBundle: true,
            canQuit: true,
          },
        ],
        readableProcessCount: 3,
        omittedProcessCount: 1,
      },
    },
  },
};
const wrappers: ReturnType<typeof mount>[] = [];
function render(
  component: Component = TrayPanelPage,
  props: Record<string, unknown> = {},
  nativeIcons = false,
  messages = {}
) {
  const pinia = createPinia();
  const wrapper = mount(component, {
    props,
    attachTo: document.body,
    global: {
      plugins: [
        pinia,
        createI18n({
          legacy: false,
          missingWarn: false,
          fallbackWarn: false,
          messages: { en: messages },
          locale: 'en',
        }),
      ],
      stubs: { MdIcon: true, MdIconMangodisk: true, MdNativeFileIcon: !nativeIcons },
    },
  });
  wrappers.push(wrapper);
  return { wrapper, store: useTrayPanelStore(pinia) };
}

describe('monitoring panel interactions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(OperatingSystemService.isLinux).mockReturnValue(false);
    vi.mocked(FileIconService.peek).mockReturnValue('cached');
    vi.mocked(FileIconService.resolve).mockResolvedValue(null);
    vi.mocked(ResidentService.onReading).mockResolvedValue(vi.fn());
    vi.mocked(ResidentService.onFocusChanged).mockResolvedValue(vi.fn());
    vi.mocked(ResidentService.onPanelMetric).mockResolvedValue(vi.fn());
    vi.mocked(ResidentService.panelMetric).mockResolvedValue('memory');
    vi.mocked(ResidentService.selectMetric).mockResolvedValue();
    vi.mocked(ResidentService.reading).mockResolvedValue(snapshot);
  });
  afterEach(() => {
    wrappers.splice(0).forEach(wrapper => wrapper.unmount());
    vi.useRealTimers();
  });

  it('stops overview animation on native blur even when the document stays visible', async () => {
    vi.mocked(ResidentService.panelMetric).mockResolvedValue('cpu');
    const { wrapper } = render();
    await flushPromises();
    expect(wrapper.findAllComponents(ResourceOverview)).toHaveLength(4);
    const focus = vi.mocked(ResidentService.onFocusChanged).mock.calls[0]![0];
    expect(wrapper.findAllComponents(ResourceOverview).every(card => card.props('active') === false)).toBe(true);
    focus(true);
    await flushPromises();
    expect(wrapper.findAllComponents(ResourceOverview).every(card => card.props('active') === true)).toBe(true);
    focus(false);
    await flushPromises();
    expect(wrapper.findAllComponents(ResourceOverview).every(card => card.props('active') === false)).toBe(true);
  });

  it('moves keyboard focus with the selected resource tab', async () => {
    const { wrapper, store } = render();
    await flushPromises();
    expect(wrapper.get('[role="tabpanel"]').attributes('id')).toBe(
      wrapper.get('#metric-tab-memory').attributes('aria-controls')
    );
    expect(wrapper.get('[role="tabpanel"]').attributes('aria-labelledby')).toBe('metric-tab-memory');
    await wrapper.get('#metric-tab-memory').trigger('keydown', { key: 'ArrowRight' });
    expect(wrapper.findAll('[role=tab]')).toHaveLength(2);
    expect(wrapper.findAll('.resource-overview')).toHaveLength(4);
    expect(store.selectedMetric).toBe('cpu');
    expect(document.activeElement?.id).toBe('metric-tab-overview');
    expect(ResidentService.selectMetric).toHaveBeenCalledWith('cpu');
    expect(wrapper.get('[role="tabpanel"]').attributes('aria-labelledby')).toBe('metric-tab-overview');
  });

  it('retains the selected memory tab when the prewarmed panel is opened again', async () => {
    const { wrapper, store } = render();
    await flushPromises();
    await wrapper.get('#metric-tab-memory').trigger('click');
    expect(store.selectedMetric).toBe('memory');
    const focus = vi.mocked(ResidentService.onFocusChanged).mock.calls[0]![0];
    focus(true);
    await flushPromises();
    expect(wrapper.get('#metric-tab-memory').attributes('aria-selected')).toBe('true');
    expect(store.selectedMetric).toBe('memory');
  });

  it('keeps a newer native selection when the initial query arrives late', async () => {
    let finish!: (metric: 'memory') => void;
    vi.mocked(ResidentService.panelMetric).mockReturnValue(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const { store } = render();
    await flushPromises();
    vi.mocked(ResidentService.onPanelMetric).mock.calls[0]![0]('disk');
    finish('memory');
    await flushPromises();
    expect(store.selectedMetric).toBe('disk');
  });

  it('shows release feedback for three seconds after loading ends, then restores the action', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { wrapper, store } = render();
    await flushPromises();
    store.releasing = true;
    store.releaseResult = { schemaVersion: 1, status: 'completed', observedReductionBytes: 1 };
    await flushPromises();
    await vi.advanceTimersByTimeAsync(4000);
    expect(wrapper.get('.release-button').attributes('aria-busy')).toBe('true');
    store.releasing = false;
    await flushPromises();
    await vi.advanceTimersByTimeAsync(2999);
    expect(wrapper.get('.release-button [role="status"]').text()).toBe('monitoring.releaseButtonReduced');
    await vi.advanceTimersByTimeAsync(1);
    expect(wrapper.get('.release-button [role="status"]').text()).toBe('monitoring.release');
    expect(wrapper.get('.release-button').attributes('title')).toBeUndefined();
    expect(store.releaseResult).toBeNull();
  });

  it('keeps Linux memory monitoring while hiding unavailable release controls', async () => {
    vi.mocked(OperatingSystemService.isLinux).mockReturnValue(true);
    const { wrapper } = render();
    await flushPromises();

    expect(wrapper.find('.memory-overview').exists()).toBe(true);
    expect(wrapper.find('.release-button').exists()).toBe(false);
    expect(wrapper.find('.release-settings-entry').exists()).toBe(false);
  });

  it('cancels the old feedback timeout when another release starts', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { wrapper, store } = render();
    await flushPromises();
    store.releaseResult = { schemaVersion: 1, status: 'completed', observedReductionBytes: 1 };
    await flushPromises();
    await vi.advanceTimersByTimeAsync(2000);
    store.releasing = true;
    store.releaseResult = null;
    await flushPromises();
    await vi.advanceTimersByTimeAsync(500);
    store.releaseResult = { schemaVersion: 1, status: 'completed', observedReductionBytes: 2 };
    store.releasing = false;
    await flushPromises();
    await vi.advanceTimersByTimeAsync(2999);
    expect(store.releaseResult?.observedReductionBytes).toBe(2);
    expect(wrapper.get('.release-button').attributes('aria-busy')).toBe('false');
    await vi.advanceTimersByTimeAsync(1);
    expect(store.releaseResult).toBeNull();
  });

  it('clears feedback on reopening without interrupting a release and disposes its timeout', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { wrapper, store } = render();
    await flushPromises();
    const focus = vi.mocked(ResidentService.onFocusChanged).mock.calls[0]![0];
    store.releaseResult = { schemaVersion: 1, status: 'completed', observedReductionBytes: 1 };
    await flushPromises();
    const cached = vi.spyOn(store, 'load');
    focus(true);
    expect(cached).toHaveBeenCalledOnce();
    expect(store.releaseResult).toBeNull();
    store.releasing = true;
    focus(true);
    expect(store.releasing).toBe(true);
    store.releasing = false;
    store.releaseResult = { schemaVersion: 1, status: 'completed', observedReductionBytes: 2 };
    await flushPromises();
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(3000);
    expect(store.releaseResult?.observedReductionBytes).toBe(2);
  });

  it.each([
    ['en-US', enUS, 'Quit'],
    ['zh-CN', zhCN, '退出'],
    ['zh-TW', zhTW, '結束'],
    ['ja-JP', jaJP, '終了'],
    ['ko-KR', koKR, '종료'],
  ] as const)('renders the localized quit action in %s', async (_locale, messages, expected) => {
    const { wrapper } = render(TrayPanelPage, {}, false, messages);
    await flushPromises();
    expect(wrapper.get('.quit-shortcut').text()).toBe(expected);
    await wrapper.get('.quit-shortcut').trigger('click');
    expect(ResidentService.quit).toHaveBeenCalledOnce();
  });

  it('reveals independently while subscribing before loading cached data', async () => {
    const { wrapper } = render();
    await flushPromises();
    expect(vi.mocked(ResidentService.onReading).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(ResidentService.reading).mock.invocationCallOrder[0]!
    );
    expect(ResidentService.panelReady).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain('Browser');
    expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('40');
  });

  it('reveals immediately even when the initial reading is still pending', async () => {
    let finish!: (value: ResidentReading) => void;
    vi.mocked(ResidentService.reading).mockReturnValue(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const { wrapper } = render();
    await flushPromises();
    expect(ResidentService.panelReady).toHaveBeenCalledOnce();
    expect(wrapper.find('.monitor-loading').exists()).toBe(true);
    expect(wrapper.find('footer').exists()).toBe(true);
    finish(snapshot);
    await flushPromises();
    expect(wrapper.text()).toContain('Browser');
  });

  it('renders data before slow icons finish, then fills the existing row without reopening', async () => {
    let finish!: (value: string | null) => void;
    vi.mocked(FileIconService.peek).mockReturnValue(undefined);
    vi.mocked(FileIconService.resolve).mockReturnValue(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const { wrapper } = render(TrayPanelPage, {}, true);
    await flushPromises();
    expect(ResidentService.panelReady).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain('Browser');
    expect(wrapper.find('.directory-fallback').exists()).toBe(true);
    expect(wrapper.get('.directory-fallback md-icon-stub').attributes('name')).toBe(ICON_NAMES.linuxFolder);
    expect(wrapper.find('.native-file-icon img').exists()).toBe(false);
    finish('data:image/png;base64,icon');
    await flushPromises();
    expect(wrapper.get('.native-file-icon img').attributes('src')).toBe('data:image/png;base64,icon');
    expect(wrapper.find('.directory-fallback').exists()).toBe(false);
    expect(ResidentService.panelReady).toHaveBeenCalledOnce();
  });

  it('provides escape, error recovery, and footer navigation without a header', async () => {
    const { wrapper, store } = render();
    await flushPromises();
    expect(wrapper.find('header').exists()).toBe(false);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    store.fail('monitoring_subscription_failed');
    await flushPromises();
    await wrapper.get('[role="alert"] button').trigger('click');
    await wrapper.get('[aria-label="monitoring.settings"]').trigger('click');
    await wrapper.get('.open-main-shortcut').trigger('click');
    expect(ResidentService.hidePanel).toHaveBeenCalledOnce();
    expect(ResidentService.refresh).toHaveBeenCalledOnce();
    expect(vi.mocked(ResidentService.openMain).mock.calls).toEqual([['settings'], ['main']]);
    await wrapper.get('.quit-shortcut').trigger('click');
    expect(ResidentService.quit).toHaveBeenCalledOnce();
    expect(wrapper.get('footer').findAll('button')).toHaveLength(3);
    expect(wrapper.text()).not.toContain('monitoring.refreshCadence');
    expect(wrapper.text()).toContain('monitoring.quit');
    expect(wrapper.text()).not.toContain('monitoring.manageApplications');
  });

  it('keeps a stale reading visibly marked after sampling or action failure', async () => {
    const { wrapper, store } = render();
    await flushPromises();
    store.accept({ ...snapshot, revision: 2, memory: { ...snapshot.memory, status: 'failed' } });
    await flushPromises();
    expect(wrapper.get('.metric-stale').text()).toContain('systemStatus.failed');
    expect(wrapper.text()).toContain('Browser');
    vi.mocked(ResidentService.openMain).mockRejectedValueOnce(new Error('failed'));
    await wrapper.get('.open-main-shortcut').trigger('click');
    await flushPromises();
    expect(store.error).toBe(true);
  });

  it('reveals an error view even if native subscription fails', async () => {
    vi.mocked(ResidentService.onReading).mockRejectedValueOnce(new Error('subscription'));
    const { wrapper } = render();
    await flushPromises();
    expect(ResidentService.panelReady).toHaveBeenCalledOnce();
    expect(wrapper.find('[role="alert"]').exists()).toBe(true);
    await wrapper.get('[role="alert"] button').trigger('click');
    await flushPromises();
    expect(ResidentService.onReading).toHaveBeenCalledTimes(2);
    expect(ResidentService.refresh).toHaveBeenCalled();
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  });

  it('disposes a listener that resolves after unmount without reopening the window', async () => {
    const dispose = vi.fn();
    let finish!: (value: () => void) => void;
    vi.mocked(ResidentService.onReading).mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    const { wrapper } = render();
    wrapper.unmount();
    finish(dispose);
    await flushPromises();
    expect(dispose).toHaveBeenCalledOnce();
    expect(ResidentService.panelReady).toHaveBeenCalledOnce();
    expect(ResidentService.reading).not.toHaveBeenCalled();
  });
});

describe('memory presentation', () => {
  it('exposes an accessible usage meter and distinct available and swap values', () => {
    const { wrapper } = render(MemoryOverview, { memory });
    expect(wrapper.text()).toContain('60 B');
    expect(wrapper.text()).toContain('2 B');
    expect(wrapper.get('[role="progressbar"]').attributes('aria-valuemax')).toBe('100');
  });

  it('renders more than eight rows with proportional backgrounds and handles zero readings', async () => {
    const applications = Array.from({ length: 12 }, (_, index) => ({
      id: String(index),
      name: `App ${index}`,
      residentBytes: (12 - index) * 10,
      processCount: 2,
      iconPath: null,
      isBundle: false,
      canQuit: false,
    }));
    const { wrapper } = render(ApplicationList, {
      summary: { applications, readableProcessCount: 24, omittedProcessCount: 0 },
    });
    expect(wrapper.findAll('li')).toHaveLength(12);
    expect(wrapper.findAll('.memory-share')[0]!.attributes('style')).toContain('width: 100%');
    expect(wrapper.findAll('.memory-share')[6]!.attributes('style')).toContain('width: 50%');
    expect(wrapper.findAll('.memory-share').every(bar => bar.attributes('aria-hidden') === 'true')).toBe(true);
    expect(wrapper.find('small').exists()).toBe(false);
    expect(wrapper.find('.list-note').exists()).toBe(false);
    await wrapper.setProps({
      summary: {
        applications: [{ ...applications[0], residentBytes: 0 }],
        readableProcessCount: 1,
        omittedProcessCount: 0,
      },
    });
    expect(wrapper.get('.memory-share').attributes('style')).toContain('width: 0%');
  });
  it('keeps expanded details attached to an identity across ranking updates and closes exited rows', async () => {
    const first = snapshot.memory.value!.processes!.applications[0]!;
    const second = { ...first, id: 'second', name: 'Second', iconPath: null, canQuit: false };
    const summary = { applications: [first, second], readableProcessCount: 4, omittedProcessCount: 0 };
    const { wrapper } = render(ApplicationList, { summary });
    await wrapper.findAll('.application-row')[0]!.trigger('click');
    expect(wrapper.get('.application-details').find('.detail-name').exists()).toBe(false);
    await wrapper.setProps({ summary: { ...summary, applications: [second, { ...first, residentBytes: 5 }] } });
    expect(wrapper.findAll('.application-row')[1]!.attributes('aria-expanded')).toBe('true');
    expect(wrapper.get('.application-details').find('.detail-name').exists()).toBe(false);
    await wrapper.findAll('.application-row')[0]!.trigger('click');
    expect(wrapper.findAll('.application-details')).toHaveLength(1);
    expect(wrapper.get('.application-details').text()).toContain('monitoring.locationUnavailable');
    expect(wrapper.find('.reveal-button').exists()).toBe(false);
    await wrapper.setProps({ summary: { ...summary, applications: [first] } });
    expect(wrapper.find('.application-details').exists()).toBe(false);
  });

  it('reveals the selected image through the shared adapter and keeps errors local and retryable', async () => {
    const { wrapper } = render(ApplicationList, { summary: snapshot.memory.value!.processes });
    await wrapper.get('.application-row').trigger('click');
    expect(wrapper.get('.application-row').attributes('aria-controls')).toBe(
      wrapper.get('.application-details').attributes('id')
    );
    expect(wrapper.get('.application-path').text()).toBe('/Browser.app');
    let finish!: () => void;
    vi.mocked(FileManagerService.reveal).mockImplementationOnce(
      () =>
        new Promise(resolve => {
          finish = resolve;
        })
    );
    await wrapper.get('.reveal-button').trigger('click');
    expect(wrapper.get('.reveal-button').attributes('disabled')).toBeDefined();
    finish();
    await flushPromises();
    expect(FileManagerService.reveal).toHaveBeenCalledWith('/Browser.app');
    vi.mocked(FileManagerService.reveal).mockRejectedValueOnce(new Error('missing'));
    await wrapper.get('.reveal-button').trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe('monitoring.revealFailed');
    vi.mocked(FileManagerService.reveal).mockResolvedValueOnce();
    await wrapper.get('.reveal-button').trigger('click');
    await flushPromises();
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    await wrapper.get('.application-row').trigger('click');
    expect(wrapper.find('.application-details').exists()).toBe(false);
  });

  it('requests normal quit once while pending and retains the row until a fresh sample removes it', async () => {
    const { wrapper } = render(ApplicationList, { summary: snapshot.memory.value!.processes }, false, enUS);
    await wrapper.get('.application-row').trigger('click');
    expect(wrapper.text().match(/Browser/g)).toHaveLength(2); // Row name and executable path only.
    expect(wrapper.find('.detail-name').exists()).toBe(false);
    let finish!: (status: 'requested') => void;
    vi.mocked(ResidentService.quitApplication).mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve;
      })
    );
    await wrapper.get('.quit-application-button').trigger('click');
    expect(wrapper.get('.quit-application-button').attributes('disabled')).toBeDefined();
    expect(wrapper.get('.quit-application-button').attributes('aria-busy')).toBe('true');
    await wrapper.get('.quit-application-button').trigger('click');
    expect(ResidentService.quitApplication).toHaveBeenCalledTimes(1);
    expect(ResidentService.quitApplication).toHaveBeenCalledWith('browser');
    finish('requested');
    await flushPromises();
    expect(wrapper.get('.quit-application-button [role="status"]').text()).toBe('Request sent');
    expect(
      wrapper.findAllComponents({ name: 'MdTooltip' }).some(hint => hint.props('text') === 'Quit request sent')
    ).toBe(true);
    expect(wrapper.find('.application-details > [role="status"]').exists()).toBe(false);
    expect(wrapper.findAll('li')).toHaveLength(1);
    await wrapper.setProps({ summary: { applications: [], readableProcessCount: 0, omittedProcessCount: 0 } });
    expect(wrapper.find('li').exists()).toBe(false);
  });

  it('keeps rejected quit requests retryable and hides the action for ineligible rows', async () => {
    const summary = snapshot.memory.value!.processes!;
    const { wrapper } = render(ApplicationList, { summary });
    await wrapper.get('.application-row').trigger('click');
    vi.mocked(ResidentService.quitApplication).mockRejectedValueOnce(new Error('OS failed'));
    await wrapper.get('.quit-application-button').trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe('monitoring.quitButtonFailed');
    expect(wrapper.get('.quit-application-button').attributes('disabled')).toBeUndefined();
    vi.mocked(ResidentService.quitApplication).mockResolvedValueOnce('unsupported');
    await wrapper.get('.quit-application-button').trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe('monitoring.quitButtonUnsupported');
    await wrapper.setProps({ summary: { ...summary, applications: [{ ...summary.applications[0], canQuit: false }] } });
    expect(wrapper.find('.quit-application-button').exists()).toBe(false);
  });

  it('distinguishes loading from an empty readable process list', async () => {
    const { wrapper } = render(ApplicationList, { summary: null });
    expect(wrapper.text()).toContain('monitoring.loadingApplications');
    await wrapper.setProps({ summary: { applications: [], readableProcessCount: 0, omittedProcessCount: 10 } });
    expect(wrapper.text()).toContain('monitoring.noApplications');
    expect(wrapper.text()).not.toContain('monitoring.loadingApplications');
  });
});

describe('compact memory overview', () => {
  afterEach(() => wrappers.splice(0).forEach(wrapper => wrapper.unmount()));
  it('omits explanatory paragraphs and exposes a single release action', async () => {
    const { wrapper } = render(MemoryOverview, { memory });
    expect(wrapper.find('details').exists()).toBe(false);
    expect(wrapper.text()).not.toContain('monitoring.memoryHint');
    expect(wrapper.text()).not.toContain('monitoring.releaseHint');
    await wrapper.get('.release-button').trigger('click');
    expect(wrapper.emitted('release')).toHaveLength(1);
    await wrapper.setProps({ releasing: true });
    expect(wrapper.get('.release-button').attributes('disabled')).toBeDefined();
    expect(wrapper.find('.release-button .animate-spin').exists()).toBe(true);
    expect(wrapper.get('.release-button [role="status"]').text()).toBe('monitoring.releasing');
  });
  it.each([
    ['completed', 30, 'monitoring.releaseObserved'],
    ['completed', 0, 'monitoring.releaseNoChange'],
    ['completed', null, 'monitoring.releaseUnmeasured'],
    ['cancelled', null, 'monitoring.releaseCancelled'],
    ['failed', null, 'monitoring.releaseFailed'],
    ['unsupported', null, 'monitoring.releaseUnsupported'],
    ['busy', null, 'monitoring.releaseBusy'],
  ])('renders %s with reduction %s truthfully', (status, observedReductionBytes, key) => {
    const { wrapper } = render(MemoryOverview, {
      memory,
      releaseResult: { schemaVersion: 1, status, observedReductionBytes },
    });
    expect(wrapper.get('.sr-only[role="status"]').text()).toBe(key);
    expect(wrapper.get('.release-button [role="status"]').classes()).not.toContain('sr-only');
    expect(wrapper.getComponent({ name: 'MdTooltip' }).props('text')).toBe(key);
    expect(wrapper.get('.release-button').attributes('title')).toBeUndefined();
    expect(wrapper.find('.release-result').exists()).toBe(false);
  });
  it('wires the release button through the store', async () => {
    vi.mocked(ResidentService.reading).mockResolvedValue(snapshot);
    vi.mocked(ResidentService.releaseMemory).mockResolvedValue({
      schemaVersion: 1,
      status: 'cancelled',
      observedReductionBytes: null,
    });
    const { wrapper } = render();
    await flushPromises();
    await wrapper.get('.release-button').trigger('click');
    await flushPromises();
    expect(ResidentService.releaseMemory).toHaveBeenCalledWith();
    expect(wrapper.text()).toContain('monitoring.releaseCancelled');
  });
});
