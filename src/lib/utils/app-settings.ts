import {
  DEFAULT_DUPLICATE_FILE_MINIMUM_PRESET,
  DEFAULT_DUPLICATE_KEEPER_RULE,
  DUPLICATE_FILE_MINIMUM_PRESETS,
  DUPLICATE_KEEPER_RULE_IDS,
} from '@/lib/models/duplicate-file';
import { DEFAULT_LARGE_FILE_MINIMUM_PRESET, LARGE_FILE_MINIMUM_PRESETS } from '@/lib/models/large-file';
import type { ByteSizePreset } from '@/lib/models/byte-size';
import { isLanguageId, LANGUAGE_IDS, THEME_IDS } from '@/lib/models/settings';
import type { AppSettings } from '@/lib/models/settings';
import { ByteSizePresetUtils } from '@/lib/utils/byte-size-preset';
import { BYTE_UNIT_BASES, type ByteUnitBase } from '@/lib/utils/format';

type UnknownRecord = Readonly<Record<string, unknown>>;

/**
 * Settings validation is deterministic and independent of persistence.
 * Incomplete documents are rejected, while recognized threshold presets are
 * normalized across platform bases without weakening the rest of the schema.
 */
export class AppSettingsUtils {
  static defaults(
    language: AppSettings['language'] = LANGUAGE_IDS.enUS,
    unitBase: ByteUnitBase = BYTE_UNIT_BASES.binary
  ): AppSettings {
    return {
      language,
      theme: THEME_IDS.system,
      largeFileMinimumBytes: ByteSizePresetUtils.bytes(DEFAULT_LARGE_FILE_MINIMUM_PRESET, unitBase),
      duplicateFileMinimumBytes: ByteSizePresetUtils.bytes(DEFAULT_DUPLICATE_FILE_MINIMUM_PRESET, unitBase),
      duplicateKeeperRule: DEFAULT_DUPLICATE_KEEPER_RULE,
      aiApiKey: '',
      aiApiBaseUrl: '',
      aiModel: '',
    };
  }

  static parse(value: unknown, unitBase: ByteUnitBase = BYTE_UNIT_BASES.binary): AppSettings {
    const rawSettings = typeof value === 'object' && value !== null ? { ...(value as Record<string, unknown>) } : {};
    
    // Backwards compatibility for versions before AI Advisor
    if (rawSettings.language && !rawSettings.aiApiKey) {
      rawSettings.aiApiKey = '';
      rawSettings.aiApiBaseUrl = '';
      rawSettings.aiModel = '';
    }

    if (
      !AppSettingsUtils.hasExactKeys(rawSettings, [
        'language',
        'theme',
        'largeFileMinimumBytes',
        'duplicateFileMinimumBytes',
        'duplicateKeeperRule',
        'aiApiKey',
        'aiApiBaseUrl',
        'aiModel',
      ])
    ) {
      throw new Error('Invalid app settings document');
    }
    const settings = rawSettings as unknown as AppSettings;
    const largeFileMinimumBytes = AppSettingsUtils.normalizePresetBytes(
      settings.largeFileMinimumBytes,
      LARGE_FILE_MINIMUM_PRESETS,
      unitBase
    );
    const duplicateFileMinimumBytes = AppSettingsUtils.normalizePresetBytes(
      settings.duplicateFileMinimumBytes,
      DUPLICATE_FILE_MINIMUM_PRESETS,
      unitBase
    );
    if (
      !isLanguageId(settings.language) ||
      !AppSettingsUtils.includes(Object.values(THEME_IDS), settings.theme) ||
      largeFileMinimumBytes === null ||
      duplicateFileMinimumBytes === null ||
      !AppSettingsUtils.includes(Object.values(DUPLICATE_KEEPER_RULE_IDS), settings.duplicateKeeperRule)
    ) {
      throw new Error('Invalid app settings value');
    }
    return {
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
  private static normalizePresetBytes(
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

  private static hasExactKeys<const Keys extends readonly string[]>(
    value: unknown,
    expectedKeys: Keys
  ): value is UnknownRecord & Record<Keys[number], unknown> {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
    const actualKeys = Object.keys(value).sort();
    return actualKeys.length === expectedKeys.length && expectedKeys.every(key => actualKeys.includes(key));
  }

  private static includes<const Values extends readonly unknown[]>(
    values: Values,
    value: unknown
  ): value is Values[number] {
    return values.includes(value);
  }
}
