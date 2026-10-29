//! Shared HTTP identity for Core requests and desktop integrations.
//! Callers retain ownership of timeouts, redirects, retries and authentication.

use std::sync::LazyLock;

use mangodisk_platform::system_identity::{self, SystemIdentity};
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};

// System identity is stable during a process lifetime. Cache it so constructing
// an HTTP client never repeats OS queries after the first request.
static APPLICATION_USER_AGENT: LazyLock<HeaderValue> = LazyLock::new(|| {
    let identity = system_identity::current();
    if identity.version.is_none() {
        log::warn!("http_user_agent_system_version_unavailable outcome=unknown");
    }
    HeaderValue::from_str(&format_user_agent(&identity))
        .expect("the formatted user agent contains only printable ASCII")
});

fn format_user_agent(identity: &SystemIdentity) -> String {
    let architecture = match identity.architecture.as_str() {
        "aarch64" | "arm64" => "arm64",
        "amd64" | "x86_64" => "x86_64",
        other => other,
    };
    format!(
        "MangoDisk/{} ({} {}; {})",
        env!("CARGO_PKG_VERSION"),
        comment_field(identity.operating_system),
        comment_field(identity.version.as_deref().unwrap_or("unknown")),
        comment_field(architecture),
    )
}

fn comment_field(value: &str) -> String {
    // Preserve native version labels such as Windows "11 (26100)" while
    // keeping each field inside one bounded HTTP comment.
    let value: String = value
        .chars()
        .take(96)
        .map(|character| match character {
            '(' => '[',
            ')' => ']',
            '\\' | ';' => '_',
            character if character.is_ascii_graphic() || character == ' ' => character,
            _ => ' ',
        })
        .collect();
    let value = value.trim();
    if value.is_empty() {
        "unknown".into()
    } else {
        value.into()
    }
}

/// Third-party clients, including the updater, use the same application headers.
pub fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, APPLICATION_USER_AGENT.clone());
    headers
}

/// Request-specific headers override these defaults without affecting other clients.
pub fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().default_headers(default_headers())
}

