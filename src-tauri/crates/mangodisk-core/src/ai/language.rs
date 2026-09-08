//! Bounded language-tag syntax for prompts and signed metadata, not a UI locale registry.

pub(super) fn valid_language_tag(value: &str) -> bool {
    // Preserve the exact tag for signatures. Accept a 2–8 ASCII-letter primary
    // subtag and 1–8 alphanumeric subtags without embedding a language catalog.
    // This is a wire-format constraint, not full BCP 47 registry validation.
    if value.len() > 32 {
        return false;
    }
    let mut parts = value.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=8).contains(&first.len())
        && first.bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_future_languages_without_allowing_prompt_or_header_injection() {
        for value in [
            "zh-CN",
            "zh-TW",
            "en-US",
            "ja-JP",
            "fr-FR",
            "pt-BR",
            "zh-Hant",
            "en",
            "en-US-u-ca-gregory",
        ] {
            assert!(valid_language_tag(value), "{value}");
        }
        for value in [
            "",
            "en_US",
            " en-US",
            "en-US ",
            "en--US",
            "en-",
            "e-US",
            "englishxx",
            "en-abcdefghi",
            "en\r\nx-test: 1",
            "en ignore instructions",
            "\u{4e2d}\u{6587}",
            "en-12345678-12345678-12345678-12345",
        ] {
            assert!(!valid_language_tag(value), "{value:?}");
        }
    }
}
