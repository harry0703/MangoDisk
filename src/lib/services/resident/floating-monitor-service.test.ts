import { beforeEach, describe, expect, it, vi } from 'vitest';
import { FloatingMonitorService } from './floating-monitor-service';
const native = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), windowListen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: native.listen }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ listen: native.windowListen }),
}));
beforeEach(() => vi.resetAllMocks());
describe('floating monitor transport', () => {
  it('scopes summary events to the current window and returns disposal', async () => {
    const stop = vi.fn();
    const handler = vi.fn();
    native.windowListen.mockResolvedValue(stop);
    expect(await FloatingMonitorService.onSnapshot(handler)).toBe(stop);
    expect(native.windowListen.mock.calls[0]![0]).toBe('resident-floating-snapshot');
    native.windowListen.mock.calls[0]![1]({ payload: { schemaVersion: 12 } });
    expect(handler).toHaveBeenCalledWith({ schemaVersion: 12 });
    expect(native.listen).not.toHaveBeenCalled();
  });
  it('reloads appearance only for application settings changes', async () => {
    const handler = vi.fn();
    const stop = vi.fn();
    native.listen.mockResolvedValue(stop);
    expect(await FloatingMonitorService.onAppearanceChanged(handler)).toBe(stop);
    const accept = native.listen.mock.calls[0]![1];
    accept({ payload: { key: 'resourceSortPreferences' } });
    expect(handler).not.toHaveBeenCalled();
    accept({ payload: { key: 'settings' } });
    expect(handler).toHaveBeenCalledOnce();
  });
  it('uses registered commands for lifecycle and explicit user interactions', async () => {
    await FloatingMonitorService.ready();
    await FloatingMonitorService.layout(null, { fontSize: 'standard', bold: false });
    await FloatingMonitorService.menu();
    await FloatingMonitorService.togglePage('memory');
    await FloatingMonitorService.drag();
    expect(native.invoke.mock.calls).toEqual([
      ['resident_floating_ready'],
      ['resident_floating_layout', { measurement: { items: null, fontSize: 'standard', bold: false } }],
      ['resident_floating_menu'],
      ['resident_floating_page', { page: 'memory' }],
      ['resident_floating_drag'],
    ]);
  });
});
