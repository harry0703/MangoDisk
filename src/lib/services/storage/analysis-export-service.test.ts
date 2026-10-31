import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AnalysisExportService } from './analysis-export-service';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { info: vi.fn(), warn: vi.fn() } }));

describe('analysis export transport', () => {
  beforeEach(() => invokeMock.mockReset());

  it('rejects incompatible or mismatched option responses', async () => {
    for (const response of [
      { schemaVersion: 2, scanId: 7 },
      { schemaVersion: 1, scanId: 8 },
      { schemaVersion: 1, scanId: 7, scanMode: 'standard', scanRoot: null, currentDirectory: null },
    ]) {
      invokeMock.mockResolvedValueOnce(response);
      await expect(AnalysisExportService.options(7)).rejects.toThrow('Unsupported');
    }
  });

  it('does not reject native scan byte totals beyond JavaScript safe integer precision', async () => {
    const coverage = {
      schemaVersion: 1,
      elevated: false,
      omissions: { permissionDenied: 0, readErrors: 0, excluded: 0, linkOrMount: 0, other: 0 },
    };
    const scope = { path: '/archive', scannedAtMs: 1, totalBytes: 2 ** 53, fileCount: 2, coverage };
    const response = { schemaVersion: 1, scanId: 7, scanMode: 'standard', scanRoot: scope, currentDirectory: scope };
    invokeMock.mockResolvedValueOnce(response);
    await expect(AnalysisExportService.options(7)).resolves.toEqual(response);
    for (const totalBytes of [-1, 0.5, NaN, Infinity, '9007199254740992']) {
      invokeMock.mockResolvedValueOnce({ ...response, scanRoot: { ...scope, totalBytes } });
      await expect(AnalysisExportService.options(7)).rejects.toThrow('Unsupported');
    }
  });

  it('keeps the save destination and report JSON inside the native adapter', async () => {
    const receipt = { schemaVersion: 1, path: '/exports/report.json', directoryCount: 12, fileBytes: 2048 };
    invokeMock.mockResolvedValueOnce(receipt);
    await expect(
      AnalysisExportService.save(7, 'currentDirectory', false, 'Save results', 'summary', 0)
    ).resolves.toEqual(receipt);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('export_analysis_result', {
      scanId: 7,
      scope: 'currentDirectory',
      includeFullPaths: false,
      dialogTitle: 'Save results',
      detail: 'summary',
      minDirectoryBytes: 0,
    });
  });

  it('treats native picker dismissal as cancellation', async () => {
    invokeMock.mockResolvedValueOnce(null);
    await expect(AnalysisExportService.save(7, 'scanRoot', true, 'Save results', 'full', 0)).resolves.toBeNull();
  });

  it('preserves typed expiration errors for the rescan interaction', async () => {
    const error = { code: 'invalidInput', details: { reason: 'analysisExportUnavailable' }, retryable: false };
    invokeMock.mockRejectedValueOnce(error);
    await expect(AnalysisExportService.save(7, 'scanRoot', false, 'Save results', 'summary', 0)).rejects.toEqual(error);
  });

  it('rejects malformed receipts instead of reporting successful export', async () => {
    invokeMock.mockResolvedValueOnce({ schemaVersion: 1, path: '', directoryCount: 0, fileBytes: -1 });
    await expect(AnalysisExportService.save(7, 'scanRoot', false, 'Save results', 'summary', 0)).rejects.toThrow(
      'receipt'
    );
  });
  it.each(['summary', 'full'] as const)('sends the byte threshold for %s exports', async detail => {
    invokeMock.mockResolvedValueOnce(null);
    await AnalysisExportService.save(7, 'scanRoot', false, 'Save results', detail, 100000000);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith(
      'export_analysis_result',
      expect.objectContaining({ detail, minDirectoryBytes: 100000000 })
    );
  });

  it('rejects invalid byte thresholds before invoking native export', async () => {
    for (const value of [-1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
      await expect(AnalysisExportService.save(7, 'scanRoot', false, 'Save results', 'full', value)).rejects.toThrow(
        'minimum directory bytes'
      );
    }
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
