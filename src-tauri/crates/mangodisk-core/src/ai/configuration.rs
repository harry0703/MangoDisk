use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::{configuration_file, AiError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiServiceMode {
    Free,
    // Missing fields belong to schema 1 custom configurations, never opt users into a new recipient.
    #[default]
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReasoningMode {
    #[default]
    Default,
    Disabled,
}

// Never derive Debug: this structure contains the user's API key.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiConfiguration {
    pub schema_version: u8,
    #[serde(default)]
    pub mode: AiServiceMode,
    #[serde(default)]
    pub free_consent: bool,
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub reasoning: ReasoningMode,
    /// Missing optional controls retain provider defaults in schema 1/2 files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiConfigurationUpdate {
    pub mode: AiServiceMode,
    pub free_consent: bool,
    pub endpoint: String,
    pub model: String,
    /// None retains the stored key only when the destination is unchanged.
    pub api_key: Option<String>,
    pub reasoning: ReasoningMode,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub schema_version: u8,
    pub mode: AiServiceMode,
    pub free_consent: bool,
    pub free_available: bool,
    pub endpoint: String,
    pub model: String,
    pub has_key: bool,
    pub reasoning: ReasoningMode,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
}

impl AiConfiguration {
    pub fn initial() -> Self {
        Self {
            schema_version: 2,
            mode: AiServiceMode::Free,
            free_consent: false,
            endpoint: String::new(),
            model: String::new(),
            api_key: String::new(),
            reasoning: ReasoningMode::Default,
            temperature: None,
            max_tokens: None,
        }
    }

    pub fn load() -> Result<Option<Self>, AiError> {
        configuration_file::read()?
            .map(|raw| {
                let mut value: Self =
                    serde_json::from_str(&raw).map_err(|_| AiError::InvalidConfiguration)?;
                value.validate()?;
                value.schema_version = 2;
                Ok(value)
            })
            .transpose()
    }

    pub fn save(update: AiConfigurationUpdate) -> Result<AiSettings, AiError> {
        let endpoint = if update.mode == AiServiceMode::Free && update.endpoint.trim().is_empty() {
            String::new()
        } else {
            normalize_endpoint(&update.endpoint)?
        };
        let api_key = match update.api_key {
            Some(key) => key.trim().to_owned(),
            None => {
                let old = Self::load()?.ok_or(AiError::NotConfigured)?;
                // A changed server must never silently receive an existing secret.
                if endpoint != old.endpoint {
                    return Err(AiError::InvalidConfiguration);
                }
                old.api_key
            }
        };
        let value = Self {
            schema_version: 2,
            mode: update.mode,
            free_consent: update.free_consent,
            endpoint,
            model: update.model.trim().to_owned(),
            api_key,
            reasoning: update.reasoning,
            temperature: update.temperature,
            max_tokens: update.max_tokens,
        };
        value.validate()?;
        let raw =
            serde_json::to_string_pretty(&value).map_err(|_| AiError::InvalidConfiguration)?;
        // Replace the whole document atomically so a partial save cannot
        // associate an existing key with a newly edited endpoint.
        configuration_file::write(&raw)?;
        Ok(value.settings())
    }

    pub fn delete() -> Result<(), AiError> {
        configuration_file::delete()
    }

    pub fn settings(&self) -> AiSettings {
        AiSettings {
            schema_version: 2,
            mode: self.mode,
            free_consent: self.free_consent,
            free_available: super::official::available(),
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            has_key: !self.api_key.is_empty(),
            reasoning: self.reasoning,
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        }
    }

    pub fn validate(&self) -> Result<(), AiError> {
        if !matches!(self.schema_version, 1 | 2)
            || self
                .temperature
                .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
            || self.max_tokens == Some(0)
            || self.model.len() > 128
            || self.model.chars().any(char::is_control)
            || self.api_key.len() > 512
            || self
                .api_key
                .chars()
                .any(|c| c.is_control() || !c.is_ascii())
            || (!self.endpoint.is_empty() && normalize_endpoint(&self.endpoint)? != self.endpoint)
        {
            return Err(AiError::InvalidConfiguration);
        }
        if self.mode == AiServiceMode::Free {
            return Ok(());
        }
        if self.model.is_empty() || self.endpoint.is_empty() {
            return Err(AiError::InvalidConfiguration);
        }
        let url = Url::parse(&self.endpoint).map_err(|_| AiError::InvalidConfiguration)?;
        if self.api_key.is_empty() && !is_loopback(&url) {
            return Err(AiError::InvalidConfiguration);
        }
        Ok(())
    }
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

pub(super) fn normalize_endpoint(input: &str) -> Result<String, AiError> {
    let trimmed = input.trim().trim_end_matches('/');
    let url = Url::parse(trimmed).map_err(|_| AiError::InvalidConfiguration)?;
    if trimmed.len() > 512
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "http" | "https")
        || url.path().ends_with("/chat/completions")
    {
        return Err(AiError::InvalidConfiguration);
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advanced_controls_are_optional_and_validated() {
        let mut config = AiConfiguration::initial();
        for temperature in [-0.1, 2.1, f64::NAN, f64::INFINITY] {
            config.temperature = Some(temperature);
            assert!(config.validate().is_err());
        }
        for temperature in [0.0, 0.7, 2.0] {
            config.temperature = Some(temperature);
            assert!(config.validate().is_ok());
        }
        config.max_tokens = Some(0);
        assert!(config.validate().is_err());
        config.max_tokens = Some(4096);
        let roundtrip: AiConfiguration =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assert_eq!(roundtrip.temperature, Some(2.0));
        assert_eq!(roundtrip.max_tokens, Some(4096));
        assert!(roundtrip.validate().is_ok());
    }

    #[test]
    fn endpoint_accepts_http_and_https_but_rejects_embedded_credentials() {
        for input in [
            "https://secret@example.com/v1",
            "https://example.com/v1?key=x",
            "file:///tmp",
            "https://example.com/v1/chat/completions",
        ] {
            assert!(normalize_endpoint(input).is_err());
        }
        assert_eq!(
            normalize_endpoint(" https://example.com/v1/ ").unwrap(),
            "https://example.com/v1"
        );
        assert!(normalize_endpoint("http://127.0.0.1:1234/v1").is_ok());
        assert!(normalize_endpoint("http://example.com/v1").is_ok());
    }

    #[test]
    fn configuration_file_preserves_key_binding_and_rejects_corrupt_documents() {
        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = AiConfiguration::delete();
            }
        }
        let _cleanup = Cleanup;
        let update = |endpoint: &str, key: Option<&str>| AiConfigurationUpdate {
            mode: AiServiceMode::Custom,
            free_consent: false,
            endpoint: endpoint.into(),
            model: "fixture-model".into(),
            api_key: key.map(str::to_owned),
            reasoning: ReasoningMode::Default,
            temperature: None,
            max_tokens: None,
        };
        AiConfiguration::save(update("https://example.com/v1", Some("synthetic-key"))).unwrap();
        let settings = AiConfiguration::save(update("https://example.com/v1/", None)).unwrap();
        assert!(settings.has_key);
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("synthetic-key"));
        assert!(matches!(
            AiConfiguration::save(update("https://different.example/v1", None)),
            Err(AiError::InvalidConfiguration)
        ));
        let retained = AiConfiguration::load().unwrap().unwrap();
        assert_eq!(retained.endpoint, "https://example.com/v1");
        assert_eq!(retained.api_key, "synthetic-key");
        // Schema 1 must retain its recipient and never imply free-service consent.
        configuration_file::write(r#"{"schemaVersion":1,"endpoint":"https://example.com/v1","model":"fixture-model","apiKey":"synthetic-key","reasoning":"default"}"#).unwrap();
        let migrated = AiConfiguration::load().unwrap().unwrap();
        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.mode, AiServiceMode::Custom);
        assert!(!migrated.free_consent);
        assert_eq!(migrated.temperature, None);
        assert_eq!(migrated.max_tokens, None);
        let mut free = update("https://example.com/v1", None);
        free.mode = AiServiceMode::Free;
        free.free_consent = true;
        AiConfiguration::save(free).unwrap();
        let free = AiConfiguration::load().unwrap().unwrap();
        assert_eq!(free.mode, AiServiceMode::Free);
        assert_eq!(free.api_key, "synthetic-key");
        assert!(free.free_consent);
        AiConfiguration::save(update("https://example.com/v1", None)).unwrap();
        assert_eq!(
            AiConfiguration::load().unwrap().unwrap().mode,
            AiServiceMode::Custom
        );
        configuration_file::write("{broken").unwrap();
        assert!(matches!(
            AiConfiguration::load(),
            Err(AiError::InvalidConfiguration)
        ));
        assert_eq!(
            configuration_file::read().unwrap().as_deref(),
            Some("{broken")
        );
        AiConfiguration::save(update("https://example.com/v1", Some("replacement"))).unwrap();
        AiConfiguration::delete().unwrap();
        assert!(AiConfiguration::load().unwrap().is_none());
        assert_eq!(AiConfiguration::initial().mode, AiServiceMode::Free);
        assert!(!AiConfiguration::initial().free_consent);
    }
}
