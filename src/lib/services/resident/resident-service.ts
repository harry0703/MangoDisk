import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import type {
  ApplicationQuitStatus,
  ApplicationForceQuitResult,
  MemoryReleaseResult,
  ResidentDestination,
  ResidentDisplayStatus,
  ResidentPreferences,
  ResidentReading,
  ResidentPanelPage,
} from '@/lib/models/resident/resident';

export class ResidentService {
  static displayStatus(): Promise<ResidentDisplayStatus> {
    return invoke('resident_get_display_status');
  }
  static onDisplayStatus(callback: (status: ResidentDisplayStatus) => void): Promise<UnlistenFn> {
    return listen<ResidentDisplayStatus>('resident-display-status', event => callback(event.payload));
  }
  static reading(): Promise<ResidentReading> {
    return invoke('monitoring_get_reading');
  }
  static resetNetworkTraffic(interfaceId: string, startedAtMs: number): Promise<ResidentReading> {
    return invoke('monitoring_reset_network_traffic', { interfaceId, startedAtMs });
  }
  static refresh(): Promise<void> {
    return invoke('monitoring_refresh');
  }
  static releaseMemory(): Promise<MemoryReleaseResult> {
    return invoke('monitoring_release_memory');
  }
  static quitApplication(applicationId: string): Promise<ApplicationQuitStatus> {
    return invoke('monitoring_quit_application', { applicationId });
  }
  static forceQuitApplication(applicationId: string): Promise<ApplicationForceQuitResult> {
    return invoke('monitoring_force_quit_application', { applicationId });
  }
  static onPreferencesChanged(callback: () => void): Promise<UnlistenFn> {
    return listen<number>('resident-preferences-changed', callback);
  }
  static preferences(): Promise<ResidentPreferences> {
    return invoke('resident_get_preferences');
  }
  static savePreferences(preferences: ResidentPreferences): Promise<ResidentPreferences> {
    return invoke('resident_save_preferences', { preferences });
  }
  static defaultPreferences(): Promise<ResidentPreferences> {
    return invoke('resident_default_preferences');
  }
  static catalogue(): Promise<ResidentReading> {
    return invoke('resident_get_catalogue');
  }
  static panelPage(): Promise<ResidentPanelPage> {
    return invoke('resident_get_panel_page');
  }
  static selectPanelPage(page: ResidentPanelPage): Promise<void> {
    return invoke('resident_select_panel_page', { page });
  }
  static onPanelPage(handler: (page: ResidentPanelPage) => void): Promise<UnlistenFn> {
    return listen<ResidentPanelPage>('resident-panel-page', event => handler(event.payload));
  }
  static autostartEnabled(): Promise<boolean> {
    return invoke('resident_get_autostart');
  }
  static setAutostart(enabled: boolean): Promise<void> {
    return invoke('resident_set_autostart', { enabled });
  }
  static openPanel(): Promise<void> {
    return invoke('resident_open_panel');
  }
  static panelReady(): Promise<void> {
    return invoke('resident_panel_ready');
  }
  static hidePanel(): Promise<void> {
    return invoke('resident_hide_panel');
  }
  static openMain(destination: ResidentDestination): Promise<void> {
    return invoke('resident_open_main', { destination });
  }
  static quit(): Promise<void> {
    return invoke('resident_quit');
  }
  static onReading(handler: (reading: ResidentReading) => void): Promise<UnlistenFn> {
    // Scope high-frequency samples to this window; global listeners also receive
    // events targeted at another window and keep hidden pages updating.
    return getCurrentWindow().listen<ResidentReading>('resident-reading', event => handler(event.payload));
  }
  static onNavigate(handler: (destination: ResidentDestination) => void): Promise<UnlistenFn> {
    return listen<ResidentDestination>('resident-open-page', event => handler(event.payload));
  }
  static async onPanelVisibility(handler: (visible: boolean) => void): Promise<UnlistenFn> {
    const current = getCurrentWindow();
    let receivedEvent = false;
    const dispose = await current.listen<boolean>('resident-panel-visibility', event => {
      receivedEvent = true;
      handler(event.payload);
    });
    try {
      // Showing can precede focus or listener registration. Seed actual visibility,
      // but never let an older query overwrite a newer native show/hide event.
      const visible = await invoke<boolean>('resident_is_panel_visible');
      if (!receivedEvent) handler(visible);
      return dispose;
    } catch (error) {
      dispose();
      throw error;
    }
  }
}
