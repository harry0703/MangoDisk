import { describe, expect, it } from 'vitest';

import type { CleanupApplicationCloseIdentity, PresentedScanRuleResult } from '@/lib/models/cleanup';

import { cleanupApplicationCloseGroups, cleanupApplicationCloseRetry } from './cleanup-application-close';

function rule(ruleId: string, name: string, runningProcesses: string[]): PresentedScanRuleResult {
  return {
    ruleId,
    name,
    categoryLabel: 'Application',
    description: '',
    impact: '',
    category: 'application',
    group: 'application',
    risk: 'safe',
    defaultSelected: true,
    recommendedSelected: true,
    bytes: 1,
    fileCount: 1,
    available: true,
    selectable: true,
    status: 'requiresClose',
    runningProcesses,
    requiresAppClose: true,
    sources: [],
    sourceCount: 0,
    sourcesTruncated: false,
    scanElapsedMs: 1,
  };
}

describe('cleanup application close groups', () => {
  it('combines cleanup rules that belong to the same running application', () => {
    const groups = cleanupApplicationCloseGroups([
      rule('app.wps-cache', 'WPS cache', ['wps.exe', 'wpscloudsvr.exe']),
      rule('app.wps-rendering-cache', 'WPS rendering cache', ['WPS.EXE', 'promecefpluginhost.exe']),
      rule('app.sogou-input-cache', 'Sogou Input cache', ['SGTool.exe']),
    ]);

    expect(groups).toHaveLength(2);
    expect(groups[0]).toMatchObject({
      id: 'app.wps-cache:app.wps-rendering-cache',
      ruleIds: ['app.wps-cache', 'app.wps-rendering-cache'],
      processes: ['wps.exe', 'wpscloudsvr.exe', 'promecefpluginhost.exe'],
    });
  });

  it('associates grouped processes with the exact native application icon source', () => {
    const groups = cleanupApplicationCloseGroups(
      [
        rule('app.lemon-cache', 'Tencent Lemon cache', ['Tencent Lemon', 'LemonMonitor']),
        rule('app.chatgpt-cache', 'ChatGPT cache', ['ChatGPT']),
      ],
      [
        { processName: 'LemonMonitor', iconPath: '/Applications/Tencent Lemon.app' },
        { processName: 'ChatGPT', iconPath: '/Applications/ChatGPT.app' },
      ]
    );

    expect(groups).toMatchObject([
      { iconPath: '/Applications/Tencent Lemon.app' },
      { iconPath: '/Applications/ChatGPT.app' },
    ]);
  });

  it('keeps same-named browser processes in separately authorized product groups', () => {
    const rules = [rule('origin', 'Origin cache', ['BRAVE.EXE']), rule('brave', 'Brave cache', ['brave.exe'])];
    const identities: CleanupApplicationCloseIdentity[] = [
      { ruleId: 'brave', applicationId: 'brave', applicationName: 'Brave', iconPath: 'regular.ico' },
      { ruleId: 'origin', applicationId: 'origin', applicationName: 'Brave Origin', iconPath: 'origin.ico' },
    ];
    for (const ordered of [rules, [...rules].reverse()]) {
      const groups = cleanupApplicationCloseGroups(ordered, [], identities);
      expect(groups).toHaveLength(2);
      expect(groups.find(group => group.applicationId === 'origin')).toMatchObject({
        id: 'origin',
        name: 'Brave Origin',
        iconPath: 'origin.ico',
        ruleIds: ['origin'],
      });
      expect(groups.find(group => group.applicationId === 'brave')).toMatchObject({
        id: 'brave',
        name: 'Brave',
        iconPath: 'regular.ico',
        ruleIds: ['brave'],
      });
    }
  });

  it('combines rules of the same owner even when their helper names do not overlap', () => {
    const groups = cleanupApplicationCloseGroups(
      [rule('cache', 'Cache', ['browser.exe']), rule('render', 'Rendering', ['renderer.exe'])],
      [],
      ['cache', 'render'].map(ruleId => ({
        ruleId,
        applicationId: 'browser',
        applicationName: 'Browser',
        iconPath: null,
      }))
    );
    expect(groups).toMatchObject([
      { id: 'cache:render', ruleIds: ['cache', 'render'], processes: ['browser.exe', 'renderer.exe'] },
    ]);
  });

  it('never borrows shared-process artwork or merges a scoped owner with legacy rules', () => {
    const groups = cleanupApplicationCloseGroups(
      [rule('origin', 'Origin cache', ['brave.exe']), rule('legacy', 'Legacy cache', ['brave.exe'])],
      [{ processName: 'brave.exe', iconPath: 'regular.ico' }],
      [{ ruleId: 'origin', applicationId: 'origin', applicationName: null, iconPath: null }]
    );
    expect(groups).toHaveLength(2);
    expect(groups[0]).toMatchObject({ id: 'origin', name: 'Origin cache', ruleIds: ['origin'] });
    expect(groups[0].iconPath).toBeUndefined();
    expect(groups[1].iconPath).toBe('regular.ico');
  });

  it('keeps force retries limited to the selected product despite a shared process name', () => {
    const groups = cleanupApplicationCloseGroups(
      [rule('brave', 'Brave cache', ['brave.exe']), rule('origin', 'Origin cache', ['brave.exe'])],
      [],
      ['brave', 'origin'].map(ruleId => ({ ruleId, applicationId: ruleId, applicationName: ruleId, iconPath: null }))
    );
    const retry = cleanupApplicationCloseRetry(groups, {
      mode: 'graceful',
      matchedProcessCount: 1,
      requestedProcessCount: 1,
      remainingProcessCount: 1,
      failedTargetCount: 0,
      elapsedMs: 1,
      targets: [
        {
          targetId: 'origin',
          status: 'completed',
          matchedProcessCount: 1,
          requestedProcessCount: 1,
          remainingProcesses: ['brave.exe'],
        },
      ],
    });
    expect(retry.ruleIds).toEqual(['origin']);
    expect(retry.items.map(item => item.id)).toEqual(['origin']);
  });

  it('force retries only targets that remain or failed', () => {
    const groups = cleanupApplicationCloseGroups([
      rule('app.closed-cache', 'Closed cache', ['closed-app']),
      rule('app.remaining-cache', 'Remaining cache', ['remaining-app']),
      rule('app.failed-cache', 'Failed cache', ['failed-app']),
    ]);

    const retry = cleanupApplicationCloseRetry(groups, {
      mode: 'graceful',
      matchedProcessCount: 3,
      requestedProcessCount: 2,
      remainingProcessCount: 1,
      failedTargetCount: 1,
      elapsedMs: 25,
      targets: [
        {
          targetId: 'app.closed-cache',
          status: 'completed',
          matchedProcessCount: 1,
          requestedProcessCount: 1,
          remainingProcesses: [],
        },
        {
          targetId: 'app.remaining-cache',
          status: 'completed',
          matchedProcessCount: 1,
          requestedProcessCount: 1,
          remainingProcesses: ['remaining-app'],
        },
        {
          targetId: 'app.failed-cache',
          status: 'failed',
          matchedProcessCount: 0,
          requestedProcessCount: 0,
          remainingProcesses: [],
        },
      ],
    });

    expect(retry.ruleIds).toEqual(['app.remaining-cache', 'app.failed-cache']);
    expect(retry.items.map(item => item.id)).toEqual(['app.remaining-cache', 'app.failed-cache']);
    expect(retry.items[1]?.processes).toEqual(['failed-app']);
  });
});
