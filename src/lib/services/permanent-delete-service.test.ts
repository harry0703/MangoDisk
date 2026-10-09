import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: invokeMock,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(),
}));

import { DuplicateFileService } from '@/lib/services/duplicate-file-service';
import { AnalysisService } from '@/lib/services/analysis-service';
import { PermanentDeleteService } from '@/lib/services/permanent-delete-service';

const candidate = {
  path: '/fixture/example.bin',
  expectedBytes: 128,
  expectedModifiedAtMs: 42,
};

describe('permanent delete services', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({
      removedPaths: [candidate.path],
      failed: [],
      releasedBytes: candidate.expectedBytes,
    });
  });

  it('binds large-file paths to the active native scan session', async () => {
    await PermanentDeleteService.deleteFiles(9, [candidate.path]);

    expect(invokeMock).toHaveBeenCalledWith('delete_files_permanently', {
      scanId: 9,
      selectedPaths: [candidate.path],
    });
  });

  it('binds duplicate deletion to the active native scan session', async () => {
    await DuplicateFileService.deletePermanently(17, [candidate]);

    expect(invokeMock).toHaveBeenCalledWith('delete_duplicate_files_permanently', {
      scanId: 17,
      candidates: [candidate],
    });
  });

  it('binds disk-analysis deletion to the active native scan session', async () => {
    invokeMock.mockResolvedValueOnce({
      schemaVersion: 1,
      requiresRescan: false,
      removedPath: candidate.path,
      releasedBytes: 64,
      removedFileCount: 1,
      updatedResults: [
        { scanId: 23, root: '/fixture', scannedAtMs: 1, totalBytes: 0, skippedCount: 0, truncated: false, entries: [] },
      ],
      invalidatedScanIds: [],
    });
    await AnalysisService.deletePermanently(23, candidate.path);

    expect(invokeMock).toHaveBeenCalledWith('delete_analysis_entry_permanently', {
      scanId: 23,
      selectedPath: candidate.path,
    });
  });
});
