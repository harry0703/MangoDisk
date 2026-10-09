import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { AnalysisService } from '@/lib/services/analysis-service';

describe('AnalysisService', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it('passes the selected analysis exclusions to the native scan', async () => {
    await AnalysisService.analyze('/fixture', false, ['/fixture/cache']);

    expect(invokeMock).toHaveBeenCalledWith('analyze_path', {
      scanMode: 'standard',
      path: '/fixture',
      refresh: false,
      excludedNames: [],
      excludedPaths: ['/fixture/cache'],
    });
  });

  it('keeps an unfiltered analysis request explicit', async () => {
    await AnalysisService.analyze(undefined, true, []);

    expect(invokeMock).toHaveBeenCalledWith('analyze_path', {
      scanMode: 'standard',
      path: null,
      refresh: true,
      excludedNames: [],
      excludedPaths: [],
    });
  });

  it('sends remainder scope, snapshot identity and pagination through the versioned command', async () => {
    const page = {
      schemaVersion: 2,
      snapshotId: 101,
      parentPath: '/fixture',
      totalBytes: 64,
      totalCount: 231,
      entries: [],
      nextOffset: null,
    };
    invokeMock.mockResolvedValueOnce(page);
    await expect(
      AnalysisService.listRemainder(
        7,
        { parentPath: '/fixture', bytes: 64, visiblePaths: ['/fixture/large'] },
        200,
        101
      )
    ).resolves.toEqual(page);
    expect(invokeMock).toHaveBeenCalledWith('list_analysis_remainder', {
      request: {
        schemaVersion: 2,
        scanId: 7,
        parentPath: '/fixture',
        expectedBytes: 64,
        visiblePaths: ['/fixture/large'],
        offset: 200,
        snapshotId: 101,
      },
    });
  });

  it('rejects an incompatible remainder response', async () => {
    invokeMock.mockResolvedValueOnce({ schemaVersion: 1 });
    await expect(
      AnalysisService.listRemainder(7, { parentPath: '/fixture', bytes: 64, visiblePaths: [] })
    ).rejects.toThrow('unsupported analysis remainder schema');
  });

  it('returns authoritative deletion snapshots through the versioned response', async () => {
    const response = {
      schemaVersion: 1,
      requiresRescan: false,
      removedPath: '/fixture/deleted.bin',
      releasedBytes: 64,
      removedFileCount: 1,
      updatedResults: [
        { scanId: 7, root: '/fixture', scannedAtMs: 1, totalBytes: 0, skippedCount: 0, truncated: false, entries: [] },
      ],
      invalidatedScanIds: [8],
    };
    invokeMock.mockResolvedValueOnce(response);
    await expect(AnalysisService.deletePermanently(7, response.removedPath)).resolves.toEqual(response);
    expect(invokeMock).toHaveBeenCalledWith('delete_analysis_entry_permanently', {
      scanId: 7,
      selectedPath: response.removedPath,
    });
  });

  it('rejects an incompatible deletion response', async () => {
    invokeMock.mockResolvedValueOnce({ schemaVersion: 99 });
    await expect(AnalysisService.deletePermanently(7, '/fixture/deleted.bin')).rejects.toMatchObject({
      code: 'operationFailed',
      details: { mutationState: 'mayHaveChanged' },
    });
  });
  it.each([
    ['null response', null],
    ['missing boolean', { requiresRescan: undefined }],
    ['null updated snapshot', { updatedResults: [null] }],
    ['missing source authority', { updatedResults: [] }],
    ['conflicting authority', { invalidatedScanIds: [7] }],
    ['invalid invalidation IDs', { invalidatedScanIds: ['7'] }],
    ['negative released bytes', { releasedBytes: -1 }],
    ['invalid released bytes', { releasedBytes: Infinity }],
    ['missing path', { removedPath: undefined }],
    ['missing file count', { removedFileCount: undefined }],
    ['invalid snapshot', { updatedResults: [{ scanId: 7, root: '/fixture', entries: null }] }],
  ])('routes %s to mutation recovery', async (_name, override) => {
    const response = {
      schemaVersion: 1,
      requiresRescan: false,
      removedPath: '/fixture/deleted.bin',
      releasedBytes: 64,
      removedFileCount: 1,
      invalidatedScanIds: [],
      updatedResults: [
        { scanId: 7, root: '/fixture', scannedAtMs: 1, totalBytes: 0, skippedCount: 0, truncated: false, entries: [] },
      ],
    };
    invokeMock.mockResolvedValueOnce(override === null ? null : { ...response, ...override });
    await expect(AnalysisService.deletePermanently(7, response.removedPath)).rejects.toMatchObject({
      code: 'operationFailed',
      retryable: false,
      details: { mutationState: 'mayHaveChanged' },
    });
  });

  it('accepts an explicit recovery response without source authority', async () => {
    const response = {
      schemaVersion: 1,
      requiresRescan: true,
      removedPath: '/fixture/deleted.bin',
      releasedBytes: 64,
      removedFileCount: 1,
      updatedResults: [],
      invalidatedScanIds: [7],
    };
    invokeMock.mockResolvedValueOnce(response);
    await expect(AnalysisService.deletePermanently(7, response.removedPath)).resolves.toEqual(response);
  });
  it('validates transferred rows and nested chart data before caching them', async () => {
    const alias = {
      name: 'alias.bin',
      path: '/fixture/folder/alias.bin',
      bytes: 64,
      fileCount: 1,
      isDirectory: false,
      modifiedAtMs: null,
      contentFingerprint: null,
    };
    const node = {
      name: 'folder',
      path: '/fixture/folder',
      bytes: 64,
      fileCount: 1,
      totalEntryCount: 1,
      children: [],
      files: [alias],
    };
    const updated = {
      scanId: 7,
      root: '/fixture',
      scannedAtMs: 1,
      totalBytes: 64,
      totalEntryCount: 1,
      scanMode: 'standard',
      skippedCount: 0,
      truncated: false,
      entries: [{ ...alias, path: node.path, name: node.name, isDirectory: true }],
      directoryHierarchy: [node],
    };
    const response = {
      schemaVersion: 1,
      requiresRescan: false,
      removedPath: '/fixture/owner.bin',
      releasedBytes: 64,
      removedFileCount: 1,
      updatedResults: [updated],
      invalidatedScanIds: [],
    };
    invokeMock.mockResolvedValueOnce(response);
    await expect(AnalysisService.deletePermanently(7, response.removedPath)).resolves.toEqual(response);
    for (const invalid of [
      { ...response, updatedResults: [updated, updated] },
      { ...response, updatedResults: [{ ...updated, scanMode: 'unsupported' }] },
      { ...response, updatedResults: [{ ...updated, entries: [null] }] },
      { ...response, updatedResults: [{ ...updated, entries: [{ ...alias, bytes: -1 }] }] },
      { ...response, updatedResults: [{ ...updated, directoryHierarchy: [null] }] },
      { ...response, updatedResults: [{ ...updated, directoryHierarchy: [{ ...node, children: [null] }] }] },
      { ...response, updatedResults: [{ ...updated, directoryHierarchy: [{ ...node, files: [null] }] }] },
    ]) {
      invokeMock.mockResolvedValueOnce(invalid);
      await expect(AnalysisService.deletePermanently(7, response.removedPath)).rejects.toMatchObject({
        code: 'operationFailed',
        details: { mutationState: 'mayHaveChanged' },
      });
    }
  });
});
