import { afterEach, describe, expect, it, vi } from 'vitest';

import { i18n } from '@/i18n';
import { LANGUAGE_IDS, LANGUAGE_OPTIONS } from '@/lib/models/settings/settings';
import { LanguageService } from '@/lib/services/settings/language-service';
import deDE from '@/locales/de-DE.json';
import enUS from '@/locales/en-US.json';
import esES from '@/locales/es-ES.json';
import frFR from '@/locales/fr-FR.json';
import jaJP from '@/locales/ja-JP.json';
import koKR from '@/locales/ko-KR.json';
import ptBR from '@/locales/pt-BR.json';
import ruRU from '@/locales/ru-RU.json';
import trTR from '@/locales/tr-TR.json';
import ukUA from '@/locales/uk-UA.json';
import zhCN from '@/locales/zh-CN.json';
import zhTW from '@/locales/zh-TW.json';

const localeResources = [zhCN, zhTW, jaJP, koKR, ruRU, ukUA, enUS, trTR, ptBR, frFR, deDE, esES];

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
    for (const resource of [trTR, ptBR, ruRU, ukUA, frFR, deDE, esES]) {
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

  it('preserves product names in the new locale rule titles', () => {
    for (const resource of [frFR, deDE, esES]) {
      for (const [id, name] of [
        ['app.slack-cache', 'Slack'],
        ['app.chatgpt-cache', 'Codex'],
        ['dev.go-cache', 'Go'],
        ['dev.yarn-cache', 'Yarn'],
        ['project.rust-build-artifacts', 'Rust'],
        ['project.swift-build-artifacts', 'Swift'],
        ['project.react-native-build-artifacts', 'React Native'],
        ['project.pants-workdir', 'Pants'],
        ['special.ai-model-gpt4all', 'GPT4All'],
      ] as const) {
        expect(resource.cleanupRules.entries[id].name).toContain(name);
      }
    }
  });

  it('preserves literal filesystem names in translated examples and safety descriptions', () => {
    for (const resource of [frFR, deDE, esES]) {
      expect(resource.storageScanExclusions.namePlaceholder).toContain('node_modules');
      for (const [id, literal] of [
        ['project.composer-build-artifacts', 'composer.json'],
        ['app.npm-cache', 'node_modules'],
        ['app.yarn-cache', 'node_modules'],
      ] as const) {
        expect(resource.cleanupRules.entries[id].impact).toContain(literal);
      }
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
      [LANGUAGE_IDS.ukUA]: 'Відкрити',
      [LANGUAGE_IDS.zhCN]: '打开',
      [LANGUAGE_IDS.zhTW]: '開啟',
      [LANGUAGE_IDS.trTR]: 'Aç',
      [LANGUAGE_IDS.ptBR]: 'Abrir',
      [LANGUAGE_IDS.frFR]: 'Ouvrir',
      [LANGUAGE_IDS.deDE]: 'Öffnen',
      [LANGUAGE_IDS.esES]: 'Abrir',
    };

    for (const locale of i18n.global.availableLocales) {
      i18n.global.locale.value = locale;
      expect(i18n.global.t('common.open')).toBe(expectedLabels[locale]);
    }
  });

  it('matches supported system languages and falls back to English', () => {
    expect(LanguageService.resolveSupportedLanguage(['zh-Hans-CN', 'en-US'])).toBe(LANGUAGE_IDS.zhCN);
    expect(LanguageService.resolveSupportedLanguage(['it-IT', 'en-GB'])).toBe(LANGUAGE_IDS.enUS);
    expect(LanguageService.resolveSupportedLanguage(['zh-Hant-HK', 'en-US'])).toBe(LANGUAGE_IDS.zhTW);
    expect(LanguageService.resolveSupportedLanguage(['ja-JP'])).toBe(LANGUAGE_IDS.jaJP);
    expect(LanguageService.resolveSupportedLanguage(['ko-KR', 'en-US'])).toBe(LANGUAGE_IDS.koKR);
    expect(LanguageService.resolveSupportedLanguage(['ko'])).toBe(LANGUAGE_IDS.koKR);
    expect(LanguageService.resolveSupportedLanguage(['ru-RU', 'en-US'])).toBe(LANGUAGE_IDS.ruRU);
    expect(LanguageService.resolveSupportedLanguage(['ru'])).toBe(LANGUAGE_IDS.ruRU);
    expect(LanguageService.resolveSupportedLanguage(['uk-UA', 'en-US'])).toBe(LANGUAGE_IDS.ukUA);
    expect(LanguageService.resolveSupportedLanguage(['uk'])).toBe(LANGUAGE_IDS.ukUA);
    expect(LanguageService.resolveSupportedLanguage(['tr-TR', 'en-US'])).toBe(LANGUAGE_IDS.trTR);
    expect(LanguageService.resolveSupportedLanguage([' TR '])).toBe(LANGUAGE_IDS.trTR);
    expect(LanguageService.resolveSupportedLanguage(['pt-BR'])).toBe(LANGUAGE_IDS.ptBR);
    expect(LanguageService.resolveSupportedLanguage(['pt-PT'])).toBe(LANGUAGE_IDS.ptBR);
    expect(LanguageService.resolveSupportedLanguage(['pt'])).toBe(LANGUAGE_IDS.ptBR);
    expect(LanguageService.resolveSupportedLanguage(['tricky', 'ptolemy', 'french', 'desktop', 'estonian'])).toBe(
      LANGUAGE_IDS.enUS
    );
    expect(LanguageService.resolveSupportedLanguage(['zh-Hant-TW'])).toBe(LANGUAGE_IDS.zhTW);
  });

  it.each([
    ['fr-FR', LANGUAGE_IDS.frFR],
    [' FR-ca ', LANGUAGE_IDS.frFR],
    ['fr', LANGUAGE_IDS.frFR],
    ['de-DE', LANGUAGE_IDS.deDE],
    ['de-AT', LANGUAGE_IDS.deDE],
    ['de', LANGUAGE_IDS.deDE],
    ['es-ES', LANGUAGE_IDS.esES],
    ['es-MX', LANGUAGE_IDS.esES],
    ['es', LANGUAGE_IDS.esES],
  ])('detects %s ahead of a secondary English preference', (language, expected) => {
    expect(LanguageService.resolveSupportedLanguage([language, 'en-US'])).toBe(expected);
  });

  it.each([
    [LANGUAGE_IDS.frFR, 'Ouvrir', '0 fichier', '1 fichier', '2 fichiers'],
    [LANGUAGE_IDS.deDE, 'Öffnen', '0 Dateien', '1 Datei', '2 Dateien'],
    [LANGUAGE_IDS.esES, 'Abrir', '0 archivos', '1 archivo', '2 archivos'],
  ] as const)('switches to %s and renders counts without fallback', (locale, open, zero, one, two) => {
    const documentStub = { documentElement: { lang: '' } };
    vi.stubGlobal('document', documentStub);
    LanguageService.apply(locale);
    expect(documentStub.documentElement.lang).toBe(locale);
    expect(i18n.global.t('common.open')).toBe(open);
    expect(i18n.global.t('common.fileCount', { count: 0 }, 0)).toBe(zero);
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe(one);
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe(two);
  });

  it.each([
    [0, 'Aucun élément à optimiser'],
    [1, '1 élément à optimiser'],
    [2, '2 éléments à optimiser'],
    [21, '21 éléments à optimiser'],
  ])('preserves the explicit French zero state for %s items', (count, expected) => {
    i18n.global.locale.value = LANGUAGE_IDS.frFR;
    expect(i18n.global.t('cleanup.applicationComponentCount', { count }, count)).toBe(expected);
  });

  it('keeps the actual French history count at zero and one', () => {
    i18n.global.locale.value = LANGUAGE_IDS.frFR;
    expect(i18n.global.t('history.clearConfirmCount', { count: 0 }, 0)).toBe('0 entrée');
    expect(i18n.global.t('history.clearConfirmCount', { count: 1 }, 1)).toBe('1 entrée');
    expect(i18n.global.t('history.clearConfirmCount', { count: 2 }, 2)).toBe('2 entrées');
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

  it.each([1, 21, 31, 101, 1001])('shows the actual Russian history clear count for %s records', count => {
    i18n.global.locale.value = LANGUAGE_IDS.ruRU;
    expect(i18n.global.t('history.clearConfirmCount', { count }, count)).toBe(`${count} запись`);
  });

  it.each([
    [0, 'Нет элементов для оптимизации'],
    [1, 'Доступен 1 элемент'],
    [2, 'Доступно 2 элемента'],
    [5, 'Доступно 5 элементов'],
    [11, 'Доступно 11 элементов'],
    [21, 'Доступен 21 элемент'],
    [22, 'Доступно 22 элемента'],
    [25, 'Доступно 25 элементов'],
  ])('preserves the Russian explicit zero and plural forms for %s items', (count, expected) => {
    i18n.global.locale.value = LANGUAGE_IDS.ruRU;
    expect(i18n.global.t('cleanup.applicationComponentCount', { count }, count)).toBe(expected);
  });

  it('detects and renders Ukrainian with native plural forms', () => {
    expect(LanguageService.resolveSupportedLanguage(['uk-UA', 'en-US'])).toBe(LANGUAGE_IDS.ukUA);
    expect(i18n.global.availableLocales).toContain(LANGUAGE_IDS.ukUA);

    i18n.global.locale.value = LANGUAGE_IDS.ukUA;
    expect(i18n.global.t('common.open')).toBe('Відкрити');
    expect(i18n.global.t('common.fileCount', { count: 0 }, 0)).toBe('0 файлів');
    expect(i18n.global.t('common.fileCount', { count: 1 }, 1)).toBe('1 файл');
    expect(i18n.global.t('common.fileCount', { count: 2 }, 2)).toBe('2 файли');
    expect(i18n.global.t('common.fileCount', { count: 5 }, 5)).toBe('5 файлів');
    expect(i18n.global.t('common.fileCount', { count: 11 }, 11)).toBe('11 файлів');
    expect(i18n.global.t('common.fileCount', { count: 21 }, 21)).toBe('21 файл');
    expect(i18n.global.t('common.fileCount', { count: 22 }, 22)).toBe('22 файли');
    expect(i18n.global.t('common.fileCount', { count: 25 }, 25)).toBe('25 файлів');
  });

  it.each([1, 21, 31, 101, 1001])('shows the actual Ukrainian history clear count for %s records', count => {
    i18n.global.locale.value = LANGUAGE_IDS.ukUA;
    expect(i18n.global.t('history.clearConfirmCount', { count }, count)).toBe(`${count} запис`);
  });

  it.each([
    [0, 'Немає елементів для оптимізації'],
    [1, 'Для оптимізації доступний 1 елемент'],
    [2, 'Для оптимізації доступні 2 елементи'],
    [5, 'Для оптимізації доступно 5 елементів'],
    [11, 'Для оптимізації доступно 11 елементів'],
    [21, 'Для оптимізації доступний 21 елемент'],
    [22, 'Для оптимізації доступні 22 елементи'],
    [25, 'Для оптимізації доступно 25 елементів'],
  ])('preserves the Ukrainian explicit zero and plural forms for %s items', (count, expected) => {
    i18n.global.locale.value = LANGUAGE_IDS.ukUA;
    expect(i18n.global.t('cleanup.applicationComponentCount', { count }, count)).toBe(expected);
  });

  it.each([
    [1, '1 програма запускається автоматично'],
    [3, '3 програми запускаються автоматично'],
    [12, '12 програм запускаються автоматично'],
  ])('pluralizes Ukrainian counts that are single-form in English for %s', (count, expected) => {
    i18n.global.locale.value = LANGUAGE_IDS.ukUA;
    expect(i18n.global.t('startup.summary.programs', { count })).toBe(expected);
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
