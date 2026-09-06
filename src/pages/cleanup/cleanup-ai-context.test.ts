import { describe, expect, it } from 'vitest';
import type { PresentedScanRuleResult } from '@/lib/models/cleanup';
import { cleanupAiContext } from './cleanup-ai-context';

describe('cleanup explanation metadata', () => {
  it('preserves full paths and scan constraints without inspecting file contents', () => {
    const rule = {
      category: 'browser',
      name: 'Browser cache',
      description: 'Cache',
      impact: 'Rebuilt',
      bytes: 10,
      fileCount: 2,
      requiresAppClose: true,
      risk: 'safe',
      status: 'requiresClose',
      available: true,
      selectable: true,
      sourcesTruncated: false,
      sourceCount: 1,
      sources: [
        { path: '/private/person/file', bytes: 10, fileCount: 2, modifiedAtMs: null, blockReason: 'requiresClose' },
      ],
      runningProcesses: ['browser-process'],
      ruleId: 'browser.cache',
    } as PresentedScanRuleResult;
    const context = cleanupAiContext(rule)!;
    expect(context.subject).toMatchObject({
      module: 'cleanup',
      impact: 'Rebuilt',
      bytes: 10,
      itemCount: 2,
      requiresAppClose: true,
      scan: {
        status: 'requiresClose',
        risk: 'safe',
        selectable: true,
        sources: rule.sources,
        runningProcesses: rule.runningProcesses,
      },
    });
    expect(JSON.stringify(context)).toContain('/private/person/file');
    expect(cleanupAiContext({ ...rule, category: 'custom' })).toBeNull();
  });
});
