import { describe, expect, it } from 'vitest';

import type { PrivacyItem } from '@/lib/models/privacy';

import { buildPrivacyResultCategories, buildPrivacyResultSourceGroups } from './privacy-result-categories';

function item(
  token: string,
  category: PrivacyItem['category'],
  itemCount: number,
  capability: PrivacyItem['capability'] = 'ready'
): PrivacyItem {
  return {
    token,
    sourceId: 'fixture-browser',
    sourceName: 'Fixture Browser',
    profileId: 'fixture-browser:Default',
    profileName: 'Default',
    category,
    kind:
      category === 'browserActivity'
        ? 'browsingHistory'
        : category === 'browserAccountState'
          ? 'cookies'
          : category === 'applicationActivity'
            ? 'recentDocuments'
            : 'recentItems',
    sensitivity: 'activity',
    impact: 'low',
    recommendation: 'recommended',
    capability,
    itemCount,
    estimatedBytes: itemCount * 16,
    selectedByDefault: true,
    requiresBrowserClose: category !== 'systemActivity',
    synchronizationMayPropagate: false,
  };
}

describe('privacy result categories', () => {
  it('keeps browser activity and account state as independent decision categories', () => {
    const categories = buildPrivacyResultCategories(
      [
        item('system', 'systemActivity', 2),
        item('activity', 'browserActivity', 5),
        item('account', 'browserAccountState', 3),
      ],
      []
    );

    expect(categories.map(category => category.id)).toEqual([
      'browserActivity',
      'browserAccountState',
      'systemActivity',
    ]);
    expect(categories[0]).toMatchObject({ itemCount: 1, traceCount: 5 });
    expect(categories[1]).toMatchObject({ itemCount: 1, traceCount: 3 });
  });

  it('keeps application activity as an independent user-visible category', () => {
    const categories = buildPrivacyResultCategories(
      [
        item('system', 'systemActivity', 2),
        item('application', 'applicationActivity', 4),
        item('browser', 'browserActivity', 5),
      ],
      []
    );

    expect(categories.map(category => category.id)).toEqual([
      'browserActivity',
      'applicationActivity',
      'systemActivity',
    ]);
    expect(categories[1]).toMatchObject({ itemCount: 1, traceCount: 4 });
  });

  it('aggregates trace counts and exposes a partial category selection', () => {
    const categories = buildPrivacyResultCategories(
      [
        item('history', 'browserActivity', 5),
        item('downloads', 'browserActivity', 3),
        item('blocked', 'browserActivity', 7, 'schemaUnsupported'),
      ],
      ['history', 'blocked']
    );

    expect(categories[0]).toMatchObject({
      itemCount: 3,
      traceCount: 15,
      selectedTraceCount: 5,
      selection: 'partial',
    });
  });

  it('does not mix a non-actionable account-state item into the activity selection', () => {
    const categories = buildPrivacyResultCategories(
      [item('ready', 'browserActivity', 4), item('empty', 'browserAccountState', 0, 'empty')],
      ['ready']
    );

    expect(categories[0]?.selection).toBe('all');
    expect(categories[1]?.selection).toBe('none');
  });
});

