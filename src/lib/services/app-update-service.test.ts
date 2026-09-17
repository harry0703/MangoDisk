import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { APP_DISTRIBUTION_IDS, APP_UPDATE_DOWNLOAD_TIMEOUT_MS } from '@/lib/models/app-update';
import { AppUpdateService } from '@/lib/services/app-update-service';

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  download: vi.fn(),
  install: vi.fn(),
  close: vi.fn(),
  relaunch: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn() }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: native.relaunch }));
vi.mock('@/lib/services/logger-service', () => ({ LoggerService: { warn: vi.fn() } }));
vi.mock('@tauri-apps/plugin-updater', () => ({
  Update: class {
    constructor(metadata: object) {
      Object.assign(this, metadata);
    }
    download = native.download;
    install = native.install;
    close = native.close;
  },
}));

const installed = APP_DISTRIBUTION_IDS.installed;
const metadata = {
  rid: 7,
  currentVersion: '1.0.0',
  version: '1.1.0',
  body: 'Release notes',
  date: '2026-07-31T00:00:00Z',
  rawJson: { url: 'https://mangodisk.app/api/updates/1.1.0/windows/x86_64/download?distribution=portable' },
};

describe('AppUpdateService', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    native.invoke.mockResolvedValue({ schemaVersion: 1, update: metadata });
    native.close.mockResolvedValue(undefined);
  });
  afterEach(async () => {
    await AppUpdateService.dispose();
  });

  it('acquires a native cached resource without a second updater check', async () => {
    await expect(AppUpdateService.check(installed, false)).resolves.toMatchObject({
      currentVersion: '1.0.0',
      version: '1.1.0',
      notes: 'Release notes',
      action: 'automaticInstall',
    });
    expect(native.invoke).toHaveBeenCalledExactlyOnceWith('acquire_app_update', { refresh: false });
    await AppUpdateService.check(installed, true);
    expect(native.invoke).toHaveBeenLastCalledWith('acquire_app_update', { refresh: true });
    expect(native.close).toHaveBeenCalledOnce();
  });

  it('shares concurrent resource acquisition', async () => {
    let resolve!: (value: object) => void;
    native.invoke.mockReturnValue(
      new Promise(done => {
        resolve = done;
      })
    );
    const first = AppUpdateService.check(installed, true);
    const second = AppUpdateService.check(installed, true);
    expect(first).toBe(second);
    resolve({ schemaVersion: 1, update: metadata });
    await first;
    expect(native.invoke).toHaveBeenCalledOnce();
  });

  it('retains the previous resource if native refresh fails', async () => {
    await AppUpdateService.check(installed, false);
    native.invoke.mockRejectedValueOnce(new Error('offline'));
    await expect(AppUpdateService.check(installed, true)).rejects.toThrow('offline');
    expect(native.close).not.toHaveBeenCalled();
    await AppUpdateService.download(() => undefined);
    expect(native.download).toHaveBeenCalledOnce();
  });

  it('clears the resource after a successful no-update response', async () => {
    await AppUpdateService.check(installed, false);
    native.invoke.mockResolvedValueOnce({ schemaVersion: 1, update: null });
    expect(await AppUpdateService.check(installed, true)).toBeNull();
    expect(native.close).toHaveBeenCalledOnce();
    await expect(AppUpdateService.download(() => undefined)).rejects.toThrow('No checked update');
  });

  it('keeps signed download and installation separate and releases installed resources', async () => {
    await AppUpdateService.check(installed, false);
    await expect(AppUpdateService.installDownloaded()).rejects.toThrow('No downloaded update');
    await AppUpdateService.download(() => undefined);
    expect(native.download).toHaveBeenCalledWith(expect.any(Function), { timeout: APP_UPDATE_DOWNLOAD_TIMEOUT_MS });
    expect(native.install).not.toHaveBeenCalled();
    await AppUpdateService.installDownloaded();
    expect(native.install).toHaveBeenCalledOnce();
    expect(native.close).toHaveBeenCalledOnce();
    expect(native.relaunch).not.toHaveBeenCalled();
    await AppUpdateService.restartApplication();
    expect(native.relaunch).toHaveBeenCalledOnce();
  });

  it('does not report installation failure when releasing an installed resource fails', async () => {
    await AppUpdateService.check(installed, false);
    await AppUpdateService.download(() => undefined);
    native.close.mockRejectedValueOnce(new Error('window resource already closed'));
    await expect(AppUpdateService.installDownloaded()).resolves.toBeUndefined();
  });

  it('returns a trusted portable URL without retaining an install resource', async () => {
    await expect(AppUpdateService.check(APP_DISTRIBUTION_IDS.portable, false)).resolves.toMatchObject({
      action: 'manualDownload',
      manualDownloadUrl: metadata.rawJson.url,
    });
    expect(native.close).toHaveBeenCalledOnce();
    await expect(AppUpdateService.download(() => undefined)).rejects.toThrow('No checked update');
  });

  it('rejects an untrusted portable URL and releases its resource', async () => {
    native.invoke.mockResolvedValueOnce({
      schemaVersion: 1,
      update: { ...metadata, rawJson: { url: 'https://example.com/app.exe' } },
    });
    await expect(AppUpdateService.check(APP_DISTRIBUTION_IDS.portable, false)).rejects.toThrow(
      'unsupported download URL'
    );
    expect(native.close).toHaveBeenCalledOnce();
  });
});
