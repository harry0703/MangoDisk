import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { Update, type DownloadEvent } from '@tauri-apps/plugin-updater';

import {
  APP_DISTRIBUTION_IDS,
  APP_UPDATE_ACTION_IDS,
  APP_UPDATE_DOWNLOAD_TIMEOUT_MS,
  type AppDistribution,
  type AppUpdateDownloadProgress,
  type AppUpdateInfo,
} from '@/lib/models/app-update';
import { LoggerService } from '@/lib/services/logger-service';

type AcquiredUpdate = {
  schemaVersion: 1;
  update: ConstructorParameters<typeof Update>[0] | null;
};

export class AppUpdateService {
  private static pendingUpdate: Update | null = null;
  private static checkPromise: Promise<AppUpdateInfo | null> | null = null;
  private static downloaded = false;

  static currentVersion(): Promise<string> {
    return getVersion();
  }

  static check(distribution: AppDistribution, refresh: boolean): Promise<AppUpdateInfo | null> {
    if (AppUpdateService.checkPromise) return AppUpdateService.checkPromise;

    AppUpdateService.checkPromise = AppUpdateService.performCheck(distribution, refresh).finally(() => {
      AppUpdateService.checkPromise = null;
    });
    return AppUpdateService.checkPromise;
  }

  static async download(onProgress: (progress: AppUpdateDownloadProgress) => void): Promise<void> {
    const update = AppUpdateService.pendingUpdate;
    if (!update) throw new Error('No checked update is available for download.');
    if (AppUpdateService.downloaded) return;

    let downloadedBytes = 0;
    let totalBytes: number | null = null;
    const reportProgress = (event: DownloadEvent) => {
      if (event.event === 'Started') {
        totalBytes = event.data.contentLength ?? null;
      } else if (event.event === 'Progress') {
        downloadedBytes += event.data.chunkLength;
      }
      onProgress({
        downloadedBytes,
        totalBytes,
        finished: event.event === 'Finished',
      });
    };

    await update.download(reportProgress, {
      timeout: APP_UPDATE_DOWNLOAD_TIMEOUT_MS,
    });
    AppUpdateService.downloaded = true;
  }

  static async installDownloaded(): Promise<void> {
    const update = AppUpdateService.pendingUpdate;
    if (!update || !AppUpdateService.downloaded) throw new Error('No downloaded update is available for installation.');

    await update.install();
    await AppUpdateService.dispose();
  }

  static async restartApplication(): Promise<void> {
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  }

  static async dispose(): Promise<void> {
    const update = AppUpdateService.pendingUpdate;
    AppUpdateService.pendingUpdate = null;
    AppUpdateService.downloaded = false;
    if (update) {
      try {
        await update.close();
      } catch (error) {
        LoggerService.warn('app-update', 'update_resource_release_failed', { error });
      }
    }
  }

  private static async performCheck(distribution: AppDistribution, refresh: boolean): Promise<AppUpdateInfo | null> {
    const result = await invoke<AcquiredUpdate>('acquire_app_update', { refresh });
    if (result.schemaVersion !== 1) throw new Error('Unsupported update resource schema.');
    const update = result.update ? new Update(result.update) : null;
    await AppUpdateService.dispose();
    if (!update) return null;

    const info = {
      currentVersion: update.currentVersion,
      version: update.version,
      date: update.date,
      notes: update.body?.trim() ?? '',
    };
    if (distribution === APP_DISTRIBUTION_IDS.portable) {
      try {
        return {
          ...info,
          action: APP_UPDATE_ACTION_IDS.manualDownload,
          manualDownloadUrl: AppUpdateService.resolvePortableDownloadUrl(update),
        };
      } finally {
        await update.close();
      }
    }

    AppUpdateService.pendingUpdate = update;
    return {
      ...info,
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
  }

  private static resolvePortableDownloadUrl(update: Update): string {
    const rawUrl = update.rawJson.url;
    if (typeof rawUrl !== 'string') throw new Error('The portable update response is missing its download URL.');

    const url = new URL(rawUrl);
    const expectedPath = `/api/updates/${encodeURIComponent(update.version)}/windows/x86_64/download`;
    if (
      url.origin !== 'https://mangodisk.app' ||
      url.pathname !== expectedPath ||
      url.searchParams.size !== 1 ||
      url.searchParams.get('distribution') !== APP_DISTRIBUTION_IDS.portable ||
      url.hash
    ) {
      throw new Error('The portable update response contains an unsupported download URL.');
    }

    return url.href;
  }
}
