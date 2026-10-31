import { ICON_NAMES } from '@/lib/models/presentation/ui';

export const APP_NAME = 'MangoDisk' as const;
export const APP_ICON_PATH = '/mangodisk.svg' as const;
export const APP_SHELL_EXPANDED_MIN_WIDTH_PX = 1100;

export function isAppShellExpanded(viewportWidth: number): boolean {
  return viewportWidth >= APP_SHELL_EXPANDED_MIN_WIDTH_PX;
}

export interface SidebarLayoutState {
  expanded: boolean;
  preferredExpanded: boolean | null;
  wideViewport: boolean;
}

export function createSidebarLayoutState(
  viewportWidth: number,
  preferredExpanded: boolean | null = null
): SidebarLayoutState {
  const wideViewport = isAppShellExpanded(viewportWidth);
  return {
    expanded: preferredExpanded ?? wideViewport,
    preferredExpanded,
    wideViewport,
  };
}

/**
 * Window width controls the layout until the user makes an explicit choice.
 * Resizing must not override that choice or turn a responsive collapse into a
 * persisted preference.
 */
export function resizeSidebarLayout(state: SidebarLayoutState, viewportWidth: number): SidebarLayoutState {
  const wideViewport = isAppShellExpanded(viewportWidth);
  if (wideViewport === state.wideViewport) return state;
  return {
    expanded: state.preferredExpanded ?? wideViewport,
    preferredExpanded: state.preferredExpanded,
    wideViewport,
  };
}

export function toggleSidebarLayout(state: SidebarLayoutState): SidebarLayoutState {
  const expanded = !state.expanded;
  return {
    expanded,
    preferredExpanded: expanded,
    wideViewport: state.wideViewport,
  };
}

export const PROJECT_LINKS = {
  website: 'https://mangodisk.app',
  repository: 'https://github.com/harry0703/mangodisk',
  issues: 'https://github.com/harry0703/mangodisk/issues',
  license: 'https://github.com/harry0703/mangodisk/blob/main/LICENSE',
} as const;

export const PAGE_IDS = {
  systemOptimization: 'system-optimization',
  systemMaintenance: 'system-maintenance',
  cleanup: 'cleanup',
  analysis: 'analysis',
  largeFiles: 'large-files',
  duplicateFiles: 'duplicate-files',
  applicationUninstall: 'application-uninstall',
  privacy: 'privacy',
  startup: 'startup',
  history: 'history',
  settings: 'settings',
} as const;

export type PageId = (typeof PAGE_IDS)[keyof typeof PAGE_IDS];

const LINUX_UNAVAILABLE_PAGES: ReadonlySet<PageId> = new Set([
  PAGE_IDS.applicationUninstall,
  PAGE_IDS.startup,
  PAGE_IDS.systemOptimization,
]);

/**
 * Keeps navigation aligned with capabilities that have an actual platform
 * implementation. Programmatic navigation uses the same boundary so tray and
 * window events cannot expose a non-functional workspace.
 */
export function isPageAvailableOnPlatform(page: PageId, platform: string): boolean {
  return platform !== 'linux' || !LINUX_UNAVAILABLE_PAGES.has(page);
}

export const PRIMARY_NAV_GROUPS = [
  {
    id: 'storage',
    titleKey: 'navigationGroups.storage',
    items: [
      { id: PAGE_IDS.cleanup, icon: ICON_NAMES.deepCleanup },
      { id: PAGE_IDS.largeFiles, icon: ICON_NAMES.largeFiles },
      { id: PAGE_IDS.duplicateFiles, icon: ICON_NAMES.duplicateFiles },
      { id: PAGE_IDS.analysis, icon: ICON_NAMES.analysis },
    ],
  },
  {
    id: 'privacy',
    titleKey: 'navigationGroups.privacy',
    items: [{ id: PAGE_IDS.privacy, icon: ICON_NAMES.shield }],
  },
  {
    id: 'system',
    titleKey: 'navigationGroups.system',
    items: [
      { id: PAGE_IDS.applicationUninstall, icon: ICON_NAMES.uninstall },
      { id: PAGE_IDS.startup, icon: ICON_NAMES.startup },
      { id: PAGE_IDS.systemOptimization, icon: ICON_NAMES.systemOptimization },
      { id: PAGE_IDS.systemMaintenance, icon: ICON_NAMES.systemMaintenance },
    ],
  },
] as const;

export function primaryNavGroupsForPlatform(platform: string) {
  return PRIMARY_NAV_GROUPS.map(group => ({
    ...group,
    items: group.items.filter(item => isPageAvailableOnPlatform(item.id, platform)),
  })).filter(group => group.items.length > 0);
}

export const SECONDARY_NAV_ITEMS = [
  { id: PAGE_IDS.history, icon: ICON_NAMES.history },
  { id: PAGE_IDS.settings, icon: ICON_NAMES.settings },
] as const;
