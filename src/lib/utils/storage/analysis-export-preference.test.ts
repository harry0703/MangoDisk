import { describe, expect, it } from 'vitest';

import { defaults, parse } from './analysis-export-preference';

describe('analysis export preference validation', () => {
  it('restores supported options including unlimited details and full paths', () => {
    expect(parse({ schemaVersion: 1, maxDepth: 6, maxEntries: 0, includeFullPaths: true })).toEqual({
      schemaVersion: 1,
      maxDepth: 6,
      maxEntries: 0,
      includeFullPaths: true,
      format: 'json',
    });
  });

  it('restores Markdown while retaining older preferences that have no format', () => {
    const saved = { schemaVersion: 1, maxDepth: 6, maxEntries: 1000, includeFullPaths: true };
    expect(parse(saved)).toEqual({ ...saved, format: 'json' });
    expect(parse({ ...saved, format: 'markdown' })).toEqual({ ...saved, format: 'markdown' });
    expect(parse({ ...saved, format: 'csv' })).toEqual({ ...saved, format: 'json' });
  });

  it('rejects unknown documents and repairs invalid fields independently', () => {
    for (const value of [null, [], 'export', {}, { schemaVersion: 2, maxDepth: 6 }]) {
      expect(parse(value)).toEqual(defaults());
    }
    expect(parse({ schemaVersion: 1, maxDepth: 7, maxEntries: 1000, includeFullPaths: 'true' })).toEqual({
      ...defaults(),
      maxEntries: 1000,
    });
    for (const value of [NaN, Infinity, -1, 1.5, '500', null]) {
      expect(parse({ schemaVersion: 1, maxDepth: value, maxEntries: value })).toEqual(defaults());
    }
  });
});
