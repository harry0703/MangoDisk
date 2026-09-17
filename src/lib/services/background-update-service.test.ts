import { beforeEach, expect, it, vi } from 'vitest';
import type { AppUpdateNotice } from '@/lib/models/app-update';
const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  focus: vi.fn(),
  stopEvent: vi.fn(),
  stopFocus: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: native.listen }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: native.focus }) }));
vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
import { BackgroundUpdateService } from './background-update-service';
let event: (event: { payload: AppUpdateNotice }) => void;
let focused: (event: { payload: boolean }) => void;
beforeEach(() => {
  vi.resetAllMocks();
  native.listen.mockImplementation(async (_name, handler) => {
    event = handler;
    return native.stopEvent;
  });
  native.focus.mockImplementation(async handler => {
    focused = handler;
    return native.stopFocus;
  });
});
it('keeps an event newer than the startup snapshot and releases listeners', async () => {
  let resolve!: (value: AppUpdateNotice) => void;
  native.invoke.mockReturnValue(
    new Promise(done => {
      resolve = done;
    })
  );
  const accept = vi.fn();
  const watching = BackgroundUpdateService.watch(accept);
  await vi.waitFor(() => expect(native.invoke).toHaveBeenCalled());
  event({ payload: { schemaVersion: 1, checked: true, revision: 2, version: '1.2.0' } });
  resolve({ schemaVersion: 1, checked: true, revision: 1, version: null });
  const stop = await watching;
  expect(accept).toHaveBeenCalledOnce();
  expect(accept).toHaveBeenCalledWith(expect.objectContaining({ version: '1.2.0' }));
  stop();
  event({ payload: { schemaVersion: 1, checked: true, revision: 3, version: null } });
  expect(accept).toHaveBeenCalledOnce();
  expect(native.stopEvent).toHaveBeenCalledOnce();
  expect(native.stopFocus).toHaveBeenCalledOnce();
});
it('rehydrates on focus and preserves the notice during IPC failure', async () => {
  native.invoke.mockResolvedValue({ schemaVersion: 1, checked: true, revision: 1, version: '1.2.0' });
  const accept = vi.fn();
  const stop = await BackgroundUpdateService.watch(accept);
  native.invoke.mockRejectedValueOnce(new Error('temporarily unavailable'));
  focused({ payload: true });
  await vi.waitFor(() => expect(native.invoke).toHaveBeenCalledTimes(2));
  expect(accept).toHaveBeenCalledOnce();
  native.invoke.mockResolvedValue({ schemaVersion: 1, checked: true, revision: 2, version: null });
  focused({ payload: true });
  await vi.waitFor(() => expect(accept).toHaveBeenCalledTimes(2));
  stop();
});
