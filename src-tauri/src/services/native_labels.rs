//! Native surfaces reuse the same locale resources as Vue.
use serde_json::Value;
use std::sync::OnceLock;
use tauri_plugin_store::StoreExt;

pub struct NativeLabels {
    pub locale: String,
    messages: &'static Value,
}
impl NativeLabels {
    pub fn load(app: &tauri::AppHandle) -> Self {
        let locale = app
            .store_builder("settings.json")
            .disable_auto_save()
            .build()
            .ok()
            .and_then(|store| store.get("settings"))
            .and_then(|value| {
                value
                    .get("language")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(tauri_plugin_os::locale)
            .unwrap_or_else(|| "en-US".into());
        Self::for_locale(&locale)
    }
    pub fn for_locale(locale: &str) -> Self {
        let locale = supported_locale(locale);
        static MESSAGES: OnceLock<[Value; 12]> = OnceLock::new();
        let values = MESSAGES.get_or_init(|| {
            [
                include_str!("../../../src/locales/en-US.json"),
                include_str!("../../../src/locales/zh-CN.json"),
                include_str!("../../../src/locales/zh-TW.json"),
                include_str!("../../../src/locales/ja-JP.json"),
                include_str!("../../../src/locales/ko-KR.json"),
                include_str!("../../../src/locales/tr-TR.json"),
                include_str!("../../../src/locales/pt-BR.json"),
                include_str!("../../../src/locales/ru-RU.json"),
                include_str!("../../../src/locales/fr-FR.json"),
                include_str!("../../../src/locales/de-DE.json"),
                include_str!("../../../src/locales/es-ES.json"),
                include_str!("../../../src/locales/uk-UA.json"),
            ]
            .map(|text| serde_json::from_str(text).expect("validated locale resource"))
        });
        let index = match locale {
            "zh-CN" => 1,
            "zh-TW" => 2,
            "ja-JP" => 3,
            "ko-KR" => 4,
            "tr-TR" => 5,
            "pt-BR" => 6,
            "ru-RU" => 7,
            "fr-FR" => 8,
            "de-DE" => 9,
            "es-ES" => 10,
            "uk-UA" => 11,
            _ => 0,
        };
        Self {
            locale: locale.into(),
            messages: &values[index],
        }
    }
    pub fn text(&self, key: &str) -> &str {
        self.messages
            .pointer(key)
            .and_then(Value::as_str)
            .unwrap_or("—")
    }
}

/// Match the frontend language-prefix policy for installations that have not
/// persisted a language yet. Traditional Chinese must precede generic Chinese.
fn supported_locale(locale: &str) -> &'static str {
    let locale = locale.trim().to_ascii_lowercase();
    for (prefix, supported) in [
        ("zh-tw", "zh-TW"),
        ("zh-hk", "zh-TW"),
        ("zh-mo", "zh-TW"),
        ("zh-hant", "zh-TW"),
        ("zh", "zh-CN"),
        ("ja", "ja-JP"),
        ("ko", "ko-KR"),
        ("tr", "tr-TR"),
        ("pt", "pt-BR"),
        ("ru", "ru-RU"),
        ("uk", "uk-UA"),
        ("fr", "fr-FR"),
        ("de", "de-DE"),
        ("es", "es-ES"),
        ("en", "en-US"),
    ] {
        if locale == prefix || locale.starts_with(&format!("{prefix}-")) {
            return supported;
        }
    }
    "en-US"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_prompts_are_complete_in_every_supported_locale() {
        for locale in [
            "en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "tr-TR", "pt-BR", "ru-RU", "uk-UA",
            "fr-FR", "de-DE", "es-ES",
        ] {
            let labels = NativeLabels::for_locale(locale);
            for key in [
                "updateRequired",
                "update",
                "exit",
                "openFailed",
                "recoveryFailed",
            ] {
                let value = labels.text(&format!("/webviewRuntime/{key}"));
                assert!(!value.is_empty() && value != "—", "{locale}: {key}");
            }
            assert!(labels.text("/webviewRuntime/openFailed").contains("{url}"));
        }
    }

    #[test]
    fn new_locales_render_native_menu_actions_without_fallback() {
        for (locale, open, settings, quit) in [
            ("fr-FR", "Ouvrir MangoDisk", "Réglages", "Quitter"),
            ("de-DE", "MangoDisk öffnen", "Einstellungen", "Beenden"),
            ("es-ES", "Abrir MangoDisk", "Ajustes", "Salir"),
            ("uk-UA", "Відкрити MangoDisk", "Параметри", "Вийти"),
        ] {
            let labels = NativeLabels::for_locale(locale);
            assert_eq!(labels.text("/systemStatus/openMain"), open);
            assert_eq!(labels.text("/systemStatus/settings"), settings);
            assert_eq!(labels.text("/systemStatus/quit"), quit);
        }
    }

    #[test]
    fn native_defaults_match_supported_browser_language_prefixes() {
        for (input, expected) in [
            ("zh-CN", "zh-CN"),
            ("zh-Hant-HK", "zh-TW"),
            ("zh-SG", "zh-CN"),
            ("ja", "ja-JP"),
            ("ko-KR", "ko-KR"),
            ("ru-RU", "ru-RU"),
            ("ru", "ru-RU"),
            ("uk-UA", "uk-UA"),
            ("uk", "uk-UA"),
            (" UK-ua ", "uk-UA"),
            ("ukrainian", "en-US"),
            ("fr", "fr-FR"),
            ("de", "de-DE"),
            ("es", "es-ES"),
            ("en-GB", "en-US"),
            ("tr", "tr-TR"),
            (" TR-tr ", "tr-TR"),
            ("pt-BR", "pt-BR"),
            ("pt-PT", "pt-BR"),
            ("pt", "pt-BR"),
            ("tricky", "en-US"),
            ("fr-FR", "fr-FR"),
            (" FR-ca ", "fr-FR"),
            ("de-DE", "de-DE"),
            ("de-AT", "de-DE"),
            ("es-ES", "es-ES"),
            ("es-MX", "es-ES"),
            ("french", "en-US"),
            ("desktop", "en-US"),
            ("estonian", "en-US"),
            ("it-IT", "en-US"),
        ] {
            assert_eq!(NativeLabels::for_locale(input).locale, expected);
        }
    }
}
