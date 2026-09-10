import { createI18n } from 'vue-i18n';

import { LANGUAGE_IDS, type LanguageId } from '@/lib/models/settings';
import enUS from '@/locales/modules/en-us';
import jaJP from '@/locales/modules/ja-jp';
import koKR from '@/locales/modules/ko-kr';
import zhCN from '@/locales/modules/zh-cn';
import zhTW from '@/locales/modules/zh-tw';

export type MessageSchema = typeof zhCN;
export type SupportedLocale = LanguageId;

/**
 * All locale resources are imported into one message graph so every view can switch languages offline.
 * Their bounded size does not justify asynchronous loading, extra failure states, or switch latency.
 */
export const i18n = createI18n<[MessageSchema], SupportedLocale, false>({
  legacy: false,
  globalInjection: false,
  locale: LANGUAGE_IDS.enUS,
  fallbackLocale: LANGUAGE_IDS.enUS,
  messages: {
    [LANGUAGE_IDS.zhCN]: zhCN,
    [LANGUAGE_IDS.zhTW]: zhTW,
    [LANGUAGE_IDS.jaJP]: jaJP,
    [LANGUAGE_IDS.koKR]: koKR,
    [LANGUAGE_IDS.enUS]: enUS,
  },
});
