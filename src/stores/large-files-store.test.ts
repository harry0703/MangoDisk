import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { LargeFileEntry, LargeFilesResult } from '@/lib/models/storage/large-file';
import { LargeFileService } from '@/lib/services/storage/large-file-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';
import { PermanentDeleteService } from '@/lib/services/filesystem/permanent-delete-service';

import { useAppStore } from './app-store';
import { useHistoryStore } from './history-store';
import { useLargeFilesStore } from './large-files-store';
import { useStorageScanPreferencesStore } from './storage-scan-preferences-store';

const removed: LargeFileEntry = {
  name: 'removed.bin',
  path: '/fixture/removed.bin',
  parentPath: '/fixture',
  bytes: 128,
  modifiedAtMs: 1,
};
const failed: LargeFileEntry = {
  name: 'failed.bin',
  path: '/fixture/failed.bin',
  parentPath: '/fixture',
  bytes: 256,
  modifiedAtMs: 2,
};

function createResult(): LargeFilesResult {
  return {
    scanId: 9,
    roots: ['/fixture'],
    scannedAtMs: 1,
    scanMode: 'complete',
    minimumBytes: 1,
    totalBytes: removed.bytes + failed.bytes,
    totalCount: 2,
    returnedCount: 2,
    truncated: false,
    skippedCount: 0,
    entries: [removed, failed],
  };
}

describe('large files store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    useStorageScanPreferencesStore().initialized = true;
  });

  it('returns partial deletion results without raising a second global error', async () => {
    const operation = {
      removedPaths: [removed.path],
      failed: [{ path: failed.path, message: 'fixture item failure' }],
      releasedBytes: removed.bytes,
    };
    vi.spyOn(PermanentDeleteService, 'deleteFiles').mockResolvedValue(operation);
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const warn = vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    const appStore = useAppStore();
    const refreshDisk = vi.spyOn(appStore, 'refreshSystemDisk').mockResolvedValue(true);
    const store = useLargeFilesStore();
    store.result = createResult();

    const result = await store.deleteManyPermanently([removed, failed]);

    expect(result).toEqual(operation);
    expect(appStore.errorCode).toBeNull();
    expect(refreshDisk).toHaveBeenCalledOnce();
    expect(store.result?.entries).toEqual([failed]);
    expect(warn).toHaveBeenCalledWith('large-files', 'delete_completed_with_failures', {
      removedCount: 1,
      failedCount: 1,
      releasedBytes: removed.bytes,
    });
  });

  it('removes already absent rows without reporting reclaimed storage or a failure', async () => {
    const operation = { removedPaths: [], alreadyAbsentPaths: [removed.path], failed: [], releasedBytes: 0 };
    vi.spyOn(PermanentDeleteService, 'deleteFiles').mockResolvedValue(operation);
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
    const warn = vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    const store = useLargeFilesStore();
    store.result = createResult();
    expect(await store.deleteManyPermanently([removed])).toEqual(operation);
    expect(store.result?.entries).toEqual([failed]);
    expect(store.result?.totalBytes).toBe(failed.bytes);
    expect(store.result?.totalCount).toBe(1);
    expect(warn).not.toHaveBeenCalled();
  });

  it('preserves retained scan totals when an edited file releases a different amount', async () => {
    const operation = { removedPaths: [removed.path], failed: [], releasedBytes: 1024 };
    vi.spyOn(PermanentDeleteService, 'deleteFiles').mockResolvedValue(operation);
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
    const store = useLargeFilesStore();
    store.result = createResult();
    const result = await store.deleteManyPermanently([removed]);
    expect(result?.releasedBytes).toBe(1024);
    expect(store.result?.totalBytes).toBe(failed.bytes);
    expect(store.result?.entries).toEqual([failed]);
  });

  it('rejects deletion while a scan is active', async () => {
    const remove = vi.spyOn(PermanentDeleteService, 'deleteFiles');
    const store = useLargeFilesStore();
    store.result = createResult();
    store.loading = true;

    const result = await store.deleteManyPermanently([removed]);

    expect(result).toBeUndefined();
    expect(remove).not.toHaveBeenCalled();
    expect(store.deleting).toBe(false);
  });

  it('discards a stale result before deletion when large-file exclusions change', async () => {
    const remove = vi.spyOn(PermanentDeleteService, 'deleteFiles');
    const store = useLargeFilesStore();
    store.result = createResult();
    useStorageScanPreferencesStore().folders = [{ path: '/fixture', scopes: ['largeFiles'] }];

    await store.deleteManyPermanently([removed]);

    expect(remove).not.toHaveBeenCalled();
    expect(store.result).toBeNull();
  });

  it('keeps a large-file result when only another scan scope changes', () => {
    const store = useLargeFilesStore();
    const result = createResult();
    store.result = result;
    useStorageScanPreferencesStore().folders = [{ path: '/fixture', scopes: ['duplicateFiles'] }];

    store.invalidateResultForExclusionChange();

    expect(store.result).toEqual(result);
  });

  it('filters the active scan without starting another filesystem scan', async () => {
    const source = createResult();
    const filtered = {
      ...source,
      scanId: 10,
      minimumBytes: 500,
      totalBytes: 0,
      totalCount: 0,
      returnedCount: 0,
      entries: [],
    };
    const filter = vi.spyOn(LargeFileService, 'filter').mockResolvedValue(filtered);
    const appStore = useAppStore();
    appStore.settings.largeFileMinimumBytes = 500;
    const store = useLargeFilesStore();
    store.result = source;

    await store.filter(500);

    expect(filter).toHaveBeenCalledWith(source.scanId, 500);
    expect(store.result).toEqual(filtered);
  });

  it('passes exclusions to Core and remembers the scan configuration', async () => {
    useStorageScanPreferencesStore().folders = [{ path: '/fixture/cache', scopes: ['largeFiles'] }];
    vi.spyOn(LargeFileService, 'listenProgress').mockResolvedValue(() => undefined);
    const find = vi.spyOn(LargeFileService, 'find').mockResolvedValue(createResult());
    const store = useLargeFilesStore();

    await store.find(['/fixture', '/other', '/fixture/nested'], 50, 'complete');

    expect(find).toHaveBeenCalledWith(
      ['/fixture', '/other', '/fixture/nested'],
      50,
      'complete',
      ['/fixture/cache'],
      []
    );
    expect(store.resultExcludedFolders).toEqual(['/fixture/cache']);
  });
  it('passes a parent and its mounted volume to Core without dropping either selection', async () => {
    const preferences = useStorageScanPreferencesStore();
    preferences.initialized = true;
    vi.spyOn(LargeFileService, 'listenProgress').mockResolvedValue(() => undefined);
    const find = vi.spyOn(LargeFileService, 'find').mockResolvedValue(createResult());
    await useLargeFilesStore().find(['/', '/Volumes/External'], 50, 'complete');
    expect(find).toHaveBeenCalledWith(['/', '/Volumes/External'], 50, 'complete', [], []);
  });
  it('does not start a scan with no selected locations', async () => {
    const find = vi.spyOn(LargeFileService, 'find');
    await useLargeFilesStore().find([], 50, 'complete');
    expect(find).not.toHaveBeenCalled();
  });
});
