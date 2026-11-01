import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AnalysisExportService } from './analysis-export-service';
import type { AnalysisExportRequest } from '@/lib/models/storage/analysis-export';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/services/diagnostics/logger-service', () => ({ LoggerService: { info: vi.fn(), warn: vi.fn() } }));
const request: AnalysisExportRequest = {
  scanId: 7,
  maxDepth: 2,
  maxEntries: 500,
  includeFullPaths: false,
  format: 'json',
};

describe('nested analysis export transport', () => {
  beforeEach(() => invokeMock.mockReset());
  it('sends only selection parameters, keeping the report and destination native', async () => {
    const receipt = { schemaVersion: 2, path: '/exports/report.json', entryCount: 12, fileBytes: 2048 };
    invokeMock.mockResolvedValueOnce(receipt);
    await expect(AnalysisExportService.save(request, 'Save results')).resolves.toEqual(receipt);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('export_analysis_result', {
      request,
      dialogTitle: 'Save results',
    });
  });
  it('allows an empty report and native picker cancellation', async () => {
    invokeMock.mockResolvedValueOnce({ schemaVersion: 2, path: '/empty.json', entryCount: 0, fileBytes: 100 });
    await expect(AnalysisExportService.save(request, 'Save results')).resolves.toMatchObject({ entryCount: 0 });
    invokeMock.mockResolvedValueOnce(null);
    await expect(
      AnalysisExportService.save({ ...request, maxDepth: 6, maxEntries: 0 }, 'Save results')
    ).resolves.toBeNull();
  });
  it('rejects invalid depth, budget or session before invoking native code', async () => {
    for (const invalid of [
      { ...request, maxDepth: 7 },
      { ...request, maxEntries: 123 },
      { ...request, scanId: 0 },
      { ...request, format: 'csv' },
    ]) {
      await expect(AnalysisExportService.save(invalid as AnalysisExportRequest, 'Save results')).rejects.toThrow(
        'request'
      );
    }
    expect(invokeMock).not.toHaveBeenCalled();
  });
  it('passes Markdown selection through without changing report or picker ownership', async () => {
    invokeMock.mockResolvedValueOnce({ schemaVersion: 2, path: '/out.md', entryCount: 12, fileBytes: 1024 });
    await AnalysisExportService.save({ ...request, format: 'markdown' }, 'Save results');
    expect(invokeMock).toHaveBeenCalledWith('export_analysis_result', {
      request: { ...request, format: 'markdown' },
      dialogTitle: 'Save results',
    });
  });
  it('preserves typed snapshot expiration errors', async () => {
    const error = { code: 'invalidInput', details: { reason: 'analysisExportUnavailable' }, retryable: false };
    invokeMock.mockRejectedValueOnce(error);
    await expect(AnalysisExportService.save(request, 'Save results')).rejects.toEqual(error);
  });
  it('rejects incompatible or malformed receipts', async () => {
    const valid = { schemaVersion: 2, path: '/out.json', entryCount: 0, fileBytes: 100 };
    for (const response of [
      { ...valid, schemaVersion: 1 },
      { ...valid, path: '' },
      { ...valid, entryCount: -1 },
      { ...valid, fileBytes: 0 },
    ]) {
      invokeMock.mockResolvedValueOnce(response);
      await expect(AnalysisExportService.save(request, 'Save results')).rejects.toThrow('receipt');
    }
  });
});
