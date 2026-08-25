import { BYTE_SIZE_UNITS, type ByteSizePreset } from '@/lib/models/byte-size';

export const LARGE_FILE_SORT_KEYS = {
  name: 'name',
  bytes: 'bytes',
  modified: 'modified',
} as const;

export const LARGE_FILE_MINIMUM_PRESETS = [
  { amount: 50, unit: BYTE_SIZE_UNITS.megabytes },
  { amount: 100, unit: BYTE_SIZE_UNITS.megabytes },
  { amount: 500, unit: BYTE_SIZE_UNITS.megabytes },
  { amount: 1, unit: BYTE_SIZE_UNITS.gigabytes },
  { amount: 5, unit: BYTE_SIZE_UNITS.gigabytes },
] as const satisfies readonly ByteSizePreset[];

export const DEFAULT_LARGE_FILE_MINIMUM_PRESET = LARGE_FILE_MINIMUM_PRESETS[1];
export const LARGE_FILE_RENDER_BATCH_SIZE = 80;

export interface LargeFileEntry {
  name: string;
  path: string;
  parentPath: string;
  bytes: number;
  modifiedAtMs: number | null;
}

export interface LargeFilesResult {
  scanId: number;
  root: string;
  scannedAtMs: number;
  minimumBytes: number;
  totalBytes: number;
  totalCount: number;
  returnedCount: number;
  truncated: boolean;
  skippedCount: number;
  cacheReused: boolean;
  entries: LargeFileEntry[];
}

export type LargeFilesSelectionMode = 'smart' | 'all' | 'none' | 'manual';
