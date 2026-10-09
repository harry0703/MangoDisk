import type { PrivacyCategory, PrivacyDataKind, PrivacyItem } from '@/lib/models/privacy';
import { isPrivacyItemActionable } from '@/lib/utils/privacy-selection';

export type PrivacyCategorySelection = 'all' | 'partial' | 'none';

export interface PrivacyResultCategory {
  id: PrivacyCategory;
  items: PrivacyItem[];
  itemCount: number;
  traceCount: number;
  selectedTraceCount: number;
  selection: PrivacyCategorySelection;
}

export interface PrivacyResultSourceGroup {
  id: string;
  sourceName: string;
  profiles: PrivacyResultProfileGroup[];
  items: PrivacyItem[];
  traceCount: number;
  selection: PrivacyCategorySelection;
  hasProfiles: boolean;
  permissionRequired: boolean;
}

export interface PrivacyResultProfileGroup {
  id: string;
  profileName: string;
  items: PrivacyItem[];
  traceCount: number;
  selection: PrivacyCategorySelection;
}

// Browser activity and account state are separate decisions: one primarily removes history,
// while the other can sign users out or delete personal browser data. Keeping both visible makes
// the capability boundary clear without inventing presentation-only categories.
const PRIVACY_CATEGORY_ORDER: readonly PrivacyCategory[] = [
  'browserActivity',
  'browserAccountState',
  'applicationActivity',
  'systemActivity',
];
// Keep browser discovery independent from product presentation. The adapters may enumerate
// sources in different orders on each platform, while the UI follows one market-familiar order.
const PRIVACY_BROWSER_SOURCE_ORDER: readonly string[] = [
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
];
const PRIVACY_BROWSER_SOURCE_PRIORITY = new Map(
  PRIVACY_BROWSER_SOURCE_ORDER.map((sourceId, index) => [sourceId, index])
);
const PRIVACY_KIND_ORDER: readonly PrivacyDataKind[] = [
  'browsingHistory',
  'searchHistory',
  'downloadHistory',
  'addressBarShortcuts',
  'frequentlyVisitedSites',
  'sessions',
  'browserCache',
  'websiteIcons',
  'cookies',
  'siteStorage',
  'sitePermissions',
  'savedPasswords',
  'autofillData',
  'currentClipboard',
  'clipboardHistory',
  'recentItems',
  'recentApplications',
  'applicationUsageHistory',
  'networkConnectionHistory',
  'folderViewHistory',
  'printerHistory',
  'jumpLists',
  'runDialogHistory',
  'fileDialogHistory',
  'systemSearchHistory',
  'explorerPathHistory',
  'shellHistory',
  'applicationCache',
  'applicationLogs',
  'applicationSessions',
  'editorLocalHistory',
  'recentDocuments',
  'recentProjects',
  'recentConnections',
  'playbackHistory',
  'recentPaths',
  'recentSearches',
];

/**
 * Builds the stable master navigation for privacy results. Categories remain
 * presentation-only: opaque item tokens continue to be the only values sent
 * back to Core when the user changes a selection.
 */
export function buildPrivacyResultCategories(
  items: readonly PrivacyItem[],
  selectedTokens: readonly string[]
): PrivacyResultCategory[] {
  const selected = new Set(selectedTokens);

  return PRIVACY_CATEGORY_ORDER.map<PrivacyResultCategory>(id => {
    const categoryItems = items.filter(item => item.category === id);
    const actionableItems = categoryItems.filter(isPrivacyItemActionable);
    const selectedItems = actionableItems.filter(item => selected.has(item.token));

    return {
      id,
      items: categoryItems,
      itemCount: categoryItems.length,
      traceCount: categoryItems.reduce((total, item) => total + item.itemCount, 0),
      selectedTraceCount: selectedItems.reduce((total, item) => total + item.itemCount, 0),
      selection:
        selectedItems.length === 0 ? 'none' : selectedItems.length === actionableItems.length ? 'all' : 'partial',
    };
  }).filter(category => category.items.length > 0);
}

/**
 * Groups browser results by provider and stable profile identity while retaining
 * every scanned row, including empty rows. Friendly profile names are presentation
 * labels only and never replace the stable identity used to restore selections.
 */
export function buildPrivacyResultSourceGroups(
  items: readonly PrivacyItem[],
  selectedTokens: readonly string[]
): PrivacyResultSourceGroup[] {
  const selected = new Set(selectedTokens);
  const groups = new Map<string, PrivacyItem[]>();

  for (const item of items) {
    const groupItems = groups.get(item.sourceId);
    if (groupItems) groupItems.push(item);
    else groups.set(item.sourceId, [item]);
  }

  return [...groups.entries()]
    .map<PrivacyResultSourceGroup>(([id, groupItems]) => {
      const profileItems = new Map<string, { profileName: string; items: PrivacyItem[] }>();
      for (const item of groupItems) {
        const profileId = item.profileId ?? '';
        const profile = profileItems.get(profileId);
        if (profile) profile.items.push(item);
        else profileItems.set(profileId, { profileName: item.profileName ?? '', items: [item] });
      }
      const profiles = [...profileItems.entries()]
        .sort(([, left], [, right]) => left.profileName.localeCompare(right.profileName))
        .map<PrivacyResultProfileGroup>(([profileId, { profileName, items }]) => {
          const orderedItems = [...items].sort(
            (left, right) => PRIVACY_KIND_ORDER.indexOf(left.kind) - PRIVACY_KIND_ORDER.indexOf(right.kind)
          );
          const actionableItems = orderedItems.filter(isPrivacyItemActionable);
          const selectedItems = actionableItems.filter(item => selected.has(item.token));
          return {
            id: `${id}:${profileId}`,
            profileName,
            items: orderedItems,
            traceCount: orderedItems.reduce((total, item) => total + item.itemCount, 0),
            selection:
              selectedItems.length === 0 ? 'none' : selectedItems.length === actionableItems.length ? 'all' : 'partial',
          };
        });
      const orderedItems = profiles.flatMap(profile => profile.items);
      const actionableItems = orderedItems.filter(isPrivacyItemActionable);
      const selectedItems = actionableItems.filter(item => selected.has(item.token));

      return {
        id,
        sourceName: orderedItems[0]?.sourceName ?? id,
        profiles,
        items: orderedItems,
        traceCount: orderedItems.reduce((total, item) => total + item.itemCount, 0),
        selection:
          selectedItems.length === 0 ? 'none' : selectedItems.length === actionableItems.length ? 'all' : 'partial',
        hasProfiles: profiles.some(profile => profile.profileName.length > 0),
        permissionRequired: orderedItems.some(item => item.capability === 'permissionRequired'),
      };
    })
    .sort((left, right) => {
      const priorityDifference =
        (PRIVACY_BROWSER_SOURCE_PRIORITY.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
        (PRIVACY_BROWSER_SOURCE_PRIORITY.get(right.id) ?? Number.MAX_SAFE_INTEGER);
      return priorityDifference || left.sourceName.localeCompare(right.sourceName) || left.id.localeCompare(right.id);
    });
}
