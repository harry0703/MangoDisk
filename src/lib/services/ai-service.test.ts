import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AiDelta } from '@/lib/models/ai';
import { AiService, AiSession } from './ai-service';
import { ClientRequestMetadataService } from './client-request-metadata-service';
import { AppDistributionService } from './app-distribution-service';

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: ipc.invoke,
  Channel: class {
    onmessage: ((delta: AiDelta) => void) | null = null;
  },
}));

vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn().mockResolvedValue('1.0.9') }));

beforeEach(() => {
  vi.restoreAllMocks();
  ipc.invoke.mockReset();
});

describe('AI IPC sessions', () => {
  it('passes typed free metadata to IPC without a header round trip', async () => {
    vi.spyOn(AppDistributionService, 'current').mockResolvedValue('installed');
    vi.spyOn(ClientRequestMetadataService, 'collect').mockResolvedValue({
      installId: 'fixture',
      locale: 'fr-FR',
      distribution: 'installed',
    });
    ipc.invoke.mockImplementation(async command => (command === 'ai_begin' ? 'operation' : {}));
    await new AiSession().run(null, 'fr-FR', vi.fn(), 'free');
    expect(ipc.invoke).toHaveBeenCalledWith(
      'ai_explain',
      expect.objectContaining({
        metadata: {
          installId: 'fixture',
          appVersion: '1.0.9',
          locale: 'fr-FR',
          distribution: 'installed',
          osVersion: 'unknown',
          timezone: 'UTC',
        },
        expectedMode: 'free',
      })
    );
  });

  it('refuses free requests with missing identity and still releases the reservation', async () => {
    vi.spyOn(AppDistributionService, 'current').mockResolvedValue('installed');
    vi.spyOn(ClientRequestMetadataService, 'collect').mockResolvedValue({ locale: 'en-US', distribution: 'installed' });
    ipc.invoke.mockImplementation(async command => (command === 'ai_begin' ? 'operation' : undefined));
    await expect(new AiSession().run(null, 'en-US', vi.fn(), 'free')).rejects.toBe('configurationUnavailable');
    expect(ipc.invoke.mock.calls.some(call => call[0] === 'ai_explain')).toBe(false);
    expect(ipc.invoke).toHaveBeenLastCalledWith('ai_cancel', { id: 'operation' });
  });
  it('reserves an ID before streaming and delivers deltas', async () => {
    ipc.invoke.mockImplementation(async (command, args) => {
      if (command === 'ai_begin') return 'operation';
      if (command === 'ai_explain') {
        args.onDelta.onmessage({ kind: 'reasoning', text: 'first' });
        args.onDelta.onmessage({ kind: 'text', text: 'second' });
        return { promptTokens: 3, completionTokens: 4 };
      }
    });
    const onDelta = vi.fn();
    await expect(new AiSession().run(null, 'en-US', onDelta)).resolves.toEqual({
      promptTokens: 3,
      completionTokens: 4,
    });
    expect(onDelta.mock.calls).toEqual([[{ kind: 'reasoning', text: 'first' }], [{ kind: 'text', text: 'second' }]]);
    expect(ipc.invoke).toHaveBeenLastCalledWith('ai_cancel', { id: 'operation' });
  });

  it('cancels before reservation completes without dispatching a paid request', async () => {
    let resolve!: (id: string) => void;
    ipc.invoke.mockImplementation(command =>
      command === 'ai_begin'
        ? new Promise<string>(r => {
            resolve = r;
          })
        : Promise.resolve()
    );
    const session = new AiSession();
    const pending = session.run(null, 'en-US', vi.fn());
    const assertion = expect(pending).rejects.toBe('cancelled');
    await session.cancel();
    resolve('reserved');
    await assertion;
    expect(ipc.invoke.mock.calls.some(call => call[0] === 'ai_explain')).toBe(false);
  });

  it('separates public settings from the secret-bearing editor configuration', async () => {
    ipc.invoke.mockResolvedValue(null);
    await AiService.settings();
    expect(ipc.invoke).toHaveBeenCalledWith('ai_get_settings');
    const state = { configuration: null, freeAvailable: true };
    ipc.invoke.mockResolvedValue(state);
    expect(await AiService.editorState()).toEqual(state);
    expect(ipc.invoke).toHaveBeenLastCalledWith('ai_get_configuration');
    await AiService.configuration();
    expect(ipc.invoke).toHaveBeenLastCalledWith('ai_get_configuration');
  });
});
