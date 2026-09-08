import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises } from '@vue/test-utils';
import type { AiContext, AiDelta } from '@/lib/models/ai';
import { useAiStore } from './ai-store';

const mocks = vi.hoisted(() => ({
  run: vi.fn(),
  cancel: vi.fn(),
  settings: vi.fn(),
  quota: vi.fn(),
  configuration: vi.fn(),
  save: vi.fn(),
}));
vi.mock('@/lib/services/ai-service', () => ({
  AiService: { settings: mocks.settings, quota: mocks.quota, configuration: mocks.configuration, save: mocks.save },
  AiSession: class {
    run = mocks.run;
    cancel = mocks.cancel;
  },
}));
const context: AiContext = {
  schemaVersion: 2,
  platform: 'unknown',
  title: 'Browser cache',
  description: 'Generated cache',
  subject: {
    module: 'cleanup',
    impact: 'Rebuilt on next use',
    bytes: 20,
    itemCount: 1,
    requiresAppClose: true,
    scan: {
      ruleId: 'browser.cache',
      risk: 'safe',
      status: 'found',
      available: true,
      selectable: true,
      runningProcesses: [],
      sources: [],
      sourceCount: 0,
      sourcesTruncated: false,
    },
  },
};
const settings = {
  schemaVersion: 2,
  mode: 'custom',
  freeConsent: false,
  freeAvailable: false,
  endpoint: 'https://example.com/v1',
  model: 'test',
  hasKey: true,
  reasoning: 'default',
};
beforeEach(() => {
  setActivePinia(createPinia());
  vi.resetAllMocks();
  mocks.settings.mockResolvedValue(settings);
  mocks.run.mockImplementation(async (_context, _language, delta) => {
    delta({ kind: 'text', text: 'Purpose. Impact.' });
  });
});

