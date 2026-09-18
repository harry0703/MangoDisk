use crate::{PlatformCancellation, PlatformResult};

/// Identifies the runtime or storage provider that owns an installed AI model.
///
/// This enum is a platform-fact source tag. It does not authorize or restrict
/// any cleanup, uninstall, or mutation action by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiModelProvider {
    Ollama,
    OpenAiCompatible,
}

/// A single installed AI model discovered by the platform inventory.
///
/// `name` is the provider-scoped model identifier (e.g. `"llama3"` for Ollama).
/// `tag` is optional when the provider does not distinguish tags from the name
/// (e.g. OpenAI model IDs). Size and modification metadata are informational;
/// the exact on-disk footprint is resolved separately by the owning cleaner.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAiModel {
    pub provider: AiModelProvider,
    pub name: String,
    pub tag: Option<String>,
    pub installed_bytes: u64,
}

/// Platform capability to enumerate installed AI models.
///
/// This is a separate trait rather than a method on `Platform` because only a
/// subset of platforms supports local AI model storage today. The default
/// implementation returns an empty list; platforms opt in with a concrete
/// override.
pub trait AiModelDiscoveryPlatform: Send + Sync {
    fn discover_ai_models(
        &self,
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<InstalledAiModel>> {
        Ok(Vec::new())
    }
}
