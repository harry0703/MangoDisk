import { invoke } from '@tauri-apps/api/core';

import { ANALYSIS_EXPORT_SCHEMA_VERSION } from '@/lib/models/storage/analysis-export';
import type {
  AnalysisExportDetail,
  AnalysisExportOptions,
  AnalysisExportReceipt,
  AnalysisExportScope,
  AnalysisExportScopeInfo,
} from '@/lib/models/storage/analysis-export';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function count(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

// Byte totals may exceed JavaScript's safe integer range. Rust keeps the exact report values.
function byteCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

function scopeInfo(value: unknown): value is AnalysisExportScopeInfo {
  if (
    !record(value) ||
    typeof value.path !== 'string' ||
    !value.path ||
    !count(value.scannedAtMs) ||
    !byteCount(value.totalBytes) ||
    !count(value.fileCount) ||
    !record(value.coverage)
  )
    return false;
  const coverage = value.coverage;
  if (
    coverage.schemaVersion !== 1 ||
    (coverage.elevated !== null && typeof coverage.elevated !== 'boolean') ||
    !record(coverage.omissions)
  )
    return false;
  const omissions = coverage.omissions;
  return ['permissionDenied', 'readErrors', 'excluded', 'linkOrMount', 'other'].every(key => count(omissions[key]));
}

/** The native adapter owns both the save picker and the report; JSON never crosses the WebView. */
export class AnalysisExportService {
  static async options(scanId: number): Promise<AnalysisExportOptions> {
    const started = performance.now();
    const result = await invoke<unknown>('analysis_export_options', { scanId });
    if (
      !record(result) ||
      result.schemaVersion !== ANALYSIS_EXPORT_SCHEMA_VERSION ||
      result.scanId !== scanId ||
      (result.scanMode !== 'standard' && result.scanMode !== 'fast') ||
      !scopeInfo(result.scanRoot) ||
      !scopeInfo(result.currentDirectory)
    ) {
      throw new Error('Unsupported analysis export options response');
    }
    LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportOptionsLoaded, {
      scanId,
      elapsedMs: Math.round(performance.now() - started),
    });
    return {
      schemaVersion: ANALYSIS_EXPORT_SCHEMA_VERSION,
      scanId,
      scanMode: result.scanMode,
      scanRoot: result.scanRoot,
      currentDirectory: result.currentDirectory,
    };
  }

  static async save(
    scanId: number,
    scope: AnalysisExportScope,
    includeFullPaths: boolean,
    dialogTitle: string,
    detail: AnalysisExportDetail,
    minDirectoryBytes: number
  ): Promise<AnalysisExportReceipt | null> {
    if (!count(minDirectoryBytes)) throw new Error('Invalid minimum directory bytes');
    const context = { operation: 'export_analysis_result', scanId, scope, includeFullPaths, detail, minDirectoryBytes };
    LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportRequested, context);
    try {
      const result = await invoke<unknown>('export_analysis_result', {
        scanId,
        scope,
        includeFullPaths,
        dialogTitle,
        detail,
        minDirectoryBytes,
      });
      if (result === null) {
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportCancelled, context);
        return null;
      }
      if (
        !record(result) ||
        result.schemaVersion !== ANALYSIS_EXPORT_SCHEMA_VERSION ||
        typeof result.path !== 'string' ||
        !result.path ||
        !count(result.directoryCount) ||
        result.directoryCount < 1 ||
        !count(result.fileBytes) ||
        result.fileBytes < 1
      ) {
        throw new Error('Unsupported analysis export receipt');
      }
      LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportCompleted, { ...context, ...result });
      return {
        schemaVersion: ANALYSIS_EXPORT_SCHEMA_VERSION,
        path: result.path,
        directoryCount: result.directoryCount,
        fileBytes: result.fileBytes,
      };
    } catch (error) {
      LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, { ...context, error });
      throw error;
    }
  }
}
