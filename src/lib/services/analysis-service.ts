import type { ScanNameExclusion } from '@/lib/models/storage-scan';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import { EVENT_NAMES } from '@/lib/models/telemetry';
import {
  ANALYSIS_DELETE_SCHEMA_VERSION,
  ANALYSIS_REMAINDER_SCHEMA_VERSION,
  ANALYSIS_SCAN_MODES,
} from '@/lib/models/analysis';
import type {
  AnalysisDeleteResult,
  AnalysisRemainderPage,
  AnalysisRemainderSelection,
  AnalysisResult,
  AnalysisScanMode,
} from '@/lib/models/analysis';
import type { TraversalProgress } from '@/lib/models/progress';
import type { CommandError } from '@/lib/utils/error';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && Number.isInteger(value) && value >= 0;
}

function isEntry(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === 'string' &&
    typeof value.path === 'string' &&
    isCount(value.bytes) &&
    isCount(value.fileCount) &&
    typeof value.isDirectory === 'boolean' &&
    (value.modifiedAtMs === null || isCount(value.modifiedAtMs)) &&
    (value.contentFingerprint === null || typeof value.contentFingerprint === 'string')
  );
}

function isHierarchyNode(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === 'string' &&
    typeof value.path === 'string' &&
    isCount(value.bytes) &&
    isCount(value.fileCount) &&
    (value.totalEntryCount === undefined || isCount(value.totalEntryCount)) &&
    Array.isArray(value.children) &&
    value.children.every(isHierarchyNode) &&
    (value.files === undefined || (Array.isArray(value.files) && value.files.every(isEntry)))
  );
}

function isAnalysisResult(value: unknown): value is AnalysisResult {
  return (
    isRecord(value) &&
    isCount(value.scanId) &&
    value.scanId > 0 &&
    typeof value.root === 'string' &&
    isCount(value.scannedAtMs) &&
    isCount(value.totalBytes) &&
    isCount(value.skippedCount) &&
    typeof value.truncated === 'boolean' &&
    Array.isArray(value.entries) &&
    value.entries.every(isEntry) &&
    (value.scanMode === undefined || value.scanMode === 'standard' || value.scanMode === 'fast') &&
    (value.totalEntryCount === undefined || isCount(value.totalEntryCount)) &&
    (value.directoryHierarchy === undefined ||
      (Array.isArray(value.directoryHierarchy) && value.directoryHierarchy.every(isHierarchyNode)))
  );
}

function isDeletionResult(value: unknown, scanId: number): value is AnalysisDeleteResult {
  if (
    !isRecord(value) ||
    value.schemaVersion !== ANALYSIS_DELETE_SCHEMA_VERSION ||
    typeof value.requiresRescan !== 'boolean' ||
    typeof value.removedPath !== 'string' ||
    !isCount(value.releasedBytes) ||
    !isCount(value.removedFileCount) ||
    !Array.isArray(value.updatedResults) ||
    !value.updatedResults.every(isAnalysisResult) ||
    !Array.isArray(value.invalidatedScanIds) ||
    !value.invalidatedScanIds.every(isCount)
  )
    return false;
  const updatedIds = value.updatedResults.map(result => result.scanId);
  return (
    new Set(updatedIds).size === updatedIds.length &&
    !value.invalidatedScanIds.some(id => updatedIds.includes(id)) &&
    (value.requiresRescan || updatedIds.includes(scanId))
  );
}

export class AnalysisService {
  static async listRemainder(
    scanId: number,
    selection: AnalysisRemainderSelection,
    offset = 0,
    snapshotId: number | null = null
  ): Promise<AnalysisRemainderPage> {
    const page = await invoke<AnalysisRemainderPage>('list_analysis_remainder', {
      request: {
        schemaVersion: ANALYSIS_REMAINDER_SCHEMA_VERSION,
        scanId,
        parentPath: selection.parentPath,
        visiblePaths: selection.visiblePaths,
        expectedBytes: selection.bytes,
        offset,
        snapshotId,
      },
    });
    if (page.schemaVersion !== ANALYSIS_REMAINDER_SCHEMA_VERSION) {
      throw new Error('unsupported analysis remainder schema');
    }
    return page;
  }

  static releaseRemainder(snapshotId: number): Promise<void> {
    return invoke('release_analysis_remainder', { snapshotId });
  }

  static analyze(
    path: string | undefined,
    refresh: boolean,
    excludedFolders: string[],
    excludedNames: ScanNameExclusion[] = [],
    scanMode: AnalysisScanMode = ANALYSIS_SCAN_MODES.standard
  ): Promise<AnalysisResult> {
    return invoke<AnalysisResult>('analyze_path', {
      path: path?.trim() || null,
      refresh,
      scanMode,
      excludedPaths: excludedFolders,
      excludedNames,
    });
  }

  static listenProgress(handler: (progress: TraversalProgress) => void): Promise<UnlistenFn> {
    return listen<TraversalProgress>(EVENT_NAMES.analysisProgress, event => handler(event.payload));
  }

  static cancel(): Promise<void> {
    return invoke<void>('cancel_analysis');
  }

  static async deletePermanently(scanId: number, selectedPath: string): Promise<AnalysisDeleteResult> {
    const result = await invoke<unknown>('delete_analysis_entry_permanently', { scanId, selectedPath });
    if (!isDeletionResult(result, scanId)) {
      // The native mutation already ran; an incompatible response must not
      // leave the deleted entry in a snapshot treated as authoritative.
      throw Object.assign(new Error('unsupported analysis deletion response schema'), {
        code: 'operationFailed',
        retryable: false,
        details: { operation: 'delete_analysis_entry_permanently', mutationState: 'mayHaveChanged' },
      } satisfies CommandError);
    }
    return result;
  }
}
