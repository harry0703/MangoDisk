use mangodisk_platform::{
    AiModelDiscoveryPlatform, AiModelProvider as PlatformAiModelProvider, PlatformCancellation,
};
use serde::Serialize;

use crate::shared::CoreResult;

/// Stable provider tag for a locally installed AI model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalModelProvider {
    Ollama,
    OpenAiCompatible,
}

/// One locally installed AI model exposed to adapters.
///
/// This is presentation data only; it never authorizes cleanup, uninstall, or
/// removal of model files by itself. The owning cleanup rule resolves the exact
/// on-disk footprint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLocalModel {
    pub provider: LocalModelProvider,
    pub name: String,
    pub tag: Option<String>,
    pub installed_bytes: u64,
}

/// Enumerates locally installed AI models, such as Ollama repositories,
/// through the current platform adapter.
pub fn discover_local_models() -> CoreResult<Vec<InstalledLocalModel>> {
    let cancellation = PlatformCancellation::new(|| false);
    let platforms = mangodisk_platform::current_platform()
        .discover_ai_models(&cancellation)
        .map_err(crate::shared::CoreError::from)?;
    Ok(platforms
        .into_iter()
        .map(|model| InstalledLocalModel {
            provider: match model.provider {
                PlatformAiModelProvider::Ollama => LocalModelProvider::Ollama,
                PlatformAiModelProvider::OpenAiCompatible => LocalModelProvider::OpenAiCompatible,
            },
            name: model.name,
            tag: model.tag,
            installed_bytes: model.installed_bytes,
        })
        .collect())
}
