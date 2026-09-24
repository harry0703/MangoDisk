import { invoke } from '@tauri-apps/api/core';
import type { ResidentDestination } from '@/lib/models/resident';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { LoggerService } from './logger-service';

export class ApplicationWindowService {
  static async showAfterMount(): Promise<ResidentDestination | null> {
    // Rust owns presentation intent. Mounting a WebView alone must not reveal it.
    try {
      return await invoke<ResidentDestination | null>('resident_main_ready');
    } catch (error) {
      ApplicationWindowService.logActionFailure('main_window_ready_failed', error);
      return null;
    }
  }

  static async minimize(): Promise<void> {
    try {
      await getCurrentWindow().minimize();
      LoggerService.info('application-window', 'main_window_minimized');
    } catch (error) {
      ApplicationWindowService.logActionFailure('main_window_minimize_failed', error);
    }
  }

  static async toggleMaximize(): Promise<void> {
    try {
      await getCurrentWindow().toggleMaximize();
      LoggerService.info('application-window', 'main_window_maximize_toggled');
    } catch (error) {
      ApplicationWindowService.logActionFailure('main_window_maximize_toggle_failed', error);
    }
  }

  static async observeMaximized(onChange: (maximized: boolean) => void): Promise<() => void> {
    const window = getCurrentWindow();
    let disposed = false;

    const synchronize = async () => {
      try {
        const maximized = await window.isMaximized();
        if (!disposed) onChange(maximized);
      } catch (error) {
        ApplicationWindowService.logActionFailure('main_window_maximize_state_read_failed', error);
      }
    };

    let unlisten: UnlistenFn = () => {};
    try {
      // Desktop systems can change the window state through the titlebar, taskbar,
      // keyboard shortcuts, or system snap layouts. A resize notification is
      // the common native signal for every path, so the icon always reflects
      // the actual window state instead of the last button interaction.
      unlisten = await window.onResized(() => {
        void synchronize();
      });
      LoggerService.info('application-window', 'main_window_maximize_observer_ready');
    } catch (error) {
      ApplicationWindowService.logActionFailure('main_window_maximize_observer_failed', error);
    }

    await synchronize();
    return () => {
      disposed = true;
      unlisten();
    };
  }

  static async close(): Promise<void> {
    try {
      await getCurrentWindow().close();
      LoggerService.info('application-window', 'main_window_close_requested');
    } catch (error) {
      ApplicationWindowService.logActionFailure('main_window_close_failed', error);
    }
  }

  private static logActionFailure(event: string, error: unknown): void {
    LoggerService.error('application-window', event, {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}
