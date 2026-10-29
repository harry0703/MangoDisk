import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ApplicationWindowService } from '@/lib/services/shell/application-window-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

const { invoke, windowMock } = vi.hoisted(() => ({
  invoke: vi.fn(),
  windowMock: {
    show: vi.fn(),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    isMaximized: vi.fn(),
    onResized: vi.fn(),
    close: vi.fn(),
  },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => windowMock,
}));

describe('ApplicationWindowService', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    Object.values(windowMock).forEach(method => method.mockReset().mockResolvedValue(undefined));
    windowMock.isMaximized.mockResolvedValue(false);
    windowMock.onResized.mockResolvedValue(() => undefined);
  });

  it('delegates administrator restart without accepting a path and preserves cancellation or failure', async () => {
    invoke.mockResolvedValueOnce(false);
    expect(await ApplicationWindowService.restartAsAdministrator()).toBe(false);
    expect(invoke).toHaveBeenLastCalledWith('restart_application_as_administrator');
    expect(windowMock.close).not.toHaveBeenCalled();
    invoke.mockRejectedValueOnce({ code: 'operationBusy' });
    await expect(ApplicationWindowService.restartAsAdministrator()).rejects.toEqual({ code: 'operationBusy' });
    expect(windowMock.close).not.toHaveBeenCalled();
    invoke.mockResolvedValueOnce(true);
    expect(await ApplicationWindowService.restartAsAdministrator()).toBe(true);
  });

  it('passes scan authority for administrator rescan without accepting a path', async () => {
    invoke.mockResolvedValueOnce(true);
    expect(await ApplicationWindowService.restartAsAdministrator(7)).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith('restart_application_as_administrator', { analysisScanId: 7 });
  });

  it('hands readiness to Rust and returns navigation without showing the window itself', async () => {
    invoke.mockResolvedValueOnce('settings');
    expect(await ApplicationWindowService.showAfterMount()).toBe('settings');
    expect(invoke).toHaveBeenCalledWith('resident_main_ready');
    expect(windowMock.show).not.toHaveBeenCalled();
  });

  it.each([
    ['minimize', 'minimize', 'main_window_minimized'],
    ['toggleMaximize', 'toggleMaximize', 'main_window_maximize_toggled'],
    ['close', 'close', 'main_window_close_requested'],
  ] as const)('runs %s against the current native window', async (serviceMethod, windowMethod, event) => {
    const info = vi.spyOn(LoggerService, 'info').mockImplementation(() => undefined);

    await ApplicationWindowService[serviceMethod]();

    expect(windowMock[windowMethod]).toHaveBeenCalledOnce();
    expect(info).toHaveBeenCalledWith('application-window', event);
  });

  it('records native window action failures without rejecting the UI event', async () => {
    const error = vi.spyOn(LoggerService, 'error').mockImplementation(() => undefined);
    windowMock.toggleMaximize.mockRejectedValueOnce(new Error('window unavailable'));

    await expect(ApplicationWindowService.toggleMaximize()).resolves.toBeUndefined();

    expect(error).toHaveBeenCalledWith('application-window', 'main_window_maximize_toggle_failed', {
      error: 'window unavailable',
    });
  });

  it('keeps maximize state synchronized with native resize events', async () => {
    const info = vi.spyOn(LoggerService, 'info').mockImplementation(() => undefined);
    const unlisten = vi.fn();
    let resized: (() => void) | undefined;
    windowMock.isMaximized.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    windowMock.onResized.mockImplementationOnce(async handler => {
      resized = handler;
      return unlisten;
    });
    const states: boolean[] = [];

    const stop = await ApplicationWindowService.observeMaximized(maximized => states.push(maximized));
    expect(states).toEqual([true]);

    resized?.();
    await vi.waitFor(() => expect(states).toEqual([true, false]));

    stop();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(info).toHaveBeenCalledWith('application-window', 'main_window_maximize_observer_ready');
  });
});
