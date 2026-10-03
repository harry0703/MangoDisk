import { createI18n } from 'vue-i18n';

import { LANGUAGE_IDS, type LanguageId } from '@/lib/models/settings';
import enUS from '@/locales/modules/en-us';
import jaJP from '@/locales/modules/ja-jp';
import koKR from '@/locales/modules/ko-kr';
import ruRU from '@/locales/modules/ru-ru';
import ptBR from '@/locales/modules/pt-br';
import trTR from '@/locales/modules/tr-tr';
import zhCN from '@/locales/modules/zh-cn';
import zhTW from '@/locales/modules/zh-tw';

export type MessageSchema = typeof zhCN;
export type SupportedLocale = LanguageId;

function russianPluralRule(choice: number, choicesLength: number): number {
  const normalizedChoice = Math.abs(choice);
  const hasExplicitZeroForm = choicesLength === 4;
  if (hasExplicitZeroForm && normalizedChoice === 0) return 0;

  const formOffset = hasExplicitZeroForm ? 1 : 0;
  const lastDigit = normalizedChoice % 10;
  const lastTwoDigits = normalizedChoice % 100;

  if (lastDigit === 1 && lastTwoDigits !== 11) return formOffset;
  if (lastDigit >= 2 && lastDigit <= 4 && (lastTwoDigits < 12 || lastTwoDigits > 14)) {
    return formOffset + 1;
  }
  return formOffset + 2;
}

/**
 * All locale resources are imported into one message graph so every view can switch languages offline.
 * Their bounded size does not justify asynchronous loading, extra failure states, or switch latency.
 */
export const i18n = createI18n<[MessageSchema], SupportedLocale, false>({
  legacy: false,
  globalInjection: false,
  locale: LANGUAGE_IDS.enUS,
  fallbackLocale: LANGUAGE_IDS.enUS,
  pluralRules: {
    [LANGUAGE_IDS.ruRU]: russianPluralRule,
  },
  messages: {
    [LANGUAGE_IDS.zhCN]: zhCN,
    [LANGUAGE_IDS.zhTW]: zhTW,
    [LANGUAGE_IDS.jaJP]: jaJP,
    [LANGUAGE_IDS.koKR]: koKR,
    [LANGUAGE_IDS.ruRU]: ruRU,
    [LANGUAGE_IDS.enUS]: enUS,
    [LANGUAGE_IDS.trTR]: trTR,
    [LANGUAGE_IDS.ptBR]: ptBR,
  },
});
