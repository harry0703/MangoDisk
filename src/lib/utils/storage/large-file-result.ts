import type { LargeFilesResult } from '@/lib/models/storage/large-file';
export function removePaths(result: LargeFilesResult, removedPaths: ReadonlySet<string>): LargeFilesResult {
  // This result remains a scan snapshot. Live reclaimed storage can differ
  // after an edit and must not change the sizes of unrelated retained rows.
  const removedBytes = result.entries.reduce(
    (bytes, entry) => bytes + (removedPaths.has(entry.path) ? entry.bytes : 0),
    0
  );
  const entries = result.entries.filter(entry => !removedPaths.has(entry.path));
  const totalCount = Math.max(0, result.totalCount - removedPaths.size);
  return {
    ...result,
    totalBytes: Math.max(0, result.totalBytes - removedBytes),
    totalCount,
    returnedCount: entries.length,
    truncated: entries.length < totalCount,
    entries,
  };
}
