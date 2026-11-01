import { invoke } from '@tauri-apps/api/core';

import {
  ANALYSIS_EXPORT_DEPTHS,
  ANALYSIS_EXPORT_ENTRY_LIMITS,
  ANALYSIS_EXPORT_SCHEMA_VERSION,
  ANALYSIS_EXPORT_FORMATS,
} from '@/lib/models/storage/analysis-export';
import type { AnalysisExportReceipt, AnalysisExportRequest } from '@/lib/models/storage/analysis-export';
import { LOG_DOMAINS, LOG_EVENTS } from '@/lib/models/diagnostics/telemetry';
import { LoggerService } from '@/lib/services/diagnostics/logger-service';

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function count(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

/** The native adapter exports the retained list and owns the save picker and report bytes. */
export class AnalysisExportService {
  static async save(request: AnalysisExportRequest, dialogTitle: string): Promise<AnalysisExportReceipt | null> {
    if (
      !count(request.scanId) ||
      request.scanId === 0 ||
      !ANALYSIS_EXPORT_FORMATS.includes(request.format) ||
      !ANALYSIS_EXPORT_DEPTHS.includes(request.maxDepth) ||
      !ANALYSIS_EXPORT_ENTRY_LIMITS.includes(request.maxEntries)
    ) {
      throw new Error('Invalid analysis export request');
    }
    const context = { operation: 'export_analysis_result', ...request };
    LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportRequested, context);
    try {
      const result = await invoke<unknown>('export_analysis_result', { request, dialogTitle });
      if (result === null) {
        LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportCancelled, context);
        return null;
      }
      if (
        !record(result) ||
        result.schemaVersion !== ANALYSIS_EXPORT_SCHEMA_VERSION ||
        typeof result.path !== 'string' ||
        !result.path ||
        !count(result.entryCount) ||
        !count(result.fileBytes) ||
        result.fileBytes < 1
      ) {
        throw new Error('Unsupported analysis export receipt');
      }
      LoggerService.info(LOG_DOMAINS.analysis, LOG_EVENTS.analysisExportCompleted, { ...context, ...result });
      return {
        schemaVersion: ANALYSIS_EXPORT_SCHEMA_VERSION,
        path: result.path,
        entryCount: result.entryCount,
        fileBytes: result.fileBytes,
      };
    } catch (error) {
      LoggerService.warn(LOG_DOMAINS.analysis, LOG_EVENTS.operationFailed, { ...context, error });
      throw error;
    }
  }
}
