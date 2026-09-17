import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { APP_DISTRIBUTION_IDS, APP_UPDATE_ACTION_IDS, APP_UPDATE_STATUS_IDS } from '@/lib/models/app-update';
import { AppDistributionService } from '@/lib/services/app-distribution-service';
import { AppUpdateService } from '@/lib/services/app-update-service';
import { InstallationIdentityService } from '@/lib/services/installation-identity-service';
import { LinkService } from '@/lib/services/link-service';
import { LoggerService } from '@/lib/services/logger-service';

import { useAppUpdateStore } from './app-update-store';

describe('app update store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    vi.spyOn(InstallationIdentityService, 'getOrCreateInstallId').mockResolvedValue(
      '019c0b3d-a9ef-7d11-89a3-d5ea10df4001'
    );
    vi.spyOn(LoggerService, 'info').mockImplementation(() => undefined);
    vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    vi.spyOn(LoggerService, 'error').mockImplementation(() => undefined);
    vi.spyOn(AppUpdateService, 'currentVersion').mockResolvedValue('1.0.0');
    vi.spyOn(AppDistributionService, 'current').mockResolvedValue(APP_DISTRIBUTION_IDS.installed);
  });

  it('reads cached results when opening About and explicitly refreshes on request', async () => {
    const check = vi.spyOn(AppUpdateService, 'check').mockResolvedValue(null);
    const store = useAppUpdateStore();
    await store.check(true, false);
    expect(check).toHaveBeenLastCalledWith(APP_DISTRIBUTION_IDS.installed, false);
    expect(store.dialogOpen).toBe(true);
    await store.check(true);
    expect(check).toHaveBeenLastCalledWith(APP_DISTRIBUTION_IDS.installed, true);
  });

  it('continues checking when optional installation metadata is unavailable', async () => {
    vi.spyOn(InstallationIdentityService, 'getOrCreateInstallId').mockRejectedValue(new Error('store unavailable'));
    vi.spyOn(AppUpdateService, 'check').mockResolvedValue(null);
    const store = useAppUpdateStore();
    await store.check(true);
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.upToDate);
  });

  it('marks automatic update results as unread without interrupting the current task', async () => {
    vi.spyOn(AppUpdateService, 'check').mockResolvedValue({
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: 'Improvements',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    });
    const store = useAppUpdateStore();

    await store.check(false);

    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.available);
    expect(store.dialogOpen).toBe(false);
    expect(store.updateNoticeUnread).toBe(true);
    expect(store.update?.version).toBe('1.1.0');
  });

  it('retains a discovered update and open dialog when a background refresh fails', async () => {
    const update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    vi.spyOn(AppUpdateService, 'check').mockResolvedValueOnce(update).mockRejectedValueOnce(new Error('offline'));
    const store = useAppUpdateStore();
    await store.check(true);
    await store.check(false);
    expect(store.update).toEqual(update);
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.available);
    expect(store.dialogOpen).toBe(true);
    expect(store.checkError).toBe('');
  });

  it('opens manually discovered updates immediately without leaving an unread notice', async () => {
    vi.spyOn(AppUpdateService, 'check').mockResolvedValue({
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: 'Improvements',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    });
    const store = useAppUpdateStore();

    await store.check(true);

    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.available);
    expect(store.dialogOpen).toBe(true);
    expect(store.updateNoticeUnread).toBe(false);
  });

  it('keeps automatic check failures silent while exposing manual failures', async () => {
    vi.spyOn(AppUpdateService, 'check').mockRejectedValue(new Error('network unavailable'));
    const store = useAppUpdateStore();

    await store.check(false);
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.idle);
    expect(store.checkError).toBe('');

    await store.check(true);
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.error);
    expect(store.checkError).toBe('network unavailable');
    expect(store.dialogOpen).toBe(true);
  });

  it('keeps the about dialog open after reporting an up-to-date result', async () => {
    vi.spyOn(AppUpdateService, 'check').mockResolvedValue(null);
    const store = useAppUpdateStore();

    await store.check(true);

    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.upToDate);
    expect(store.dialogOpen).toBe(true);
    expect(store.updateNoticeUnread).toBe(false);
  });

  it('opens an available update from the about dialog without leaving an unread notice', () => {
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.available;
    store.updateNoticeUnread = true;

    store.showAbout();

    expect(store.dialogOpen).toBe(true);
    expect(store.updateNoticeUnread).toBe(false);
  });

  it('opens the about dialog and clears an unread update notice', () => {
    const store = useAppUpdateStore();
    store.updateNoticeUnread = true;

    store.showAbout();

    expect(store.dialogOpen).toBe(true);
    expect(store.updateNoticeUnread).toBe(false);
  });

  it('tracks download progress and waits for explicit installation', async () => {
    const download = vi.spyOn(AppUpdateService, 'download').mockImplementation(async onProgress => {
      onProgress({ downloadedBytes: 50, totalBytes: 100, finished: false });
      onProgress({ downloadedBytes: 100, totalBytes: 100, finished: true });
    });
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.available;
    store.dialogOpen = true;

    await store.download();

    expect(download).toHaveBeenCalledOnce();
    expect(store.downloadedBytes).toBe(100);
    expect(store.totalBytes).toBe(100);
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.downloaded);
  });

  it('allows the dialog to close while a download continues in the background', async () => {
    let finishDownload: (() => void) | undefined;
    vi.spyOn(AppUpdateService, 'download').mockImplementation(
      () => new Promise<void>(resolve => (finishDownload = resolve))
    );
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.available;
    store.dialogOpen = true;

    const pendingDownload = store.download();
    store.dismiss();

    expect(store.dialogOpen).toBe(false);
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.downloading);

    finishDownload?.();
    await pendingDownload;

    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.downloaded);
    expect(store.updateNoticeUnread).toBe(true);
  });

  it('keeps the checked update available when downloading fails', async () => {
    vi.spyOn(AppUpdateService, 'download').mockRejectedValue(new Error('network interrupted'));
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.available;

    await store.download();

    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.available);
    expect(store.failureStage).toBe('download');
    expect(store.actionError).toBe('network interrupted');
  });

  it('keeps a downloaded update ready when installation fails', async () => {
    vi.spyOn(AppUpdateService, 'installDownloaded').mockRejectedValue(new Error('signature rejected'));
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.downloaded;

    await store.installDownloaded();

    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.downloaded);
    expect(store.failureStage).toBe('install');
    expect(store.actionError).toBe('signature rejected');
    expect(store.dialogOpen).toBe(true);
  });

  it('offers restart instead of reinstalling when relaunch fails after installation', async () => {
    vi.spyOn(AppUpdateService, 'installDownloaded').mockResolvedValue();
    vi.spyOn(AppUpdateService, 'restartApplication').mockRejectedValue(new Error('restart unavailable'));
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.downloaded;

    await store.installDownloaded();

    expect(AppUpdateService.installDownloaded).toHaveBeenCalledOnce();
    expect(AppUpdateService.restartApplication).toHaveBeenCalledOnce();
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.restartRequired);
    expect(store.failureStage).toBe('restart');
    expect(store.actionError).toBe('restart unavailable');
    expect(store.dialogOpen).toBe(true);
  });

  it('retries only the application restart after installation completes', async () => {
    vi.spyOn(AppUpdateService, 'installDownloaded');
    vi.spyOn(AppUpdateService, 'restartApplication').mockRejectedValue(new Error('restart unavailable'));
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.restartRequired;

    await store.restartApplication();

    expect(AppUpdateService.installDownloaded).not.toHaveBeenCalled();
    expect(AppUpdateService.restartApplication).toHaveBeenCalledOnce();
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.restartRequired);
  });

  it('preserves a downloaded update when the user checks again', async () => {
    const check = vi.spyOn(AppUpdateService, 'check');
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.downloaded;

    await store.check(true);

    expect(check).not.toHaveBeenCalled();
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.downloaded);
    expect(store.dialogOpen).toBe(true);
  });

  it('preserves the restart-required state when the user checks again', async () => {
    const check = vi.spyOn(AppUpdateService, 'check');
    const store = useAppUpdateStore();
    store.update = {
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: '',
      action: APP_UPDATE_ACTION_IDS.automaticInstall,
    };
    store.status = APP_UPDATE_STATUS_IDS.restartRequired;

    await store.check(true);

    expect(check).not.toHaveBeenCalled();
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.restartRequired);
    expect(store.dialogOpen).toBe(true);
  });

  it('opens the portable download without entering the updater download flow', async () => {
    vi.mocked(AppDistributionService.current).mockResolvedValue(APP_DISTRIBUTION_IDS.portable);
    vi.spyOn(AppUpdateService, 'check').mockResolvedValue({
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: 'Improvements',
      action: APP_UPDATE_ACTION_IDS.manualDownload,
      manualDownloadUrl: 'https://mangodisk.app/api/updates/1.1.0/windows/x86_64/download?distribution=portable',
    });
    const automaticDownload = vi.spyOn(AppUpdateService, 'download');
    const open = vi.spyOn(LinkService, 'open').mockResolvedValue();
    const store = useAppUpdateStore();

    await store.check(true);
    await store.download();
    await store.openManualDownload();

    expect(store.distribution).toBe(APP_DISTRIBUTION_IDS.portable);
    expect(automaticDownload).not.toHaveBeenCalled();
    expect(open).toHaveBeenCalledWith(
      'https://mangodisk.app/api/updates/1.1.0/windows/x86_64/download?distribution=portable'
    );
    expect(store.status).toBe(APP_UPDATE_STATUS_IDS.available);
  });
});
