import { describe, expect, it } from 'vitest';

import {
  APP_SHELL_EXPANDED_MIN_WIDTH_PX,
  createSidebarLayoutState,
  isAppShellExpanded,
  isPageAvailableOnPlatform,
  PAGE_IDS,
  PRIMARY_NAV_GROUPS,
  primaryNavGroupsForPlatform,
  resizeSidebarLayout,
  toggleSidebarLayout,
} from './application-shell';
import { ICON_NAMES } from './ui';

describe('application shell layout', () => {
  it('expands the sidebar at the desktop shell breakpoint', () => {
    expect(isAppShellExpanded(APP_SHELL_EXPANDED_MIN_WIDTH_PX - 1)).toBe(false);
    expect(isAppShellExpanded(APP_SHELL_EXPANDED_MIN_WIDTH_PX)).toBe(true);
  });

  it('keeps the user toggle stable while resizing within the same responsive range', () => {
    const collapsed = toggleSidebarLayout(createSidebarLayoutState(APP_SHELL_EXPANDED_MIN_WIDTH_PX));

    expect(resizeSidebarLayout(collapsed, APP_SHELL_EXPANDED_MIN_WIDTH_PX + 200)).toEqual(collapsed);
  });

  it('collapses on a narrow viewport and restores the last explicit preference when widened', () => {
    const expanded = createSidebarLayoutState(APP_SHELL_EXPANDED_MIN_WIDTH_PX);
    const narrow = resizeSidebarLayout(expanded, APP_SHELL_EXPANDED_MIN_WIDTH_PX - 1);
    const manuallyCollapsed = toggleSidebarLayout(expanded);
    const narrowAfterManualCollapse = resizeSidebarLayout(manuallyCollapsed, APP_SHELL_EXPANDED_MIN_WIDTH_PX - 1);

    expect(narrow).toMatchObject({ expanded: false, preferredExpanded: true, wideViewport: false });
    expect(resizeSidebarLayout(narrow, APP_SHELL_EXPANDED_MIN_WIDTH_PX)).toMatchObject({
      expanded: true,
      preferredExpanded: true,
      wideViewport: true,
    });
    expect(resizeSidebarLayout(narrowAfterManualCollapse, APP_SHELL_EXPANDED_MIN_WIDTH_PX)).toMatchObject({
      expanded: false,
      preferredExpanded: false,
      wideViewport: true,
    });
  });

  it('allows an explicit expansion in a narrow window', () => {
    const narrow = createSidebarLayoutState(APP_SHELL_EXPANDED_MIN_WIDTH_PX - 1);

    expect(toggleSidebarLayout(narrow)).toMatchObject({ expanded: true, preferredExpanded: true });
  });

  it('separates storage, privacy, and system tools by user task', () => {
    expect(PRIMARY_NAV_GROUPS.map(group => group.id)).toEqual(['storage', 'privacy', 'system']);
    expect(PRIMARY_NAV_GROUPS[0].items.map(item => item.id)).toEqual([
      PAGE_IDS.cleanup,
      PAGE_IDS.largeFiles,
      PAGE_IDS.duplicateFiles,
      PAGE_IDS.analysis,
    ]);
    expect(PRIMARY_NAV_GROUPS[1].items.map(item => item.id)).toEqual([PAGE_IDS.privacy]);
    expect(PRIMARY_NAV_GROUPS[2].items.map(item => item.id)).toEqual([
      PAGE_IDS.applicationUninstall,
      PAGE_IDS.startup,
      PAGE_IDS.systemOptimization,
      PAGE_IDS.systemMaintenance,
    ]);
  });

  it('uses the dedicated acceleration icon for system optimization', () => {
    const systemItems = PRIMARY_NAV_GROUPS[2].items;
    expect(systemItems.find(item => item.id === PAGE_IDS.systemOptimization)?.icon).toBe(ICON_NAMES.systemOptimization);
    expect(systemItems.find(item => item.id === PAGE_IDS.systemMaintenance)?.icon).toBe(ICON_NAMES.systemMaintenance);
  });

  it('hides Linux workspaces whose platform operations are unavailable', () => {
    expect(isPageAvailableOnPlatform(PAGE_IDS.applicationUninstall, 'linux')).toBe(false);
    expect(isPageAvailableOnPlatform(PAGE_IDS.startup, 'linux')).toBe(false);
    expect(isPageAvailableOnPlatform(PAGE_IDS.systemOptimization, 'linux')).toBe(false);
    expect(isPageAvailableOnPlatform(PAGE_IDS.systemMaintenance, 'linux')).toBe(true);

    expect(primaryNavGroupsForPlatform('linux')[2]?.items.map(item => item.id)).toEqual([PAGE_IDS.systemMaintenance]);
  });

  it('keeps the complete navigation on supported desktop platforms', () => {
    for (const platform of ['macos', 'windows']) {
      expect(primaryNavGroupsForPlatform(platform)).toEqual(PRIMARY_NAV_GROUPS);
    }
  });
});