describe('AI explanations', () => {
  it('does not send official requests when the build has no signing credentials', async () => {
    mocks.settings.mockResolvedValue({ ...settings, mode: 'free', freeConsent: true });
    await useAiStore().show(context, 'en-US');
    expect(mocks.quota).not.toHaveBeenCalled();
    expect(mocks.run).not.toHaveBeenCalled();
  });

  it('requires consent for free service and preserves custom credentials when consenting', async () => {
    mocks.settings.mockResolvedValue({ ...settings, mode: 'free', freeAvailable: true });
    mocks.configuration.mockResolvedValue({ ...settings, apiKey: 'synthetic-key' });
    const store = useAiStore();
    await store.show(context, 'en-US');
    expect(mocks.run).not.toHaveBeenCalled();
    mocks.settings.mockResolvedValue({ ...settings, mode: 'free', freeAvailable: true, freeConsent: true });
    await Promise.all([store.acceptFree('cleanup'), store.acceptFree('startup')]);
    expect(mocks.save).toHaveBeenCalledTimes(1);
    expect(mocks.save).toHaveBeenCalledWith(
      expect.objectContaining({ mode: 'free', freeConsent: true, apiKey: 'synthetic-key', endpoint: settings.endpoint })
    );
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(mocks.run.mock.calls[0]?.[3]).toBe('free');
  });

  it('deduplicates quota reads and refreshes again after a request completes', async () => {
    let resolve!: (value: unknown) => void;
    const snapshot = { remaining: 20 };
    mocks.quota.mockImplementationOnce(
      () =>
        new Promise(done => {
          resolve = done;
        })
    );
    mocks.quota.mockResolvedValueOnce({ remaining: 19 });
    const store = useAiStore();
    const first = store.refreshQuota('en-US');
    const second = store.refreshQuota('en-US');
    const afterRequest = store.refreshQuota('en-US', true);
    expect(mocks.quota).toHaveBeenCalledTimes(1);
    resolve(snapshot);
    await Promise.all([first, second, afterRequest]);
    expect(mocks.quota).toHaveBeenCalledTimes(2);
    expect(store.quota?.remaining).toBe(19);
    await store.refreshQuota('en-US');
    expect(mocks.quota).toHaveBeenCalledTimes(2);
  });

  it('retains quota on refresh failure and recovers from an initial failed read', async () => {
    const store = useAiStore();
    mocks.quota.mockRejectedValueOnce('connectionFailed');
    await store.refreshQuota('en-US');
    expect(store.quota).toBeNull();
    expect(store.quotaError).toBe('connectionFailed');
    mocks.quota.mockResolvedValueOnce({ remaining: 13, dailyLimit: 20 });
    await store.refreshQuota('en-US');
    expect(store.quota?.remaining).toBe(13);
    expect(store.quotaError).toBeNull();
    mocks.quota.mockRejectedValueOnce('connectionFailed');
    await store.refreshQuota('en-US', true);
    expect(store.quota?.remaining).toBe(13);
    expect(store.quotaError).toBe('connectionFailed');
  });

  it('coalesces a burst of forced refreshes but reads again for a later completion', async () => {
    const releases: Array<(value: unknown) => void> = [];
    mocks.quota.mockImplementation(() => new Promise(resolve => releases.push(resolve)));
    const store = useAiStore();
    const initial = store.refreshQuota('en-US');
    const burst = Array.from({ length: 5 }, () => store.refreshQuota('en-US', true));
    expect(mocks.quota).toHaveBeenCalledTimes(1);
    releases[0]!({ remaining: 20 });
    await flushPromises();
    expect(mocks.quota).toHaveBeenCalledTimes(2);
    // Another completion after the second GET began needs a genuinely newer read.
    const later = store.refreshQuota('en-US', true);
    releases[1]!({ remaining: 19 });
    await flushPromises();
    expect(mocks.quota).toHaveBeenCalledTimes(3);
    releases[2]!({ remaining: 18 });
    await Promise.all([initial, ...burst, later]);
    expect(store.quota?.remaining).toBe(18);
    expect(store.quotaPending).toBeNull();
  });

  it('coalesces forced refreshes after a failed read and allows recovery', async () => {
    let reject!: (reason: string) => void;
    mocks.quota.mockImplementationOnce(
      () =>
        new Promise((_resolve, fail) => {
          reject = fail;
        })
    );
    mocks.quota.mockResolvedValue({ remaining: 17 });
    const store = useAiStore();
    const initial = store.refreshQuota('en-US');
    const burst = Array.from({ length: 5 }, () => store.refreshQuota('en-US', true));
    reject('connectionFailed');
    await Promise.all([initial, ...burst]);
    expect(mocks.quota).toHaveBeenCalledTimes(2);
    expect(store.quota?.remaining).toBe(17);
    expect(store.quotaError).toBeNull();
  });

  it('preserves installation quota and its in-flight refresh when saving configuration', async () => {
    const store = useAiStore();
    mocks.quota.mockResolvedValueOnce({ remaining: 13, dailyLimit: 20 });
    await store.refreshQuota('en-US');
    let resolve!: (value: unknown) => void;
    mocks.quota.mockImplementationOnce(
      () =>
        new Promise(done => {
          resolve = done;
        })
    );
    const pending = store.refreshQuota('en-US', true);
    await store.configurationChanged();
    expect(store.quota?.remaining).toBe(13);
    resolve({ remaining: 12, dailyLimit: 20 });
    await pending;
    expect(store.quota?.remaining).toBe(12);
  });

  it('avoids known service shutdown requests and resumes after availability returns', async () => {
    const store = useAiStore();
    const freeSettings = { ...settings, mode: 'free', freeConsent: true, freeAvailable: true };
    mocks.settings.mockResolvedValue(freeSettings);
    mocks.quota.mockResolvedValueOnce({ remaining: 20, unavailableReason: 'AI_SERVICE_DISABLED' });
    await store.refreshQuota('en-US');
    await store.show(context, 'en-US');
    expect(mocks.run).not.toHaveBeenCalled();
    expect(store.workspaces.cleanup.error).toBe('freeUnavailable');
    mocks.quota.mockResolvedValue({ remaining: 20, unavailableReason: null });
    await store.refreshQuota('en-US', true);
    await store.generate('cleanup');
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(store.workspaces.cleanup.status).toBe('completed');
    expect(store.workspaces.cleanup.error).toBeNull();
  });

  it.each(['reopen', 'a-b-a', 'stop-and-reopen'])('waits for delayed cancellation before %s', async action => {
    let fail!: (error: string) => void;
    let delta!: (value: AiDelta) => void;
    mocks.run.mockImplementationOnce((_context, _language, callback) => {
      delta = callback;
      callback({ kind: 'text', text: 'Old partial' });
      return new Promise<void>((_resolve, reject) => {
        fail = reject;
      });
    });
    const store = useAiStore();
    const first = store.show(context, 'en-US');
    await flushPromises();
    let intermediate: Promise<void> | undefined;
    if (action === 'reopen') await store.close('cleanup');
    else if (action === 'a-b-a') intermediate = store.show({ ...context, title: 'B' }, 'en-US');
    else await store.stop('cleanup');
    const reopened = store.show(context, 'en-US');
    await flushPromises();
    expect(store.workspaces.cleanup.cancelling).toBe(true);
    expect(mocks.run).toHaveBeenCalledTimes(1);
    delta({ kind: 'text', text: ' Late old chunk' });
    expect(store.workspaces.cleanup.text).toBe('Old partial');
    fail('cancelled');
    await Promise.all([first, intermediate, reopened]);
    expect(mocks.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.run).toHaveBeenCalledTimes(2);
    expect(store.workspaces.cleanup.context?.title).toBe(context.title);
    expect(store.workspaces.cleanup.text).toBe('Purpose. Impact.');
    expect(store.workspaces.cleanup.status).toBe('completed');
    expect(store.workspaces.cleanup.cancelling).toBe(false);
  });

  it.each(['success', 'failure'])('does not restart a reclosed panel after late transport %s', async outcome => {
    let finish!: () => void;
    mocks.run.mockImplementationOnce((_context, _language, delta) => {
      delta({ kind: 'text', text: 'Retiring answer' });
      return new Promise<void>((resolve, reject) => {
        finish = () => (outcome === 'success' ? resolve() : reject('connectionFailed'));
      });
    });
    const store = useAiStore();
    const first = store.show(context, 'en-US');
    await flushPromises();
    await store.close('cleanup');
    const reopened = store.show(context, 'en-US');
    await store.close('cleanup');
    // A successful transport callback arriving after a stop is not a cache hit.
    finish();
    await Promise.all([first, reopened]);
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(store.workspaces.cleanup.open).toBe(false);
    expect(store.workspaces.cleanup.status).toBe('cancelled');
    expect(store.cache).toEqual({});
  });

  it('isolates simultaneous module streams, cancellation, failure and retry', async () => {
    const startup: AiContext = {
      ...context,
      title: 'Startup',
      subject: { module: 'startup', entries: [], omittedCount: 0 },
    };
    const streams = new Map<
      string,
      { delta: (value: AiDelta) => void; finish: () => void; fail: (error: string) => void }
    >();
    mocks.run.mockImplementation(
      (item: AiContext, _language, delta) =>
        new Promise<void>((finish, fail) => {
          streams.set(item.subject.module, { delta, finish, fail });
        })
    );
    const store = useAiStore();
    const cleanupPending = store.show(context, 'en-US');
    const startupPending = store.show(startup, 'zh-CN');
    await flushPromises();
    expect(mocks.run).toHaveBeenCalledTimes(2);
    expect(mocks.cancel).not.toHaveBeenCalled();
    const cleanup = streams.get('cleanup')!;
    const entry = streams.get('startup')!;
    cleanup.delta({ kind: 'reasoning', text: 'Cache thought' });
    entry.delta({ kind: 'reasoning', text: 'Startup thought' });
    cleanup.delta({ kind: 'text', text: 'Cache answer' });
    entry.delta({ kind: 'text', text: 'Startup answer' });
    store.minimize('cleanup');
    expect(store.workspaces.startup.minimized).toBe(false);
    expect(store.workspaces.cleanup.reasoning).toBe('Cache thought');
    expect(store.workspaces.startup.reasoning).toBe('Startup thought');
    mocks.cancel.mockImplementationOnce(async () => cleanup.fail('cancelled'));
    await store.close('cleanup');
    await cleanupPending;
    entry.delta({ kind: 'text', text: ' continues' });
    expect(store.workspaces.startup.text).toBe('Startup answer continues');
    expect(store.workspaces.startup.status).toBe('generating');
    expect(store.workspaces.startup.open).toBe(true);
    entry.fail('incompleteStream');
    await startupPending;
    expect(store.workspaces.startup.status).toBe('failed');
    mocks.run.mockImplementationOnce(async (_item, _language, delta) => delta({ kind: 'text', text: 'Retry answer' }));
    await store.generate('startup');
    expect(store.workspaces.startup.text).toBe('Retry answer');
    expect(store.workspaces.cleanup.text).toBe('Cache answer');
    expect(store.workspaces.cleanup.status).toBe('cancelled');
  });

  it('starts another module while settings IO is pending and ignores a closed module late read', async () => {
    let resolve!: (value: typeof settings) => void;
    mocks.settings.mockImplementationOnce(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    const store = useAiStore();
    const pending = store.show(context, 'en-US');
    await store.show({ ...context, subject: { module: 'startup', entries: [], omittedCount: 0 } }, 'en-US');
    expect(store.workspaces.startup.status).toBe('completed');
    await store.close('cleanup');
    resolve(settings);
    await pending;
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(store.workspaces.cleanup.loadingSettings).toBe(false);
    expect(store.workspaces.startup.open).toBe(true);
  });

  it('preserves completed module answers on settings changes without billable background restarts', async () => {
    const store = useAiStore();
    await store.show(context, 'en-US');
    await store.show({ ...context, subject: { module: 'startup', entries: [], omittedCount: 0 } }, 'en-US');
    await store.configurationChanged(null, 'startup');
    expect(mocks.run).toHaveBeenCalledTimes(3);
    expect(store.workspaces.cleanup.text).toBe('Purpose. Impact.');
    expect(store.workspaces.cleanup.open).toBe(true);
    await store.configurationChanged();
    expect(mocks.run).toHaveBeenCalledTimes(3);
  });

  it('streams and caches reasoning separately, clearing it on retry or a different item', async () => {
    mocks.run.mockImplementationOnce(async (_context, _language, delta) => {
      delta({ kind: 'reasoning', text: 'Inspecting the item' });
      delta({ kind: 'text', text: 'Final answer' });
    });
    const store = useAiStore();
    await store.show(context, 'en-US');
    expect(store.workspaces.cleanup.reasoning).toBe('Inspecting the item');
    expect(store.workspaces.cleanup.text).toBe('Final answer');
    await store.show(context, 'en-US');
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(store.workspaces.cleanup.reasoning).toBe('Inspecting the item');
    await store.show({ ...context, title: 'Another item' }, 'en-US');
    expect(store.workspaces.cleanup.reasoning).toBe('');
    await store.show(context, 'en-US');
    await store.generate('cleanup');
    expect(store.workspaces.cleanup.reasoning).toBe('');
  });

  it('retains reasoning-only failures for inspection without caching them as answers', async () => {
    mocks.run.mockImplementationOnce(async (_context, _language, delta) => {
      delta({ kind: 'reasoning', text: 'Incomplete thought' });
      throw 'emptyResponse';
    });
    const store = useAiStore();
    await store.show(context, 'en-US');
    expect(store.workspaces.cleanup.reasoning).toBe('Incomplete thought');
    expect(store.workspaces.cleanup.text).toBe('');
    expect(store.workspaces.cleanup.status).toBe('failed');
    expect(store.cache).toEqual({});
  });

  it('separates module caches and only dismisses the changed module', async () => {
    const store = useAiStore();
    await store.show(context, 'en-US');
    const maintenance: AiContext = {
      ...context,
      subject: {
        module: 'systemMaintenance',
        taskId: 'macos.maintenance.audio-service',
        status: 'available',
        riskLevel: 'standard',
        requiresRestart: false,
        requiresElevation: false,
        estimatedDurationSeconds: 1,
        diagnostic: null,
      },
    };
    await store.show(maintenance, 'en-US');
    expect(mocks.run).toHaveBeenCalledTimes(2);
    store.dismissModule('privacy');
    expect(store.open).toBe(true);
    store.dismissModule('systemMaintenance');
    expect(store.workspaces.systemMaintenance.open).toBe(false);
    expect(store.workspaces.cleanup.open).toBe(true);
    await store.show(context, 'en-US');
    expect(mocks.run).toHaveBeenCalledTimes(2);
  });

  it('automatically requests on click and reuses completed answers in the same language', async () => {
    const store = useAiStore();
    await store.show(context, 'en-US');
    expect(store.workspaces.cleanup.text).toBe('Purpose. Impact.');
    expect(store.workspaces.cleanup.status).toBe('completed');
    await store.show(context, 'en-US');
    expect(mocks.run).toHaveBeenCalledTimes(1);
    await store.show(context, 'ja-JP');
    expect(mocks.run).toHaveBeenCalledTimes(2);
  });

  it('does not send requests until configured, then resumes the open item', async () => {
    mocks.settings.mockResolvedValueOnce(null);
    const store = useAiStore();
    await store.show(context, 'en-US');
    expect(mocks.run).not.toHaveBeenCalled();
    await store.configurationChanged(null, 'cleanup');
    expect(mocks.run).toHaveBeenCalledTimes(1);
  });

  it('retains but never caches failed partial text and supports retry', async () => {
    mocks.run.mockImplementationOnce(async (_context, _language, delta) => {
      delta({ kind: 'text', text: 'Partial' });
      throw 'incompleteStream';
    });
    const store = useAiStore();
    await store.show(context, 'en-US');
    expect(store.workspaces.cleanup.text).toBe('Partial');
    expect(store.workspaces.cleanup.status).toBe('failed');
    expect(store.cache).toEqual({});
    await store.generate('cleanup');
    expect(store.workspaces.cleanup.status).toBe('completed');
    expect(store.workspaces.cleanup.text).toBe('Purpose. Impact.');
  });

  it('does not automatically retry a failed connection test after saving', async () => {
    const store = useAiStore();
    await store.show(context, 'en-US');
    await store.configurationChanged('unauthorized', 'cleanup');
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(store.cache).toEqual({});
    expect(store.workspaces.cleanup.error).toBe('unauthorized');
    expect(store.workspaces.cleanup.status).toBe('failed');
  });

  it('keeps streaming while minimized and repeated clicks only restore it', async () => {
    let finish!: () => void;
    let delta!: (delta: AiDelta) => void;
    mocks.run.mockImplementation((_context, _language, callback) => {
      delta = callback;
      return new Promise<void>(resolve => {
        finish = resolve;
      });
    });
    const store = useAiStore();
    const pending = store.show(context, 'en-US');
    await flushPromises();
    store.minimize('cleanup');
    delta({ kind: 'text', text: 'Background reply' });
    expect(store.workspaces.cleanup.minimized).toBe(true);
    expect(mocks.cancel).not.toHaveBeenCalled();
    await store.show(context, 'en-US');
    expect(store.workspaces.cleanup.minimized).toBe(false);
    expect(mocks.run).toHaveBeenCalledTimes(1);
    finish();
    await pending;
    expect(store.workspaces.cleanup.text).toBe('Background reply');
    expect(store.workspaces.cleanup.status).toBe('completed');
  });

  it('cancels the old stream before switching items without mixing partial answers', async () => {
    let reject!: (error: string) => void;
    mocks.run.mockImplementationOnce((_context, _language, delta) => {
      delta({ kind: 'text', text: 'Old partial' });
      return new Promise((_resolve, r) => {
        reject = r;
      });
    });
    mocks.cancel.mockImplementation(async () => reject('cancelled'));
    const store = useAiStore();
    const first = store.show(context, 'en-US');
    await flushPromises();
    await store.show({ ...context, title: 'Other cache' }, 'en-US');
    await first;
    expect(mocks.cancel).toHaveBeenCalledTimes(1);
    expect(store.workspaces.cleanup.context?.title).toBe('Other cache');
    expect(store.workspaces.cleanup.text).toBe('Purpose. Impact.');
    expect(Object.keys(store.cache)).toHaveLength(1);
  });

  it('closes and cancels the stream without caching partial output', async () => {
    let reject!: (error: string) => void;
    mocks.run.mockImplementation(
      () =>
        new Promise((_resolve, r) => {
          reject = r;
        })
    );
    mocks.cancel.mockImplementation(async () => reject('cancelled'));
    const store = useAiStore();
    const pending = store.show(context, 'en-US');
    await flushPromises();
    await store.close('cleanup');
    await pending;
    expect(store.open).toBe(false);
    expect(store.workspaces.cleanup.status).toBe('cancelled');
    expect(store.cache).toEqual({});
  });

  it('does not start after closing during configuration loading', async () => {
    let resolve!: (value: typeof settings) => void;
    mocks.settings.mockImplementation(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    const store = useAiStore();
    const pending = store.show(context, 'en-US');
    await store.close('cleanup');
    resolve(settings);
    await pending;
    expect(mocks.run).not.toHaveBeenCalled();
  });

  it('only starts the latest selection during rapid clicks', async () => {
    const store = useAiStore();
    await Promise.all([store.show(context, 'en-US'), store.show({ ...context, title: 'Latest' }, 'en-US')]);
    expect(mocks.run).toHaveBeenCalledTimes(1);
    expect(mocks.run.mock.calls[0]?.[0].title).toBe('Latest');
  });
});
