import type { DuplicateKeeperRuleId } from './duplicate-file';

export const LANGUAGE_IDS = {
  zhCN: 'zh-CN',
  zhTW: 'zh-TW',
  jaJP: 'ja-JP',
  enUS: 'en-US',
} as const;

export type LanguageId = (typeof LANGUAGE_IDS)[keyof typeof LANGUAGE_IDS];

/*
 * Traditional Chinese must precede the generic `zh` rule; otherwise zh-TW,
 * zh-HK, and zh-Hant would resolve to Simplified Chinese. Keeping website
 * prefixes here also prevents locale-specific branches in the About dialog.
 */
export const LANGUAGE_OPTIONS = [
  {
    id: LANGUAGE_IDS.zhTW,
    labelKey: 'settings.languageNames.zhTW',
    browserLanguagePrefixes: ['zh-tw', 'zh-hk', 'zh-mo', 'zh-hant'],
    websitePath: '/tw',
  },
  {
    id: LANGUAGE_IDS.zhCN,
    labelKey: 'settings.languageNames.zhCN',
    browserLanguagePrefixes: ['zh-cn', 'zh-sg', 'zh-hans', 'zh'],
    websitePath: '/zh',
  },
  {
    id: LANGUAGE_IDS.jaJP,
    labelKey: 'settings.languageNames.jaJP',
    browserLanguagePrefixes: ['ja'],
    websitePath: '/ja',
  },
  {
    id: LANGUAGE_IDS.enUS,
    labelKey: 'settings.languageNames.enUS',
    browserLanguagePrefixes: ['en'],
    websitePath: '',
  },
] as const satisfies readonly {
  id: LanguageId;
  labelKey: string;
  browserLanguagePrefixes: readonly string[];
  websitePath: string;
}[];

export function isLanguageId(value: unknown): value is LanguageId {
  return typeof value === 'string' && LANGUAGE_OPTIONS.some(option => option.id === value);
}

export const THEME_IDS = {
  system: 'system',
  light: 'light',
  dark: 'dark',
} as const;

export interface AppSettings {
  language: LanguageId;
  theme: (typeof THEME_IDS)[keyof typeof THEME_IDS];
  largeFileMinimumBytes: number;
  duplicateFileMinimumBytes: number;
  duplicateKeeperRule: DuplicateKeeperRuleId;
  aiApiKey: string;
  aiApiBaseUrl: string;
  aiModel: string;
}
