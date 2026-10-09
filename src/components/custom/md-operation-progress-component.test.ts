// @vitest-environment happy-dom

import { shallowMount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { i18n } from '@/i18n';
import type { TraversalProgress } from '@/lib/models/progress';
import { ICON_NAMES } from '@/lib/models/ui';

import MdOperationProgress from './md-operation-progress.vue';

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));

let monotonicMs = 100;
const wrappers: ReturnType<typeof shallowMount>[] = [];
const originalLocale = i18n.global.locale.value;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  vi.setSystemTime(new Date('2026-10-02T00:00:00Z'));
  monotonicMs = 100;
  vi.spyOn(performance, 'now').mockImplementation(() => monotonicMs);
  i18n.global.locale.value = 'en-US';
});

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount();
  i18n.global.locale.value = originalLocale;
  vi.restoreAllMocks();
  vi.useRealTimers();
});

function progress(operationId: number, elapsedMs: number): TraversalProgress {
  return {
    operationId,
    currentStage: 'analyzing',
    currentPath: 'C:\\fixture\\file.bin',
    itemsScanned: 1,
    bytesScanned: 64,
    completedSteps: 0,
    totalSteps: 0,
    foundItems: 0,
    foundBytes: 0,
    elapsedMs,
  };
}

function mountProgress(initial: TraversalProgress | null) {
  const wrapper = shallowMount(MdOperationProgress, {
    props: {
      title: 'Scanning',
      progress: initial,
      pathLabel: 'Current object',
      preparingText: 'Preparing',
      hint: 'Read-only scan',
      cancelable: true,
      cancelDisabled: false,
      iconName: ICON_NAMES.folder,
    },
    global: {
      plugins: [i18n],
      stubs: { Card: { template: '<div><slot /></div>' } },
    },
  });
  wrappers.push(wrapper);
  return wrapper;
}

function elapsedText(wrapper: ReturnType<typeof mountProgress>) {
  return wrapper.get('.progress-stats > span:last-child strong').text();
}

async function advance(milliseconds: number) {
  monotonicMs += milliseconds;
  await vi.advanceTimersByTimeAsync(milliseconds);
}

describe('operation progress elapsed time', () => {
  it.each([586_661_173, -586_661_173])('ignores a wall-clock adjustment of %i ms', async jump => {
    const wrapper = mountProgress(progress(1, 18_000));
    expect(elapsedText(wrapper)).toBe('18 sec');
    vi.setSystemTime(Date.now() + jump);
    await advance(1000);
    expect(elapsedText(wrapper)).toBe('19 sec');
    expect(wrapper.get('.path-meta').text()).toContain('19 sec');
    await wrapper.setProps({ progress: progress(1, 20_000) });
    await advance(1000);
    expect(elapsedText(wrapper)).toBe('21 sec');
  });

  it('keeps counting while preparing before the first backend event', async () => {
    const wrapper = mountProgress(null);
    vi.setSystemTime(Date.now() + 586_661_173);
    await advance(2000);
    expect(elapsedText(wrapper)).toBe('2 sec');
    await wrapper.setProps({ progress: progress(1, 1000) });
    expect(elapsedText(wrapper)).toBe('1 sec');
  });

  it('does not move backwards when backend events arrive late or out of order', async () => {
    const wrapper = mountProgress(progress(1, 18_000));
    await advance(4000);
    expect(elapsedText(wrapper)).toBe('22 sec');
    await wrapper.setProps({ progress: progress(1, 19_000) });
    expect(elapsedText(wrapper)).toBe('22 sec');
    await wrapper.setProps({ progress: progress(1, 17_000) });
    await advance(1000);
    expect(elapsedText(wrapper)).toBe('23 sec');
  });

  it('resets the elapsed anchor for a new operation and a return to preparation', async () => {
    const wrapper = mountProgress(progress(1, 18_000));
    await advance(1000);
    await wrapper.setProps({ progress: progress(2, 500) });
    await advance(1000);
    expect(elapsedText(wrapper)).toBe('1 sec');
    await wrapper.setProps({ progress: null });
    await advance(1000);
    expect(elapsedText(wrapper)).toBe('1 sec');
  });

  it('preserves cancellation and releases its interval when unmounted', async () => {
    const wrapper = mountProgress(progress(1, 0));
    const intervals = vi.getTimerCount();
    expect(intervals).toBeGreaterThan(0);
    wrapper.getComponent({ name: 'Button' }).vm.$emit('click');
    expect(wrapper.emitted('cancel')).toHaveLength(1);
    await wrapper.setProps({ cancelDisabled: true });
    expect(wrapper.getComponent({ name: 'Button' }).attributes('disabled')).toBeDefined();
    wrapper.unmount();
    wrappers.splice(wrappers.indexOf(wrapper), 1);
    expect(vi.getTimerCount()).toBe(intervals - 1);
  });
});
