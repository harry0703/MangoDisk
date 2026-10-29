import { invoke } from '@tauri-apps/api/core';
import type { TaskbarMonitorCatalogue } from '@/lib/models/resident/taskbar-monitors';

export class TaskbarMonitorService {
  static catalogue(): Promise<TaskbarMonitorCatalogue> {
    return invoke('resident_get_taskbar_monitors');
  }
  static identify(): Promise<void> {
    return invoke('resident_identify_taskbar_monitors');
  }
  static openSettings(): Promise<void> {
    return invoke('resident_open_taskbar_settings');
  }
}
