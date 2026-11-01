import {
  ANALYSIS_EXPORT_DEPTHS,
  ANALYSIS_EXPORT_ENTRY_LIMITS,
  ANALYSIS_EXPORT_PREFERENCES_SCHEMA_VERSION,
  ANALYSIS_EXPORT_FORMATS,
  type AnalysisExportDepth,
  type AnalysisExportEntryLimit,
  type AnalysisExportPreferences,
  type AnalysisExportFormat,
} from '@/lib/models/storage/analysis-export';

export function defaults(): AnalysisExportPreferences {
  return {
    schemaVersion: ANALYSIS_EXPORT_PREFERENCES_SCHEMA_VERSION,
    maxDepth: 2,
    maxEntries: 500,
    includeFullPaths: false,
    format: 'json',
  };
}

export function isDepth(value: unknown): value is AnalysisExportDepth {
  return ANALYSIS_EXPORT_DEPTHS.some(depth => depth === value);
}

export function isEntryLimit(value: unknown): value is AnalysisExportEntryLimit {
  return ANALYSIS_EXPORT_ENTRY_LIMITS.some(limit => limit === value);
}

export function isFormat(value: unknown): value is AnalysisExportFormat {
  return ANALYSIS_EXPORT_FORMATS.some(format => format === value);
}

export function parse(value: unknown): AnalysisExportPreferences {
  const result = defaults();
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return result;
  const record = value as Record<string, unknown>;
  if (record.schemaVersion !== ANALYSIS_EXPORT_PREFERENCES_SCHEMA_VERSION) return result;
  if (isDepth(record.maxDepth)) result.maxDepth = record.maxDepth;
  if (isEntryLimit(record.maxEntries)) result.maxEntries = record.maxEntries;
  if (typeof record.includeFullPaths === 'boolean') result.includeFullPaths = record.includeFullPaths;
  if (isFormat(record.format)) result.format = record.format;
  return result;
}
