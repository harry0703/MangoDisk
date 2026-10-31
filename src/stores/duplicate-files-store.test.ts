import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  DUPLICATE_ENTRY_DELETE_POLICIES,
  DUPLICATE_GROUP_KINDS,
  DUPLICATE_SCAN_LOCATION_MODES,
  type DuplicateFilesResult,
  type DuplicateGroup,
  type DuplicateGroupPage,
} from '@/lib/models/storage/duplicate-file';
import type { PermanentDeleteBatchResult } from '@/lib/models/filesystem/permanent-delete';
import { FILE_CATEGORY_IDS } from '@/lib/models/filesystem/file-category';
import { DuplicateFileService } from '@/lib/services/storage/duplicate-file-service';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

import { useAppStore } from './app-store';
import { useDuplicateFilesStore } from './duplicate-files-store';
import { useHistoryStore } from './history-store';
import { useStorageScanPreferencesStore } from './storage-scan-preferences-store';

function createGroup(id: string, name: string): DuplicateGroup {
  return {
    id,
    hash: `hash-${id}`,
    kind: DUPLICATE_GROUP_KINDS.file,
    bytesPerFile: 1024,
    fileCountPerEntry: 1,
    reclaimableBytes: 1024,
    entries: [
      {
        name,
        path: `/one/${name}`,
        parentPath: '/one',
        bytes: 1024,
        allocatedBytes: 1024,
        modifiedAtMs: 1,
        deletePolicy: DUPLICATE_ENTRY_DELETE_POLICIES.cleanable,
      },
      {
        name,
        path: `/two/${name}`,
        parentPath: '/two',
        bytes: 1024,
        allocatedBytes: 1024,
        modifiedAtMs: 2,
        deletePolicy: DUPLICATE_ENTRY_DELETE_POLICIES.cleanable,
      },
    ],
  };
}

function createResult(groups: DuplicateGroup[]): DuplicateFilesResult {
  return {
    scanId: 7,
    roots: ['/fixture'],
    protectedRoots: [],
    scannedAtMs: 1,
    scannedFileCount: 6,
    skippedCount: 0,
    duplicateFileCount: groups.length * 2,
    totalDuplicateBytes: groups.length * 2048,
    reclaimableBytes: groups.length * 1024,
    totalGroupCount: 3,
    returnedGroupCount: 3,
    truncated: false,
    groups,
  };
}

