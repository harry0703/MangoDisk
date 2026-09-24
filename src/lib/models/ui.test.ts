import { describe, expect, it } from 'vitest';

import { ICON_NAMES } from './ui';

describe('UI icon names', () => {
  it('provides the sparkles icon used by system optimization actions', () => {
    expect(ICON_NAMES.sparkles).toBe('sparkles');
  });

  it('provides the copy icon used by inline actions', () => {
    expect(ICON_NAMES.copy).toBe('copy');
  });

  it('provides the clock icon used by system-owned background work', () => {
    expect(ICON_NAMES.clock).toBe('clock');
  });

  it('provides the Linux folder fallback used when no native icon provider is available', () => {
    expect(ICON_NAMES.linuxFolder).toBe('linuxFolder');
  });
});
