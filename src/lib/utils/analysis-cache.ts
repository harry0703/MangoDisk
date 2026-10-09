import type { AnalysisDeleteResult, AnalysisResult } from '@/lib/models/analysis';
import * as PathUtils from '@/lib/utils/path';
export function key(path: string): string {
  return PathUtils.comparisonKey(path);
}
export function touch(order: readonly string[], key: string): string[] {
  return [...order.filter(item => item !== key), key];
}
/**
 * The bounded recent-results cache accelerates navigation without copying
 * the native index. Eviction only causes a later result reload.
 */
export function store(
  cache: Readonly<Record<string, AnalysisResult>>,
  order: readonly string[],
  result: AnalysisResult,
  limit: number
): {
  cache: Record<string, AnalysisResult>;
  order: string[];
} {
  const cacheKey = key(result.root);
  const nextCache = { ...cache, [cacheKey]: result };
  const nextOrder = touch(order, cacheKey);
  while (nextOrder.length > limit) {
    const removedKey = nextOrder.shift();
    if (removedKey) delete nextCache[removedKey];
  }
  return { cache: nextCache, order: nextOrder };
}
export function retainExisting(order: readonly string[], cache: Readonly<Record<string, AnalysisResult>>): string[] {
  return order.filter(key => Boolean(cache[key]));
}
/** Applies Core-owned snapshots, including hard-link allocation transfers. */
export function syncAfterDelete(
  cache: Readonly<Record<string, AnalysisResult>>,
  order: readonly string[],
  removed: AnalysisDeleteResult,
  limit: number
): { cache: Record<string, AnalysisResult>; order: string[] } {
  const invalidated = new Set(removed.invalidatedScanIds);
  const retained = Object.fromEntries(Object.entries(cache).filter(([, result]) => !invalidated.has(result.scanId)));
  let synchronized = { cache: retained, order: retainExisting(order, retained) };
  // Native sessions can outlive UI eviction. Restored siblings must reenter the
  // same bounded cache used by navigation rather than creating untracked roots.
  for (const result of removed.updatedResults) {
    synchronized = store(synchronized.cache, synchronized.order, result, limit);
  }
  return synchronized;
}

/** Drops ancestors and descendants whose snapshot may have changed. */
export function invalidateChangedPath(
  cache: Readonly<Record<string, AnalysisResult>>,
  changedPath: string
): Record<string, AnalysisResult> {
  const changedKey = key(changedPath);
  return Object.fromEntries(
    Object.entries(cache).filter(
      ([root]) => !PathUtils.isSameOrChildKey(root, changedKey) && !PathUtils.isSameOrChildKey(changedKey, root)
    )
  );
}
