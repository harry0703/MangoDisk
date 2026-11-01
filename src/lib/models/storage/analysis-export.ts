export const ANALYSIS_EXPORT_SCHEMA_VERSION = 2;
export const ANALYSIS_EXPORT_DEPTHS = [1, 2, 3, 4, 5, 6] as const;
export const ANALYSIS_EXPORT_ENTRY_LIMITS = [200, 500, 1000, 2000, 0] as const;
export type AnalysisExportDepth = (typeof ANALYSIS_EXPORT_DEPTHS)[number];
export type AnalysisExportEntryLimit = (typeof ANALYSIS_EXPORT_ENTRY_LIMITS)[number];
export const ANALYSIS_EXPORT_FORMATS = ['json', 'markdown'] as const;
export type AnalysisExportFormat = (typeof ANALYSIS_EXPORT_FORMATS)[number];

export const ANALYSIS_EXPORT_PREFERENCES_SCHEMA_VERSION = 1;

export interface AnalysisExportPreferences {
  schemaVersion: typeof ANALYSIS_EXPORT_PREFERENCES_SCHEMA_VERSION;
  maxDepth: AnalysisExportDepth;
  maxEntries: AnalysisExportEntryLimit;
  includeFullPaths: boolean;
  format: AnalysisExportFormat;
}

export interface AnalysisExportRequest {
  scanId: number;
  maxDepth: AnalysisExportDepth;
  /** Zero means all available cached detail within the selected depth. */
  maxEntries: AnalysisExportEntryLimit;
  includeFullPaths: boolean;
  format: AnalysisExportFormat;
}

export interface AnalysisExportReceipt {
  schemaVersion: typeof ANALYSIS_EXPORT_SCHEMA_VERSION;
  path: string;
  entryCount: number;
  fileBytes: number;
}
