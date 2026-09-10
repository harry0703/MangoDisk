import { afterEach, describe, expect, it, vi } from 'vitest';

import { i18n } from '@/i18n';
import { LANGUAGE_IDS, LANGUAGE_OPTIONS } from '@/lib/models/settings';
import { LanguageService } from '@/lib/services/language-service';
import enUS from '@/locales/en-US.json';
import jaJP from '@/locales/ja-JP.json';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';

const localeResources = [zhCN, zhTW, jaJP, enUS];

function leafKeys(value: unknown, prefix = ''): string[] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return [prefix];
  return Object.entries(value).flatMap(([key, child]) => leafKeys(child, prefix ? `${prefix}.${key}` : key));
}

function leafValues(value: unknown): unknown[] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return [value];
  return Object.values(value).flatMap(leafValues);
}

function leafEntries(value: unknown, prefix = ''): Array<[string, unknown]> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return [[prefix, value]];
  return Object.entries(value).flatMap(([key, child]) => leafEntries(child, prefix ? `${prefix}.${key}` : key));
}

describe('i18n resources', () => {
  afterEach(() => {
    i18n.global.locale.value = LANGUAGE_IDS.zhCN;
    vi.unstubAllGlobals();
  });

  it('keeps shared schema keys aligned across locales', () => {
    const chineseKeys = new Set(leafKeys(zhCN));
    for (const resource of localeResources) {
      const resourceKeys = new Set(leafKeys(resource));
      const missingKeys = [...chineseKeys].filter(key => !resourceKeys.has(key));
      // Curated rules may vary by locale, but shared UI keys must never fall back unexpectedly.
      const unexpectedKeys = [...resourceKeys].filter(
        key => !chineseKeys.has(key) && !key.startsWith('cleanupRules.entries.')
      );

      expect(missingKeys).toEqual([]);
      expect(unexpectedKeys).toEqual([]);
    }
  });

  it('keeps selectable languages aligned with bundled locale resources', () => {
    expect([...i18n.global.availableLocales].sort()).toEqual(LANGUAGE_OPTIONS.map(option => option.id).sort());
    for (const locale of i18n.global.availableLocales) {
      for (const option of LANGUAGE_OPTIONS) {
        expect(i18n.global.te(option.labelKey, locale)).toBe(true);
      }
    }
  });

  it('contains only non-empty localized strings', () => {
    for (const resource of localeResources) {
      const invalidValues = leafValues(resource).filter(value => typeof value !== 'string' || !value.trim());
      expect(invalidValues).toEqual([]);
    }
  });

  it('keeps compact interface copy free of trailing periods', () => {
    for (const resource of localeResources) {
      const keysWithTrailingPeriods = leafEntries(resource)
        .filter(([key, value]) => {
          if (key.startsWith('cleanupRules.entries.')) return false;
          return typeof value === 'string' && (value.endsWith('。') || value.endsWith('.'));
        })
        .map(([key]) => key);

      expect(keysWithTrailingPeriods).toEqual([]);
    }
  });

  it('keeps every curated rule presentation complete', () => {
    for (const resource of localeResources) {
      const incompleteRules = Object.entries(resource.cleanupRules.entries)
        .filter(([, rule]) => !rule.name.trim() || !rule.description.trim() || !rule.impact.trim())
        .map(([ruleId]) => ruleId);

      expect(incompleteRules).toEqual([]);
    }
  });

  it('synchronizes the composer and document language', () => {
    const documentStub = { documentElement: { lang: '' } };
    vi.stubGlobal('document', documentStub);

    LanguageService.apply(LANGUAGE_IDS.enUS);

    expect(i18n.global.locale.value).toBe(LANGUAGE_IDS.enUS);
    expect(documentStub.documentElement.lang).toBe(LANGUAGE_IDS.enUS);
    expect(i18n.global.t('common.cancel')).toBe('Cancel');
  });

  it('resolves the disk analysis open action in every locale', () => {
    const expectedLabels = {
      [LANGUAGE_IDS.enUS]: 'Open',
      [LANGUAGE_IDS.jaJP]: '開く',
      [LANGUAGE_IDS.koKR]: '열기',
      [LANGUAGE_IDS.zhCN]: '打开',
      [LANGUAGE_IDS.zhTW]: '開啟',
    };

    for (const locale of i18n.global.availableLocales) {
      i18n.global.locale.value = locale;
      expect(i18n.global.t('common.open')).toBe(expectedLabels[locale]);
    }
  });

  it('matches supported system languages and falls back to English', () => {
    expect(LanguageService.resolveSupportedLanguage(['zh-Hans-CN', 'en-US'])).toBe(LANGUAGE_IDS.zhCN);
    expect(LanguageService.resolveSupportedLanguage(['fr-FR', 'en-GB'])).toBe(LANGUAGE_IDS.enUS);
    expect(LanguageService.resolveSupportedLanguage(['zh-Hant-HK', 'en-US'])).toBe(LANGUAGE_IDS.zhTW);
    expect(LanguageService.resolveSupportedLanguage(['ja-JP'])).toBe(LANGUAGE_IDS.jaJP);
    expect(LanguageService.resolveSupportedLanguage(['ko-KR', 'en-US'])).toBe(LANGUAGE_IDS.koKR);
    expect(LanguageService.resolveSupportedLanguage(['ko'])).toBe(LANGUAGE_IDS.koKR);
  });

  it('applies interpolation and pluralization for the active locale', () => {
    i18n.global.locale.value = LANGUAGE_IDS.zhCN;
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 个文件');

    i18n.global.locale.value = LANGUAGE_IDS.enUS;
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe('1 file');
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 files');

    i18n.global.locale.value = LANGUAGE_IDS.zhTW;
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 個檔案');

    i18n.global.locale.value = LANGUAGE_IDS.jaJP;
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 ファイル');

    i18n.global.locale.value = LANGUAGE_IDS.koKR;
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2개 파일');
  });
});
