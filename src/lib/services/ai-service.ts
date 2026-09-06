import { Channel, invoke } from '@tauri-apps/api/core';
import type { AiConfiguration, AiConfigurationUpdate, AiDelta, AiContext, AiSettings, AiUsage } from '@/lib/models/ai';
import { LoggerService } from '@/lib/services/logger-service';

/** One owned IPC session; closing a dialog cancels its request, not another module's work. */
export class AiSession {
  private cancelled = false;
  private id: string | null = null;

  async run(context: AiContext | null, language: string, onDelta: (delta: AiDelta) => void): Promise<AiUsage> {
    this.id = await invoke<string>('ai_begin');
    try {
      if (this.cancelled) throw 'cancelled';
      const onDeltaChannel = new Channel<AiDelta>();
      onDeltaChannel.onmessage = text => {
        if (!this.cancelled) onDelta(text);
      };
      const result = await invoke<AiUsage>('ai_explain', {
        id: this.id,
        request: { context, language },
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
  static configuration(): Promise<AiConfiguration | null> {
    return invoke('ai_get_configuration');
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
}
