import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));

import { FolderSelectionService } from '@/lib/services/storage/folder-selection-service';
import { MAX_DIRECTORY_ENTRIES_PER_REQUEST } from '@/lib/models/storage/folder-selection';
import { parseCommandErrorReason } from '@/lib/utils/diagnostics/error';

describe('directory entry resolution', () => {
  beforeEach(() => vi.clearAllMocks());

  it('keeps mappings but deduplicates targets for an explicit selection', async () => {
    invokeMock.mockResolvedValue({
      schemaVersion: 1,
      directories: [
        { requestedPath: 'C:\\Alias', path: '\\\\server\\share\\Folder' },
        { requestedPath: '\\\\SERVER\\share\\folder', path: '\\\\server\\share\\Folder' },
      ],
      rejectedCount: 0,
      redirectedCount: 1,
    });
    expect(await FolderSelectionService.filterExistingDirectories(['C:\\Alias', '\\\\SERVER\\share\\folder'])).toEqual([
      '\\\\server\\share\\Folder',
    ]);
  });

  it('reports rejected selections without returning a partial silent success', async () => {
    invokeMock.mockResolvedValue({ schemaVersion: 1, directories: [], rejectedCount: 1, redirectedCount: 0 });
    await expect(FolderSelectionService.filterExistingDirectories(['C:\\Missing'])).rejects.toMatchObject({
      code: 'invalidInput',
      details: { reason: 'folderUnavailable' },
    });
    expect(
      parseCommandErrorReason({ code: 'invalidInput', details: { reason: 'folderUnavailable' }, retryable: true })
    ).toBe('folderUnavailable');
  });

  it('does not report cancellation as a failure', async () => {
    expect(await FolderSelectionService.filterExistingDirectories([])).toEqual([]);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('validates a large explicit scope in bounded batches', async () => {
    const paths = Array.from({ length: 70 }, (_, index) => `/folder-${index}`);
    invokeMock.mockImplementation(async (_command: string, args: { paths: string[] }) => ({
      schemaVersion: 1,
      directories: args.paths.map(path => ({ requestedPath: path, path })),
    }));
    expect(await FolderSelectionService.filterExistingDirectories(paths)).toEqual(paths);
    expect(invokeMock.mock.calls.map(([, args]) => args.paths.length)).toEqual([64, 6]);
  });

  it('does not access later batches after a scan request is cancelled', async () => {
    let cancelled = false;
    invokeMock.mockImplementationOnce(async (_command: string, args: { paths: string[] }) => {
      cancelled = true;
      return { schemaVersion: 1, directories: args.paths.map(path => ({ requestedPath: path, path })) };
    });
    const paths = Array.from({ length: 70 }, (_, index) => `/folder-${index}`);
    expect(await FolderSelectionService.filterExistingDirectories(paths, () => cancelled)).toEqual([]);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('rejects incompatible protocol versions', async () => {
    invokeMock.mockResolvedValue({ schemaVersion: 2, directories: [] });
    await expect(FolderSelectionService.resolveDirectories(['C:\\Folder'])).rejects.toThrow(
      'Unsupported directory resolution response'
    );
  });

  it('rejects oversized selections before invoking native path resolution', async () => {
    const paths = Array.from({ length: MAX_DIRECTORY_ENTRIES_PER_REQUEST + 1 }, (_, index) => `/folder-${index}`);
    const error = {
      code: 'invalidInput',
      details: { reason: 'folderSelectionLimitExceeded' },
      retryable: false,
    } as const;
    await expect(FolderSelectionService.resolveDirectories(paths)).rejects.toMatchObject(error);
    expect(parseCommandErrorReason(error)).toBe('folderSelectionLimitExceeded');
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
