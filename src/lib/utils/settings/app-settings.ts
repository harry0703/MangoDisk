import {
  DEFAULT_DUPLICATE_FILE_MINIMUM_PRESET,
  DEFAULT_DUPLICATE_KEEPER_RULE,
  DUPLICATE_FILE_MINIMUM_PRESETS,
  DUPLICATE_KEEPER_RULE_IDS,
} from '@/lib/models/storage/duplicate-file';
import { DEFAULT_LARGE_FILE_MINIMUM_PRESET, LARGE_FILE_MINIMUM_PRESETS } from '@/lib/models/storage/large-file';
import type { ByteSizePreset } from '@/lib/models/settings/byte-size';
import { COLOR_THEME_IDS, isColorThemeId, isLanguageId, LANGUAGE_IDS, THEME_IDS } from '@/lib/models/settings/settings';
import type { AppSettings } from '@/lib/models/settings/settings';
import * as ByteSizePresetUtils from '@/lib/utils/settings/byte-size-preset';
import { BYTE_UNIT_BASES, type ByteUnitBase } from '@/lib/utils/presentation/format';

/** Validates and normalizes the persisted settings document at its boundary. */
type UnknownRecord = Readonly<Record<string, unknown>>;
export function defaults(
  language: AppSettings['language'] = LANGUAGE_IDS.enUS,
  unitBase: ByteUnitBase = BYTE_UNIT_BASES.binary
): AppSettings {
  return {
    hideCleanupReadFailureAlerts: false,
    sidebarExpanded: null,
    language,
    theme: THEME_IDS.system,
    colorTheme: COLOR_THEME_IDS.mango,
    largeFileMinimumBytes: ByteSizePresetUtils.bytes(DEFAULT_LARGE_FILE_MINIMUM_PRESET, unitBase),
    duplicateFileMinimumBytes: ByteSizePresetUtils.bytes(DEFAULT_DUPLICATE_FILE_MINIMUM_PRESET, unitBase),
    duplicateKeeperRule: DEFAULT_DUPLICATE_KEEPER_RULE,
  };
}
export function parse(value: unknown, unitBase: ByteUnitBase = BYTE_UNIT_BASES.binary): AppSettings {
  const legacyKeys = [
    'language',
    'theme',
    'largeFileMinimumBytes',
    'duplicateFileMinimumBytes',
    'duplicateKeeperRule',
  ] as const;
  const optionalKeys = ['hideCleanupReadFailureAlerts', 'colorTheme', 'sidebarExpanded'] as const;
  const savedOptionalKeys = optionalKeys.filter(key => typeof value === 'object' && value !== null && key in value);
  if (!hasExactKeys(value, [...legacyKeys, ...savedOptionalKeys])) {
    throw new Error('Invalid app settings document');
  }
  const settings = value;
  // Released documents omit newer choices. Preserve all saved preferences and
  // retain classic colors until the user explicitly selects another palette.
  const hideCleanupReadFailureAlerts =
    'hideCleanupReadFailureAlerts' in settings ? settings.hideCleanupReadFailureAlerts : false;
  // An unavailable palette must not discard valid language or scan preferences.
  const colorTheme = isColorThemeId(settings.colorTheme) ? settings.colorTheme : COLOR_THEME_IDS.mango;
  // Missing or invalid shell preferences retain responsive layout without
  // discarding unrelated settings from older releases.
  const sidebarExpanded = typeof settings.sidebarExpanded === 'boolean' ? settings.sidebarExpanded : null;
  const largeFileMinimumBytes = normalizePresetBytes(
    settings.largeFileMinimumBytes,
    LARGE_FILE_MINIMUM_PRESETS,
    unitBase
  );
  const duplicateFileMinimumBytes = normalizePresetBytes(
    settings.duplicateFileMinimumBytes,
    DUPLICATE_FILE_MINIMUM_PRESETS,
    unitBase
  );
  if (
    typeof hideCleanupReadFailureAlerts !== 'boolean' ||
    !isLanguageId(settings.language) ||
    !includes(Object.values(THEME_IDS), settings.theme) ||
    largeFileMinimumBytes === null ||
    duplicateFileMinimumBytes === null ||
    !includes(Object.values(DUPLICATE_KEEPER_RULE_IDS), settings.duplicateKeeperRule)
  ) {
    throw new Error('Invalid app settings value');
  }
  return {
    hideCleanupReadFailureAlerts,
    sidebarExpanded,
    colorTheme,
    language: settings.language,
    theme: settings.theme,
    largeFileMinimumBytes,
    duplicateFileMinimumBytes,
    duplicateKeeperRule: settings.duplicateKeeperRule,
  };
}
/**
 * Maps a saved threshold to the equivalent preset for the current platform.
 * Existing releases persisted binary values on every OS, so macOS must accept
 * those values and normalize them to decimal bytes without discarding unrelated
 * language, theme, or duplicate-selection preferences.
 */
function normalizePresetBytes(
  value: unknown,
  presets: readonly ByteSizePreset[],
  unitBase: ByteUnitBase
): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  const currentValues = ByteSizePresetUtils.byteValues(presets, unitBase);
  if (currentValues.includes(value)) return value;
  const previousBase = unitBase === BYTE_UNIT_BASES.decimal ? BYTE_UNIT_BASES.binary : BYTE_UNIT_BASES.decimal;
  const previousIndex = ByteSizePresetUtils.byteValues(presets, previousBase).indexOf(value);
  return previousIndex < 0 ? null : (currentValues[previousIndex] ?? null);
}
function hasExactKeys<const Keys extends readonly string[]>(
  value: unknown,
  expectedKeys: Keys
): value is UnknownRecord & Record<Keys[number], unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const actualKeys = Object.keys(value).sort();
  return actualKeys.length === expectedKeys.length && expectedKeys.every(key => actualKeys.includes(key));
}
function includes<const Values extends readonly unknown[]>(values: Values, value: unknown): value is Values[number] {
  return values.includes(value);
}
