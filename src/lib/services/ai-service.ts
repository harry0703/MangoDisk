import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AiConfiguration,
  AiConfigurationUpdate,
  AiDelta,
  AiContext,
  AiSettings,
  AiUsage,
  AiServiceMode,
  AiClientMetadata,
  AiQuota,
  AiEditorState,
  AiPreferences,
  InstalledLocalModel,
} from '@/lib/models/ai';
import { getVersion } from '@tauri-apps/api/app';
import { ClientRequestMetadataService } from '@/lib/services/client-request-metadata-service';
import { AppDistributionService } from '@/lib/services/app-distribution-service';
import { LoggerService } from '@/lib/services/logger-service';

/** One owned IPC session; closing a dialog cancels its request, not another module's work. */
export class AiSession {
  private cancelled = false;
  private id: string | null = null;

  async run(
    context: AiContext | null,
    language: string,
    onDelta: (delta: AiDelta) => void,
    mode: AiServiceMode = 'custom'
  ): Promise<AiUsage> {
    this.id = await invoke<string>('ai_begin');
    try {
      if (this.cancelled) throw 'cancelled';
      const metadata = mode === 'free' ? await AiService.metadata(language) : null;
      if (this.cancelled) throw 'cancelled';
      const onDeltaChannel = new Channel<AiDelta>();
      onDeltaChannel.onmessage = text => {
        if (!this.cancelled) onDelta(text);
      };
      const result = await invoke<AiUsage>('ai_explain', {
        id: this.id,
        request: { context, language },
        metadata,
        expectedMode: mode,
        onDelta: onDeltaChannel,
      });
      if (this.cancelled) throw 'cancelled';
      return result;
    } finally {
      try {
        await this.cancel();
      } catch {
        LoggerService.warn('ai', 'request_release_failed');
      }
    }
  }

  async cancel(): Promise<void> {
    this.cancelled = true;
    if (this.id !== null) await invoke<void>('ai_cancel', { id: this.id });
  }
}

export class AiService {
  static preferences(): Promise<AiPreferences> {
    return invoke('ai_get_preferences');
  }
  static setEnabled(enabled: boolean): Promise<AiPreferences> {
    return invoke('ai_set_enabled', { enabled });
  }
  static async metadata(language: string): Promise<AiClientMetadata> {
    const distribution = await AppDistributionService.current();
    const metadata = await ClientRequestMetadataService.collect(language, distribution, source => {
      LoggerService.warn('ai', 'request_metadata_unavailable', { source });
    });
    const installId = metadata.installId;
    if (!installId) throw 'configurationUnavailable';
    return {
      installId,
      appVersion: await getVersion(),
      locale: metadata.locale,
      distribution: metadata.distribution,
      osVersion: metadata.osVersion || 'unknown',
      timezone: metadata.timezone || 'UTC',
    };
  }
  static async quota(language: string, isCurrent: () => boolean): Promise<AiQuota> {
    const metadata = await this.metadata(language);
    // Metadata collection can span a disable/re-enable cycle. The caller owns
    // that lifecycle; reject stale work before IPC admits a new network request.
    if (!isCurrent()) {
      LoggerService.info('ai', 'quota_request_cancelled_before_dispatch');
      throw 'cancelled';
    }
    return invoke('ai_get_quota', { metadata });
  }
  static editorState(): Promise<AiEditorState> {
    return invoke('ai_get_configuration');
  }
  static async configuration(): Promise<AiConfiguration | null> {
    return (await this.editorState()).configuration;
  }
  static settings(): Promise<AiSettings | null> {
    return invoke('ai_get_settings');
  }
  static save(update: AiConfigurationUpdate): Promise<AiSettings> {
    return invoke('ai_save_settings', { update });
  }
  static delete(): Promise<void> {
    return invoke('ai_delete_settings');
  }

  /** Enumerates locally installed AI models (e.g. Ollama repositories). */
  static listLocalModels(): Promise<InstalledLocalModel[]> {
    return invoke('ai_list_local_models');
  }
}
