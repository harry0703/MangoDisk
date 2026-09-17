import type {
  SystemSettingChangeSelectionItem,
  SystemSettingItem,
  SystemSettingsCatalog,
} from '@/lib/models/system-settings';

export const SYSTEM_OPTIMIZATION_MODE_IDS = ['unchanged', 'smart', 'performance', 'privacy'] as const;
export type SystemOptimizationPresetMode = (typeof SYSTEM_OPTIMIZATION_MODE_IDS)[number];
export type SystemOptimizationMode = SystemOptimizationPresetMode | 'manual';

/**
 * Smart extras are broadly useful settings that remain opt-out in the detailed list. More focused
 * modes inherit this baseline, so choosing a stronger mode never removes the practical defaults.
 */
const SMART_EXTRAS: Record<SystemSettingsCatalog['platform'], ReadonlySet<string>> = {
  macos: new Set([
    'macos.finder.disable-extension-warning',
    'macos.finder.default-list-view',
    'macos.finder.folders-first-on-desktop',
    'macos.finder.show-external-drives-on-desktop',
    'macos.finder.show-removable-media-on-desktop',
    'macos.safari.show-status-bar',
    'macos.activity-monitor.show-all-processes',
    'macos.keyboard.full-navigation',
    'macos.printing.quit-after-finish',
    'macos.text.disable-auto-correct',
    'macos.text.disable-smart-quotes',
    'macos.text.disable-smart-dashes',
    'macos.text.disable-auto-capitalization',
    'macos.text.disable-period-substitution',
    'macos.dock.enable-spring-loading',
    'macos.dock.scroll-to-expose',
    'macos.sound.disable-volume-feedback',
  ]),
  windows: new Set([
    'windows.explorer.show-hidden-files',
    'windows.explorer.launch-this-pc',
    'windows.explorer.compact-mode',
    'windows.explorer.show-full-path',
    'windows.explorer.show-item-checkboxes',
    'windows.explorer.show-status-bar',
    'windows.explorer.disable-aero-shake',
    'windows.explorer.classic-context-menu',
    'windows.taskbar.enable-end-task',
    'windows.taskbar.hide-widgets',
    'windows.taskbar.disable-widgets-board',
    'windows.taskbar.hide-search',
    'windows.taskbar.hide-search-policy',
    'windows.taskbar.hide-weather',
    'windows.taskbar.hide-chat',
    'windows.taskbar.hide-copilot',
    'windows.taskbar.show-desktop-corner',
    'windows.accessibility.disable-sticky-keys-shortcut',
    'windows.accessibility.disable-filter-keys-shortcut',
    'windows.accessibility.disable-toggle-keys-shortcut',
    'windows.storage.enable-storage-sense',
    'windows.edge.disable-sidebar',
    'windows.ai.disable-copilot',
    'windows.update.prevent-restart-when-logged-on',
    'windows.filesystem.enable-long-paths',
    'windows.recovery.enable-registry-backups',
    'windows.explorer.confirm-file-delete',
    'windows.explorer.use-manual-default-printer',
    'windows.taskbar.show-on-all-displays',
    'windows.update.enable-microsoft-product-updates',
    'windows.update.enable-restart-notifications',
  ]),
  linux: new Set(),
};

/**
 * Focused modes automatically include their matching intent categories. This list only contains
 * cross-category settings whose effect still belongs to that mode. High-risk settings are filtered
 * separately and always remain explicit manual choices.
 */
const FOCUSED_EXTRAS: Record<
  SystemSettingsCatalog['platform'],
  Record<Exclude<SystemOptimizationPresetMode, 'unchanged' | 'smart'>, ReadonlySet<string>>