describe('duplicate files store pagination', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    useStorageScanPreferencesStore().initialized = true;
  });

  it('skips unrelated pages until the selected category receives a group', async () => {
    const initialGroup = createGroup('initial', 'document.pdf');
    const archiveGroup = createGroup('archive', 'archive.zip');
    const audioGroup = createGroup('audio', 'recording.mp3');
    const page = vi
      .spyOn(DuplicateFileService, 'page')
      .mockResolvedValueOnce({ scanId: 7, offset: 1, nextOffset: 2, totalCount: 3, groups: [archiveGroup] })
      .mockResolvedValueOnce({ scanId: 7, offset: 2, nextOffset: null, totalCount: 3, groups: [audioGroup] });
    const store = useDuplicateFilesStore();
    store.result = createResult([initialGroup]);
    store.resultComplete = true;
    store.nextPageOffset = 1;

    await store.loadMore(FILE_CATEGORY_IDS.audio);

    expect(page).toHaveBeenCalledTimes(2);
    expect(store.result?.groups).toEqual([initialGroup, archiveGroup, audioGroup]);
    expect(store.nextPageOffset).toBeNull();
    expect(store.loadingMore).toBe(false);
  });

  it('loads only one page when every category is shown', async () => {
    const initialGroup = createGroup('initial', 'document.pdf');
    const archiveGroup = createGroup('archive', 'archive.zip');
    const page = vi.spyOn(DuplicateFileService, 'page').mockResolvedValue({
      scanId: 7,
      offset: 1,
      nextOffset: 2,
      totalCount: 3,
      groups: [archiveGroup],
    });
    const store = useDuplicateFilesStore();
    store.result = createResult([initialGroup]);
    store.resultComplete = true;
    store.nextPageOffset = 1;

    await store.loadMore(FILE_CATEGORY_IDS.all);

    expect(page).toHaveBeenCalledOnce();
    expect(store.result?.groups).toEqual([initialGroup, archiveGroup]);
    expect(store.nextPageOffset).toBe(2);
  });

  it('loads every native result page for one complete smart selection', async () => {
    const initialGroup = createGroup('initial', 'document.pdf');
    const archiveGroup = createGroup('archive', 'archive.zip');
    const audioGroup = createGroup('audio', 'recording.mp3');
    const page = vi
      .spyOn(DuplicateFileService, 'page')
      .mockResolvedValueOnce({ scanId: 7, offset: 1, nextOffset: 2, totalCount: 3, groups: [archiveGroup] })
      .mockResolvedValueOnce({ scanId: 7, offset: 2, nextOffset: null, totalCount: 3, groups: [audioGroup] });
    const store = useDuplicateFilesStore();
    store.result = createResult([initialGroup]);
    store.resultComplete = true;
    store.nextPageOffset = 1;

    await store.loadMore(FILE_CATEGORY_IDS.all, true);

    expect(page).toHaveBeenCalledTimes(2);
    expect(store.result?.groups).toEqual([initialGroup, archiveGroup, audioGroup]);
    expect(store.nextPageOffset).toBeNull();
  });

  it('reconciles an already absent duplicate without a global error', async () => {
    const group = createGroup('missing', 'document.pdf');
    const absent = group.entries[0];
    const operation = { removedPaths: [], alreadyAbsentPaths: [absent.path], failed: [], releasedBytes: 0 };
    vi.spyOn(DuplicateFileService, 'deletePermanently').mockResolvedValue(operation);
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
    const store = useDuplicateFilesStore();
    store.result = createResult([group]);
    store.resultComplete = true;
    expect(await store.deletePermanently([absent])).toEqual(operation);
    expect(store.result?.groups).toEqual([]);
    expect(useAppStore().errorCode).toBeNull();
  });

  it.each(['during-delete', 'failed-during-delete', 'after-delete', 'failed-after-delete'] as const)(
    'discards an in-flight page received %s and keeps the next cursor usable',
    async timing => {
      const initial = createGroup('initial', 'document.pdf');
      const next = createGroup('next', 'archive.zip');
      const removed = initial.entries[0];
      let finishPage: (page: DuplicateGroupPage) => void = () => undefined;
      let failPage: (error: Error) => void = () => undefined;
      let finishDelete: (result: PermanentDeleteBatchResult) => void = () => undefined;
      const page = vi.spyOn(DuplicateFileService, 'page').mockImplementationOnce(
        () =>
          new Promise((resolve, reject) => {
            finishPage = resolve;
            failPage = reject;
          })
      );
      vi.spyOn(DuplicateFileService, 'deletePermanently').mockImplementationOnce(
        () =>
          new Promise(resolve => {
            finishDelete = resolve;
          })
      );
      vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
      vi.spyOn(useAppStore(), 'refreshSystemDisk').mockResolvedValue(true);
      const store = useDuplicateFilesStore();
      store.result = createResult([initial]);
      store.resultComplete = true;
      store.nextPageOffset = 1;
      const loading = store.loadMore();
      const deleting = store.deletePermanently([removed]);
      const operation = { removedPaths: [removed.path], failed: [], releasedBytes: removed.allocatedBytes };
      const response = { scanId: 7, offset: 1, nextOffset: null, totalCount: 2, groups: [next] };
      if (timing === 'during-delete' || timing === 'failed-during-delete') {
        if (timing === 'failed-during-delete') failPage(new Error('obsolete page failure'));
        else finishPage(response);
        await loading;
        finishDelete(operation);
        await deleting;
      } else {
        finishDelete(operation);
        await deleting;
        if (timing === 'failed-after-delete') failPage(new Error('obsolete page failure'));
        else finishPage(response);
        await loading;
      }
      expect(store.result?.groups).toEqual([]);
      expect(store.nextPageOffset).toBe(0);
      expect(store.loadingMore).toBe(false);
      expect(store.resultComplete).toBe(true);
      expect(useAppStore().errorCode).toBeNull();
      page.mockResolvedValueOnce({ ...response, offset: 0 });
      await store.loadMore();
      expect(page).toHaveBeenLastCalledWith(7, 0, 40);
      expect(store.result?.groups).toEqual([next]);
      expect(store.nextPageOffset).toBeNull();
    }
  );

  it('keeps a current pagination failure visible and revokes deletion authority', async () => {
    vi.spyOn(DuplicateFileService, 'page').mockRejectedValue(new Error('current page failure'));
    const store = useDuplicateFilesStore();
    store.result = createResult([createGroup('initial', 'document.pdf')]);
    store.resultComplete = true;
    store.nextPageOffset = 1;
    await store.loadMore();
    expect(store.resultComplete).toBe(false);
    expect(store.nextPageOffset).toBeNull();
    expect(useAppStore().errorCode).not.toBeNull();
  });

  it.each(['success', 'failure'] as const)('discards an old page %s during a same-scope refresh', async outcome => {
    const initial = createGroup('initial', 'document.pdf');
    const replacement = createGroup('replacement', 'recording.mp3');
    let finishPage: (page: DuplicateGroupPage) => void = () => undefined;
    let failPage: (error: Error) => void = () => undefined;
    let finishScan: (result: DuplicateFilesResult) => void = () => undefined;
    vi.spyOn(DuplicateFileService, 'page').mockImplementationOnce(
      () =>
        new Promise((resolve, reject) => {
          finishPage = resolve;
          failPage = reject;
        })
    );
    vi.spyOn(DuplicateFileService, 'listenProgress').mockResolvedValue(vi.fn());
    vi.spyOn(DuplicateFileService, 'listenGroups').mockResolvedValue(vi.fn());
    const find = vi.spyOn(DuplicateFileService, 'find').mockImplementationOnce(
      () =>
        new Promise(resolve => {
          finishScan = resolve;
        })
    );
    const store = useDuplicateFilesStore();
    store.result = createResult([initial]);
    store.resultComplete = true;
    store.nextPageOffset = 1;
    const loading = store.loadMore();
    const refresh = store.find(
      [{ path: '/fixture', mode: DUPLICATE_SCAN_LOCATION_MODES.cleanable }],
      useAppStore().settings.duplicateFileMinimumBytes
    );
    await vi.waitFor(() => expect(find).toHaveBeenCalledOnce());
    if (outcome === 'failure') failPage(new Error('the old result session expired'));
    else finishPage({ scanId: 7, offset: 1, nextOffset: null, totalCount: 2, groups: [replacement] });
    await loading;
    const whileRefreshing = {
      groups: store.result?.groups,
      resultComplete: store.resultComplete,
      error: useAppStore().errorCode,
    };
    finishScan({ ...createResult([replacement]), scanId: 8 });
    await refresh;
    expect(whileRefreshing).toEqual({ groups: [initial], resultComplete: true, error: null });
    expect(store.result?.scanId).toBe(8);
    expect(store.result?.groups).toEqual([replacement]);
  });

  it.each(['success', 'failure'] as const)(
    'keeps old rows read-only after a failed refresh and late page %s',
    async outcome => {
      const initial = createGroup('initial', 'document.pdf');
      let finishPage: (page: DuplicateGroupPage) => void = () => undefined;
      let failPage: (error: Error) => void = () => undefined;
      vi.spyOn(DuplicateFileService, 'page').mockImplementationOnce(
        () =>
          new Promise((resolve, reject) => {
            finishPage = resolve;
            failPage = reject;
          })
      );
      vi.spyOn(DuplicateFileService, 'listenProgress').mockResolvedValue(vi.fn());
      vi.spyOn(DuplicateFileService, 'listenGroups').mockResolvedValue(vi.fn());
      const scanError = new Error('the replacement scan failed');
      vi.spyOn(DuplicateFileService, 'find').mockRejectedValue(scanError);
      const remove = vi.spyOn(DuplicateFileService, 'deletePermanently');
      const report = vi.spyOn(useAppStore(), 'reportError');
      const store = useDuplicateFilesStore();
      store.result = createResult([initial]);
      store.resultComplete = true;
      store.nextPageOffset = 1;
      const loading = store.loadMore();
      await store.find(
        [{ path: '/fixture', mode: DUPLICATE_SCAN_LOCATION_MODES.cleanable }],
        useAppStore().settings.duplicateFileMinimumBytes
      );
      const authorityAfterFailure = store.resultComplete;
      await store.deletePermanently([initial.entries[0]]);
      if (outcome === 'failure') failPage(new Error('the old session expired'));
      else
        finishPage({
          scanId: 7,
          offset: 1,
          nextOffset: null,
          totalCount: 2,
          groups: [createGroup('late', 'archive.zip')],
        });
      await loading;
      expect(authorityAfterFailure).toBe(false);
      expect(store.result?.groups).toEqual([initial]);
      expect(store.resultComplete).toBe(false);
      expect(store.nextPageOffset).toBeNull();
      expect(remove).not.toHaveBeenCalled();
      expect(report).toHaveBeenCalledExactlyOnceWith(scanError);
    }
  );

  it('preserves old result authority when refresh fails before invoking Core', async () => {
    vi.spyOn(DuplicateFileService, 'listenProgress').mockRejectedValue(new Error('subscription unavailable'));
    vi.spyOn(DuplicateFileService, 'listenGroups').mockResolvedValue(vi.fn());
    const find = vi.spyOn(DuplicateFileService, 'find');
    const store = useDuplicateFilesStore();
    store.result = createResult([createGroup('initial', 'document.pdf')]);
    store.resultComplete = true;
    store.nextPageOffset = 1;
    await store.find(
      [{ path: '/fixture', mode: DUPLICATE_SCAN_LOCATION_MODES.cleanable }],
      useAppStore().settings.duplicateFileMinimumBytes
    );
    expect(find).not.toHaveBeenCalled();
    expect(store.resultComplete).toBe(true);
    expect(store.nextPageOffset).toBe(1);
  });

  it('returns partial deletion results without raising a second global error', async () => {
    const group = createGroup('partial-delete', 'archive.zip');
    const removed = group.entries[0]!;
    const failed = group.entries[1]!;
    const operation = {
      removedPaths: [removed.path],
      failed: [{ path: failed.path, message: 'fixture item failure' }],
      releasedBytes: removed.bytes,
    };
    vi.spyOn(DuplicateFileService, 'deletePermanently').mockResolvedValue(operation);
    vi.spyOn(useHistoryStore(), 'load').mockResolvedValue();
    const warn = vi.spyOn(LoggerService, 'warn').mockImplementation(() => undefined);
    const appStore = useAppStore();
    const refreshDisk = vi.spyOn(appStore, 'refreshSystemDisk').mockResolvedValue(true);
    const store = useDuplicateFilesStore();
    store.result = createResult([group]);
    store.resultComplete = true;

    const result = await store.deletePermanently([removed, failed]);

    expect(result).toEqual(operation);
    expect(appStore.errorCode).toBeNull();
    expect(refreshDisk).toHaveBeenCalledOnce();
    expect(warn).toHaveBeenCalledWith('duplicate-files', 'delete_completed_with_failures', {
      removedCount: 1,
      failedCount: 1,
      releasedBytes: removed.bytes,
    });
  });

  it('rejects deletion while a scan is active', async () => {
    const group = createGroup('active-scan', 'document.pdf');
    const remove = vi.spyOn(DuplicateFileService, 'deletePermanently');
    const store = useDuplicateFilesStore();
    store.result = createResult([group]);
    store.resultComplete = true;
    store.loading = true;

    const result = await store.deletePermanently([group.entries[0]!]);

    expect(result).toBeUndefined();
    expect(remove).not.toHaveBeenCalled();
    expect(store.deleting).toBe(false);
  });

  it('discards a stale result before deletion when duplicate exclusions change', async () => {
    const group = createGroup('stale', 'document.pdf');
    const remove = vi.spyOn(DuplicateFileService, 'deletePermanently');
    const store = useDuplicateFilesStore();
    store.result = createResult([group]);
    store.resultComplete = true;
    useStorageScanPreferencesStore().folders = [{ path: '/one', scopes: ['duplicateFiles'] }];

    await store.deletePermanently([group.entries[0]!]);

    expect(remove).not.toHaveBeenCalled();
    expect(store.result).toBeNull();
    expect(store.resultComplete).toBe(false);
  });

  it('keeps a duplicate result when only another scan scope changes', () => {
    const store = useDuplicateFilesStore();
    const result = createResult([createGroup('current', 'document.pdf')]);
    store.result = result;
    useStorageScanPreferencesStore().folders = [{ path: '/one', scopes: ['largeFiles'] }];

    store.invalidateResultForExclusionChange();

    expect(store.result).toEqual(result);
  });

  it('passes shared exclusions to Core and records the result configuration', async () => {
    const preferences = useStorageScanPreferencesStore();
    preferences.initialized = true;
    preferences.folders = [{ path: '/fixture/cache', scopes: ['duplicateFiles'] }];
    vi.spyOn(DuplicateFileService, 'listenProgress').mockResolvedValue(vi.fn());
    vi.spyOn(DuplicateFileService, 'listenGroups').mockResolvedValue(vi.fn());
    const find = vi.spyOn(DuplicateFileService, 'find').mockResolvedValue(createResult([]));
    const locations = [{ path: '/fixture', mode: DUPLICATE_SCAN_LOCATION_MODES.cleanable }];
    const store = useDuplicateFilesStore();

    await store.find(locations, useAppStore().settings.duplicateFileMinimumBytes);

    expect(find).toHaveBeenCalledWith(
      locations,
      useAppStore().settings.duplicateFileMinimumBytes,
      ['/fixture/cache'],
      []
    );
    expect(store.resultExcludedFolders).toEqual(['/fixture/cache']);
  });

  it('keeps equivalent multi-root results visible until a refresh completes', async () => {
    const currentGroup = createGroup('current', 'document.pdf');
    const replacementGroup = createGroup('replacement', 'recording.mp3');
    const currentResult = { ...createResult([currentGroup]), roots: ['E:\\Work', 'F:\\Chat'] };
    const replacementResult = { ...createResult([replacementGroup]), scanId: 8 };
    let finishScan: (result: DuplicateFilesResult) => void = () => undefined;
    vi.spyOn(DuplicateFileService, 'listenProgress').mockResolvedValue(vi.fn());
    vi.spyOn(DuplicateFileService, 'listenGroups').mockResolvedValue(vi.fn());
    vi.spyOn(DuplicateFileService, 'find').mockImplementation(
      () =>
        new Promise(resolve => {
          finishScan = resolve;
        })
    );
    const store = useDuplicateFilesStore();
    store.result = currentResult;
    store.resultComplete = true;

    const refresh = store.find(
      ['F:\\Chat', 'E:\\Work', 'E:\\Work\\nested'].map(path => ({
        path,
        mode: DUPLICATE_SCAN_LOCATION_MODES.cleanable,
      })),
      useAppStore().settings.duplicateFileMinimumBytes
    );
    await vi.waitFor(() => expect(DuplicateFileService.find).toHaveBeenCalledOnce());

    expect(store.loading).toBe(true);
    expect(store.result).toEqual(currentResult);
    expect(store.result?.scanId).toBe(7);
    expect(store.resultComplete).toBe(true);

    finishScan(replacementResult);
    await refresh;

    expect(store.result).toEqual(replacementResult);
    expect(store.resultComplete).toBe(true);
    expect(store.loading).toBe(false);
  });

  it('clears stale results when scanning a different scope', async () => {
    let finishScan: (result: DuplicateFilesResult) => void = () => undefined;
    vi.spyOn(DuplicateFileService, 'listenProgress').mockResolvedValue(vi.fn());
    vi.spyOn(DuplicateFileService, 'listenGroups').mockResolvedValue(vi.fn());
    vi.spyOn(DuplicateFileService, 'find').mockImplementation(
      () =>
        new Promise(resolve => {
          finishScan = resolve;
        })
    );
    const store = useDuplicateFilesStore();
    store.result = createResult([createGroup('current', 'document.pdf')]);
    store.resultComplete = true;

    const scan = store.find(
      [{ path: '/another-fixture', mode: DUPLICATE_SCAN_LOCATION_MODES.cleanable }],
      useAppStore().settings.duplicateFileMinimumBytes
    );
    await vi.waitFor(() => expect(DuplicateFileService.find).toHaveBeenCalledOnce());

    expect(store.result).toBeNull();
    expect(store.resultComplete).toBe(false);

    finishScan({ ...createResult([]), roots: ['/another-fixture'], scanId: 9 });
    await scan;
  });
});
