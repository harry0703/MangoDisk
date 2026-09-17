import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { AppUpdateNotice } from '@/lib/models/app-update';
import { LoggerService } from '@/lib/services/logger-service';

/** Native discovery continues even when neither frontend window exists. */
export class BackgroundUpdateService {
  static async watch(handler: (notice: AppUpdateNotice) => void): Promise<() => void> {
    let disposed = false;
    let revision = -1;
    const accept = (notice: AppUpdateNotice) => {
      if (disposed || notice.schemaVersion !== 1 || notice.revision <= revision) return;
      revision = notice.revision;
      handler(notice);
    };
    const refresh = async () => {
      try {
        accept(await invoke<AppUpdateNotice>('get_app_update_notice'));
      } catch (error) {
        LoggerService.warn('app-update', 'update_notice_read_failed', { error });
      }
    };
    const stopEvent = await listen<AppUpdateNotice>('app-update-notice', event => accept(event.payload));
    let stopFocus: (() => void) | undefined;
    try {
      stopFocus = await getCurrentWindow().onFocusChanged(event => {
        if (event.payload) void refresh();
      });
      await refresh();
    } catch (error) {
      stopEvent();
      stopFocus?.();
      throw error;
    }
    return () => {
      disposed = true;
      stopEvent();
      stopFocus?.();
    };
  }
}
