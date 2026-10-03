import { afterEach, describe, expect, it, vi } from 'vitest';

import { i18n } from '@/i18n';
import { LANGUAGE_IDS, LANGUAGE_OPTIONS } from '@/lib/models/settings';
import { LanguageService } from '@/lib/services/language-service';
import enUS from '@/locales/en-US.json';
import jaJP from '@/locales/ja-JP.json';
import koKR from '@/locales/ko-KR.json';
import ptBR from '@/locales/pt-BR.json';
import ruRU from '@/locales/ru-RU.json';
import trTR from '@/locales/tr-TR.json';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';

const localeResources = [zhCN, zhTW, jaJP, koKR, ruRU, enUS, trTR, ptBR];

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

  it('preserves interpolation arguments in the newly integrated translations', () => {
    const englishEntries = new Map(leafEntries(enUS));
    const argumentsIn = (value: unknown) =>
      typeof value === 'string' ? [...new Set(value.match(/\{[a-zA-Z][a-zA-Z0-9_]*\}/gu) ?? [])].sort() : [];
    for (const resource of [trTR, ptBR]) {
      const mismatchedKeys = leafEntries(resource)
        .filter(
          ([key, value]) => JSON.stringify(argumentsIn(value)) !== JSON.stringify(argumentsIn(englishEntries.get(key)))
        )
        .map(([key]) => key);
      expect(mismatchedKeys).toEqual([]);
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
      [LANGUAGE_IDS.ruRU]: 'Открыть',
      [LANGUAGE_IDS.zhCN]: '打开',
      [LANGUAGE_IDS.zhTW]: '開啟',
      [LANGUAGE_IDS.trTR]: 'Aç',
      [LANGUAGE_IDS.ptBR]: 'Abrir',
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
    expect(LanguageService.resolveSupportedLanguage(['ru-RU', 'en-US'])).toBe(LANGUAGE_IDS.ruRU);
    expect(LanguageService.resolveSupportedLanguage(['ru'])).toBe(LANGUAGE_IDS.ruRU);
    expect(LanguageService.resolveSupportedLanguage(['tr-TR', 'en-US'])).toBe(LANGUAGE_IDS.trTR);
    expect(LanguageService.resolveSupportedLanguage([' TR '])).toBe(LANGUAGE_IDS.trTR);
    expect(LanguageService.resolveSupportedLanguage(['pt-BR'])).toBe(LANGUAGE_IDS.ptBR);
    expect(LanguageService.resolveSupportedLanguage(['pt-PT'])).toBe(LANGUAGE_IDS.ptBR);
    expect(LanguageService.resolveSupportedLanguage(['pt'])).toBe(LANGUAGE_IDS.ptBR);
    expect(LanguageService.resolveSupportedLanguage(['tricky', 'ptolemy'])).toBe(LANGUAGE_IDS.enUS);
  });

  it('detects and renders Russian with native plural forms', () => {
    expect(LanguageService.resolveSupportedLanguage(['ru-RU', 'en-US'])).toBe('ru-RU');
    expect(i18n.global.availableLocales).toContain('ru-RU');

    i18n.global.locale.value = 'ru-RU' as typeof i18n.global.locale.value;
    expect(i18n.global.t('common.open')).toBe('Открыть');
    expect(i18n.global.t('common.fileCount', { count: 0 }, 0)).toBe('0 файлов');
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe('1 файл');
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 файла');
    expect(i18n.global.t('common.fileCount', { count: 5 }, 5)).toBe('5 файлов');
    expect(i18n.global.t('common.fileCount', { count: 11 }, 11)).toBe('11 файлов');
    expect(i18n.global.t('common.fileCount', { count: 21 }, 21)).toBe('21 файл');
    expect(i18n.global.t('common.fileCount', { count: 22 }, 22)).toBe('22 файла');
    expect(i18n.global.t('common.fileCount', { count: 25 }, 25)).toBe('25 файлов');
    expect(i18n.global.t('cleanup.applicationComponentCount', { count: 0 }, 0)).toBe('Нет элементов для оптимизации');
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

    i18n.global.locale.value = LANGUAGE_IDS.ruRU;
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe('1 файл');
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 файла');
    expect(i18n.global.t('common.fileCount', { count: 5 }, 5)).toBe('5 файлов');

    i18n.global.locale.value = LANGUAGE_IDS.trTR;
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe('1 dosya');
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 dosya');

    i18n.global.locale.value = LANGUAGE_IDS.ptBR;
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe('1 arquivo');
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 arquivos');
    expect(i18n.global.t('settings.scanExclusionsCount', { count: 1 })).toBe('1 regra');
    expect(i18n.global.t('settings.scanExclusionsCount', { count: 2 })).toBe('2 regras');
    expect(i18n.global.t('startup.summary.programs', { count: 1 })).toBe('1 app inicia automaticamente');
    expect(i18n.global.t('startup.summary.programs', { count: 2 })).toBe('2 apps iniciam automaticamente');
  });
});