/// Optional first-party metadata, added only by callers of MangoDisk's own APIs.
/// This never changes the defaults used for third-party HTTP requests.
#[derive(Clone, Default)]
pub struct OfficialRequestMetadata {
    pub locale: String,
    pub distribution: String,
    pub os_version: Option<String>,
    pub system_locale: Option<String>,
    pub native_arch: Option<String>,
    pub install_id: Option<String>,
    pub request_id: Option<String>,
}
impl OfficialRequestMetadata {
    pub fn headers(&self) -> HeaderMap {
        let mut headers = default_headers();
        headers.insert(
            "x-mangodisk-app-version",
            HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
        );
        if let Some(locale) = valid_locale(&self.locale) {
            insert_metadata(&mut headers, "accept-language", locale, 32);
            insert_metadata(&mut headers, "x-mangodisk-locale", locale, 32);
        }
        if matches!(self.distribution.as_str(), "installed" | "portable") {
            insert_metadata(
                &mut headers,
                "x-mangodisk-distribution",
                &self.distribution,
                16,
            );
        }
        if let Some(value) = &self.os_version {
            insert_metadata(&mut headers, "x-mangodisk-os-version", value, 128);
        }
        if let Some(value) = self.system_locale.as_deref().and_then(valid_locale) {
            insert_metadata(&mut headers, "x-mangodisk-system-locale", value, 32);
        }
        if let Some(value @ ("aarch64" | "x86_64")) = self.native_arch.as_deref() {
            insert_metadata(&mut headers, "x-mangodisk-native-arch", value, 16);
        }
        for (name, value) in [
            ("x-mangodisk-install-id", &self.install_id),
            ("x-request-id", &self.request_id),
        ] {
            if let Some(id) = value
                .as_deref()
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
            {
                let mut value =
                    HeaderValue::from_str(&id.to_string()).expect("UUID is a valid HTTP value");
                value.set_sensitive(true);
                headers.insert(name, value);
            }
        }
        headers
    }
}
fn valid_locale(locale: &str) -> Option<&str> {
    let locale = locale.trim();
    (locale.len() >= 2
        && locale.len() <= 32
        && !locale.starts_with('-')
        && !locale.ends_with('-')
        && locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then_some(locale)
}
fn insert_metadata(headers: &mut HeaderMap, name: &'static str, value: &str, limit: usize) {
    let value = value.trim();
    if !value.is_empty()
        && value.len() <= limit
        && value.bytes().all(|byte| (32..=126).contains(&byte))
    {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_headers_are_consistent_optional_and_do_not_change_generic_clients() {
        let id = uuid::Uuid::new_v4().to_string();
        let metadata = OfficialRequestMetadata {
            locale: "zh-CN".into(),
            distribution: "installed".into(),
            os_version: Some("15.7".into()),
            system_locale: Some("it-IT".into()),
            native_arch: Some("aarch64".into()),
            install_id: Some(id.to_uppercase()),
            request_id: Some(id.clone()),
        };
        let headers = metadata.headers();
        assert_eq!(headers["user-agent"], default_headers()["user-agent"]);
        assert_eq!(headers["accept-language"], "zh-CN");
        assert_eq!(headers["x-mangodisk-locale"], "zh-CN");
        assert_eq!(
            headers["x-mangodisk-app-version"],
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(headers["x-mangodisk-install-id"], id);
        assert!(headers["x-mangodisk-install-id"].is_sensitive());
        assert_eq!(headers["x-mangodisk-system-locale"], "it-IT");
        assert_eq!(headers["x-mangodisk-native-arch"], "aarch64");
        assert_eq!(default_headers().len(), 1);
        let omitted = OfficialRequestMetadata {
            locale: "bad\r\nInjected: yes".into(),
            distribution: "unknown".into(),
            os_version: Some("bad\nvalue".into()),
            system_locale: Some("a".repeat(33)),
            native_arch: Some("invalid".into()),
            install_id: Some("invalid".into()),
            request_id: None,
        }
        .headers();
        assert_eq!(omitted.len(), 2);
        for locale in ["pt-BR", "it-IT", "zh-Hant-TW"] {
            assert_eq!(valid_locale(locale), Some(locale));
        }
        for locale in ["", "-en", "en-", "en-US\r\nother: value"] {
            assert!(valid_locale(locale).is_none());
        }
    }

    #[test]
    fn user_agent_formats_system_versions_and_architectures() {
        for (operating_system, version, architecture, expected) in [
            ("macOS", "15.7.7", "aarch64", "macOS 15.7.7; arm64"),
            (
                "Windows",
                "11 (26100)",
                "x86_64",
                "Windows 11 [26100]; x86_64",
            ),
            ("Linux", "24.04", "aarch64", "Linux 24.04; arm64"),
        ] {
            let identity = SystemIdentity {
                operating_system,
                version: Some(version.into()),
                architecture: architecture.into(),
            };
            assert_eq!(
                format_user_agent(&identity),
                format!("MangoDisk/{} ({expected})", env!("CARGO_PKG_VERSION")),
            );
        }
    }

    #[test]
    fn missing_or_unusual_system_versions_cannot_break_requests() {
        let mut identity = SystemIdentity {
            operating_system: "Linux",
            version: None,
            architecture: "x86_64".into(),
        };
        assert!(format_user_agent(&identity).contains("Linux unknown; x86_64"));
        identity.version = Some("release\r\nInjected: value; (\\)\u{65e5}".repeat(100));
        let value = format_user_agent(&identity);
        assert!(HeaderValue::from_str(&value).is_ok());
        assert!(value.len() < 160);
        assert_eq!(value.matches('(').count(), 1);
        assert_eq!(value.matches(')').count(), 1);
        assert_eq!(value.matches(';').count(), 1);
        assert!(!value.contains('\\'));
    }
}