> = {
  macos: {
    performance: new Set(['macos.keyboard.disable-press-and-hold']),
    privacy: new Set(['macos.sharing.disable-airdrop']),
  },
  windows: {
    performance: new Set(['windows.performance.reduce-crash-dump', 'windows.storage.disable-ntfs-last-access']),
    privacy: new Set([
      'windows.edge.disable-sidebar',
      'windows.cloud.disable-onedrive-sync',
      'windows.input.disable-windows-ink',
      'windows.taskbar.hide-chat',
    ]),
  },
  linux: {
    performance: new Set(),
    privacy: new Set(),
  },
};

export function isSystemOptimizationPresetMode(value: unknown): value is SystemOptimizationPresetMode {
  return SYSTEM_OPTIMIZATION_MODE_IDS.includes(value as SystemOptimizationPresetMode);
}

function matchesFocusedCategory(item: SystemSettingItem, mode: SystemOptimizationPresetMode): boolean {
  if (mode === 'performance') return item.category === 'performance' || item.category === 'gaming';
  if (mode === 'privacy') return item.category === 'privacy';
  return false;
}

/** Hides platform presets that currently cannot add anything beyond the smart baseline. */
export function systemOptimizationModesForPlatform(
  platform: SystemSettingsCatalog['platform']
): SystemOptimizationPresetMode[] {
  return SYSTEM_OPTIMIZATION_MODE_IDS.filter(
    mode => mode === 'unchanged' || mode === 'smart' || FOCUSED_EXTRAS[platform][mode].size > 0
  );
}

/** Returns only settings that still need a change and are available on the current machine. */
export function systemOptimizationSelectionForMode(
  catalog: SystemSettingsCatalog,
  mode: SystemOptimizationPresetMode
): string[] {
  if (mode === 'unchanged') return [];
  const smartExtras = SMART_EXTRAS[catalog.platform];
  const focusedExtras = mode === 'smart' ? null : FOCUSED_EXTRAS[catalog.platform][mode];
  return catalog.items
    .filter(
      item =>
        item.status === 'recommended' &&
        item.riskLevel !== 'high' &&
        (item.selectedByDefault ||
          smartExtras.has(item.settingId) ||
          focusedExtras?.has(item.settingId) === true ||
          matchesFocusedCategory(item, mode))
    )
    .map(item => item.settingId);
}

/**
 * Presets are additive: they enable their recommended settings without silently disabling an
 * optimization that is already active. The returned state is a draft and has no side effects.
 */
export function systemOptimizationDesiredIdsForMode(
  catalog: SystemSettingsCatalog,
  mode: SystemOptimizationPresetMode
): string[] {
  const desired = new Set(catalog.items.filter(item => item.status === 'optimized').map(item => item.settingId));
  for (const settingId of systemOptimizationSelectionForMode(catalog, mode)) desired.add(settingId);
  return catalog.items.filter(item => desired.has(item.settingId)).map(item => item.settingId);
}

/** Returns the smallest typed batch required to move the scanned state to the UI draft. */
export function systemOptimizationPendingChanges(
  catalog: SystemSettingsCatalog,
  desiredOptimizedIds: Iterable<string>
): SystemSettingChangeSelectionItem[] {
  const desired = new Set(desiredOptimizedIds);
  return catalog.items
    .filter(item => item.status !== 'unavailable' && desired.has(item.settingId) !== (item.status === 'optimized'))
    .map(item => ({
      settingId: item.settingId,
      target: desired.has(item.settingId) ? 'optimized' : 'default',
    }));
}

/**
 * High-risk confirmation applies only when enabling a trade-off. Restoring the operating-system
 * state must remain direct so the warning never discourages a safer reversal.
 */
export function systemOptimizationHighRiskEnables(
  catalog: SystemSettingsCatalog,
  changes: readonly SystemSettingChangeSelectionItem[]
): SystemSettingItem[] {
  const enabledIds = new Set(changes.filter(change => change.target === 'optimized').map(change => change.settingId));
  return catalog.items.filter(item => item.riskLevel === 'high' && enabledIds.has(item.settingId));
}
