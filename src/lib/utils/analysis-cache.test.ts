import { describe, expect, it } from 'vitest';
import type { AnalysisDeleteResult, AnalysisResult } from '@/lib/models/analysis';
import { key, syncAfterDelete } from './analysis-cache';

function snapshot(root: string, scanId: number, totalBytes = 64): AnalysisResult {
  return { scanId, root, scannedAtMs: 1, totalBytes, skippedCount: 0, truncated: false, entries: [] };
}

describe('analysis cache deletion synchronization', () => {
  it.each([
    ['/fixture', '/fixture/child', '/sibling'],
    ['C:\\Fixture', 'c:/fixture/Child', 'C:\\Sibling'],
    ['\\\\server\\share\\Fixture', '\\\\server\\share\\Fixture\\Child', '\\\\server\\share\\Sibling'],
  ])('uses scan authority for %s without expiring a transferred sibling', (root, child, sibling) => {
    const source = snapshot(root, 7);
    const outdatedChild = snapshot(child, 8);
    const sharedSibling = snapshot(sibling, 9, 0);
    const unrelated = snapshot('/unrelated', 10);
    const cache = Object.fromEntries(
      [source, outdatedChild, sharedSibling, unrelated].map(row => [key(row.root), row])
    );
    const updatedSource = { ...source, totalBytes: 0 };
    const updatedSibling = { ...sharedSibling, totalBytes: 64 };
    const response: AnalysisDeleteResult = {
      schemaVersion: 1,
      requiresRescan: false,
      removedPath: `${root}/deleted.bin`,
      releasedBytes: 64,
      removedFileCount: 1,
      updatedResults: [updatedSource, updatedSibling],
      invalidatedScanIds: [8],
    };
    const synchronized = syncAfterDelete(cache, Object.keys(cache), response, 80).cache;
    expect(synchronized[key(root)]).toBe(updatedSource);
    expect(synchronized[key(child)]).toBeUndefined();
    expect(synchronized[key(sibling)]).toBe(updatedSibling);
    expect(synchronized['/unrelated']).toBe(unrelated);
    expect(cache[key(root)]).toBe(source);
    expect(cache[key(child)]).toBe(outdatedChild);
    expect(sharedSibling.totalBytes).toBe(0);
  });

  it('replaces a Windows root under the same normalized key', () => {
    const previous = snapshot('C:\\Fixture', 7);
    const updated = snapshot('c:/FIXTURE/', 9, 20);
    const synchronized = syncAfterDelete(
      { [key(previous.root)]: previous },
      [key(previous.root)],
      {
        schemaVersion: 1,
        requiresRescan: false,
        removedPath: 'C:\\Fixture\\item',
        releasedBytes: 44,
        removedFileCount: 1,
        updatedResults: [updated],
        invalidatedScanIds: [7],
      },
      80
    ).cache;
    expect(Object.keys(synchronized)).toEqual([key(previous.root)]);
    expect(synchronized[key(previous.root)]).toBe(updated);
  });
});
