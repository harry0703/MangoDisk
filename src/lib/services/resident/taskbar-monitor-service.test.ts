import { describe, expect, it, vi } from 'vitest';
import { TaskbarMonitorService } from './taskbar-monitor-service';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
describe('taskbar display protocol', () => {
  it('uses bounded native commands without accepting arbitrary paths or window handles', async () => {
    await TaskbarMonitorService.catalogue();
    await TaskbarMonitorService.identify();
    await TaskbarMonitorService.openSettings();
    expect(invoke.mock.calls).toEqual([
      ['resident_get_taskbar_monitors'],
      ['resident_identify_taskbar_monitors'],
      ['resident_open_taskbar_settings'],
    ]);
  });
});
