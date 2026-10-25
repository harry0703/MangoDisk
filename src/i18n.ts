import { createI18n } from 'vue-i18n';

import { LANGUAGE_IDS, type LanguageId } from '@/lib/models/settings/settings';
import deDE from '@/locales/modules/de-de';
import enUS from '@/locales/modules/en-us';
import esES from '@/locales/modules/es-es';
import frFR from '@/locales/modules/fr-fr';
import jaJP from '@/locales/modules/ja-jp';
import koKR from '@/locales/modules/ko-kr';
import ptBR from '@/locales/modules/pt-br';
import ruRU from '@/locales/modules/ru-ru';
import trTR from '@/locales/modules/tr-tr';
import ukUA from '@/locales/modules/uk-ua';
import zhCN from '@/locales/modules/zh-cn';
import zhTW from '@/locales/modules/zh-tw';

export type MessageSchema = typeof zhCN;
export type SupportedLocale = LanguageId;

// French uses singular for zero and one, while three-form messages keep an explicit zero state.
function frenchPluralRule(choice: number, choicesLength: number): number {
  const normalizedChoice = Math.abs(choice);
  if (choicesLength === 2) return normalizedChoice < 2 ? 0 : 1;
  if (choicesLength === 3) return normalizedChoice === 0 ? 0 : normalizedChoice < 2 ? 1 : 2;
  return 0;
}

// Russian and Ukrainian share one/few/many forms; four-form messages keep an explicit zero state.
function eastSlavicPluralRule(choice: number, choicesLength: number): number {
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
    [LANGUAGE_IDS.frFR]: frenchPluralRule,
    [LANGUAGE_IDS.ruRU]: eastSlavicPluralRule,
    [LANGUAGE_IDS.ukUA]: eastSlavicPluralRule,
  },
  messages: {
    [LANGUAGE_IDS.zhCN]: zhCN,
    [LANGUAGE_IDS.zhTW]: zhTW,
    [LANGUAGE_IDS.jaJP]: jaJP,
    [LANGUAGE_IDS.koKR]: koKR,
    [LANGUAGE_IDS.ruRU]: ruRU,
    [LANGUAGE_IDS.ukUA]: ukUA,
    [LANGUAGE_IDS.enUS]: enUS,
    [LANGUAGE_IDS.trTR]: trTR,
    [LANGUAGE_IDS.ptBR]: ptBR,
    [LANGUAGE_IDS.frFR]: frFR,
    [LANGUAGE_IDS.deDE]: deDE,
    [LANGUAGE_IDS.esES]: esES,
  },
});

// Locale modules can update while Vue retains the active composer. Replace
// its messages too, so new keys do not appear as raw labels during development.
if (import.meta.hot) {
  import.meta.hot.accept(
    [
      './locales/modules/de-de',
      './locales/modules/en-us',
      './locales/modules/es-es',
      './locales/modules/fr-fr',
      './locales/modules/ja-jp',
      './locales/modules/ko-kr',
      './locales/modules/pt-br',
      './locales/modules/ru-ru',
      './locales/modules/tr-tr',
      './locales/modules/uk-ua',
      './locales/modules/zh-cn',
      './locales/modules/zh-tw',
    ],
    updatedModules => {
      const locales = [
        LANGUAGE_IDS.deDE,
        LANGUAGE_IDS.enUS,
        LANGUAGE_IDS.esES,
        LANGUAGE_IDS.frFR,
        LANGUAGE_IDS.jaJP,
        LANGUAGE_IDS.koKR,
        LANGUAGE_IDS.ptBR,
        LANGUAGE_IDS.ruRU,
        LANGUAGE_IDS.trTR,
        LANGUAGE_IDS.ukUA,
        LANGUAGE_IDS.zhCN,
        LANGUAGE_IDS.zhTW,
      ];
      updatedModules.forEach((updatedModule, index) => {
        const locale = locales[index];
        const messages = updatedModule?.default as MessageSchema | undefined;
        if (locale && messages) i18n.global.setLocaleMessage(locale, messages);
      });
    }
  );
}
