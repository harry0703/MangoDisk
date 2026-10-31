import type { ByteSizePreset } from '@/lib/models/settings/byte-size';
import type { AnalysisResult, AnalysisScanMode } from '@/lib/models/storage/analysis';

export const ANALYSIS_EXPORT_SCHEMA_VERSION = 1;
export type AnalysisExportScope = 'scanRoot' | 'currentDirectory';
export type AnalysisExportDetail = 'summary' | 'full';

export const ANALYSIS_EXPORT_MINIMUM_SIZE_PRESETS: readonly ByteSizePreset[] = [
  { amount: 10, unit: 'MB' },
  { amount: 100, unit: 'MB' },
  { amount: 1, unit: 'GB' },
];

export interface AnalysisExportScopeInfo {
  path: string;
  scannedAtMs: number;
  totalBytes: number;
  fileCount: number;
  coverage: NonNullable<AnalysisResult['coverage']>;
}

export interface AnalysisExportOptions {
  schemaVersion: typeof ANALYSIS_EXPORT_SCHEMA_VERSION;
  scanId: number;
  scanMode: AnalysisScanMode;
  scanRoot: AnalysisExportScopeInfo;
  currentDirectory: AnalysisExportScopeInfo;
}

export interface AnalysisExportReceipt {
  schemaVersion: typeof ANALYSIS_EXPORT_SCHEMA_VERSION;
  path: string;
  directoryCount: number;
  fileBytes: number;
}