describe('privacy result source groups', () => {
  it('marks application sources without synthetic profile rows', () => {
    const applicationItem = {
      ...item('vscode-cache', 'applicationActivity', 8),
      sourceId: 'vscode',
      sourceName: 'Visual Studio Code',
      profileId: null,
      profileName: null,
      kind: 'applicationCache' as const,
    };

    const groups = buildPrivacyResultSourceGroups([applicationItem], []);

    expect(groups[0]).toMatchObject({ id: 'vscode', hasProfiles: false, traceCount: 8 });
    expect(groups[0]?.profiles).toHaveLength(1);
  });

  it('groups profiles under their browser and keeps zero-result rows', () => {
    const defaultHistory = item('default-history', 'browserActivity', 4);
    const profileDownload = {
      ...item('profile-download', 'browserActivity', 0, 'empty'),
      profileId: 'fixture-browser:Profile 1',
      profileName: 'Profile 1',
      kind: 'downloadHistory' as const,
    };
    const firefoxHistory = {
      ...item('firefox-history', 'browserActivity', 2),
      sourceId: 'firefox',
      sourceName: 'Firefox',
      profileId: 'firefox:default-release',
      profileName: 'default-release',
    };

    const groups = buildPrivacyResultSourceGroups(
      [defaultHistory, profileDownload, firefoxHistory],
      ['default-history']
    );

    expect(groups).toHaveLength(2);
    const fixtureGroup = groups.find(group => group.id === 'fixture-browser');
    expect(fixtureGroup).toMatchObject({
      id: 'fixture-browser',
      traceCount: 4,
      selection: 'all',
    });
    expect(fixtureGroup?.profiles.map(profile => [profile.profileName, profile.traceCount])).toEqual([
      ['Default', 4],
      ['Profile 1', 0],
    ]);
    expect(fixtureGroup?.items.map(result => [result.kind, result.itemCount])).toEqual([
      ['browsingHistory', 4],
      ['downloadHistory', 0],
    ]);
  });

  it('keeps Brave and Brave Origin selection independent for equally named profiles', () => {
    const items = ['brave', 'brave-origin'].map(sourceId => ({
      ...item(sourceId, 'browserActivity', 1),
      sourceId,
      sourceName: sourceId === 'brave' ? 'Brave' : 'Brave Origin',
      profileId: `${sourceId}:Default`,
    }));
    const groups = buildPrivacyResultSourceGroups(items, ['brave-origin']);
    expect(groups.map(group => [group.id, group.selection, group.profiles[0]?.id])).toEqual([
      ['brave', 'none', 'brave:brave:Default'],
      ['brave-origin', 'all', 'brave-origin:brave-origin:Default'],
    ]);
  });

  it('sorts browser groups by the stable product priority instead of discovery order', () => {
    const sourceNames: Record<string, string> = {
      chrome: 'Google Chrome',
      edge: 'Microsoft Edge',
      firefox: 'Firefox',
      safari: 'Safari',
      opera: 'Opera',
      samsung_internet: 'Samsung Internet',
      brave: 'Brave',
      'brave-origin': 'Brave Origin',
      '360_safe_browser': '360 Safe Browser',
      yandex: 'Yandex Browser',
      qq_browser: 'QQ Browser',
      vivaldi: 'Vivaldi',
      chromium: 'Chromium',
    };
    const discoveryOrder = [
      'qq_browser',
      'chromium',
      'vivaldi',
      'yandex',
      '360_safe_browser',
      'brave',
      'brave-origin',
      'samsung_internet',
      'opera',
      'safari',
      'firefox',
      'edge',
      'chrome',
    ];
    const items = discoveryOrder.map(sourceId => ({
      ...item(sourceId, 'browserActivity', 1),
      sourceId,
      sourceName: sourceNames[sourceId] ?? sourceId,
      profileId: `${sourceId}:Default`,
    }));

    expect(buildPrivacyResultSourceGroups(items, []).map(group => group.id)).toEqual([
      'chrome',
      'edge',
      'firefox',
      'safari',
      'opera',
      'samsung_internet',
      'brave',
      'brave-origin',
      '360_safe_browser',
      'yandex',
      'qq_browser',
      'vivaldi',
      'chromium',
    ]);
  });

  it('keeps expanded Chromium traces in a stable decision-oriented order', () => {
    const kinds: PrivacyItem['kind'][] = [
      'websiteIcons',
      'browserCache',
      'frequentlyVisitedSites',
      'addressBarShortcuts',
      'downloadHistory',
      'searchHistory',
      'browsingHistory',
    ];
    const items = kinds.map(kind => ({
      ...item(kind, 'browserActivity', 1),
      kind,
    }));

    expect(buildPrivacyResultSourceGroups(items, [])[0]?.items.map(result => result.kind)).toEqual([
      'browsingHistory',
      'searchHistory',
      'downloadHistory',
      'addressBarShortcuts',
      'frequentlyVisitedSites',
      'browserCache',
      'websiteIcons',
    ]);
  });
});
