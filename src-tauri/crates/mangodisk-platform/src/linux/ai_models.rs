use std::path::{Path, PathBuf};

use crate::{
    AiModelProvider, InstalledAiModel, PlatformCancellation, PlatformError, PlatformResult,
};

/// Charged model names that represent quantized builds of the same base model
/// are kept as-is; deduplication is the consumer's concern. This module only
/// reports stable provider facts about locally installed models.
/// Discovers installed Ollama models by reading the local model repository.
///
/// The repository layout is `<root>/manifests/<registry>/<namespace?>/name/tag`
/// where `tag` is a JSON manifest file. Total size is derived from the manifest
/// layer sizes; the exact blob footprint may differ after garbage collection.
pub(crate) fn discover_ollama_models(
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<InstalledAiModel>> {
    let Some(models_root) = ollama_models_root() else {
        return Ok(Vec::new());
    };
    discover_ollama_models_in(&models_root, cancellation)
}

fn discover_ollama_models_in(
    models_root: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<InstalledAiModel>> {
    let manifests_root = models_root.join("manifests");
    if !manifests_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut models = Vec::new();
    let walker = walkdir::WalkDir::new(&manifests_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            entry.file_name().to_string_lossy() != ".temp"
        });
    for entry in walker {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "Ollama model discovery was cancelled",
            ));
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!(
                    "ollama_manifest_walk_skipped manifest_root={} reason={}",
                    manifests_root.display(),
                    error
                );
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(manifest) = read_model_manifest(path) else {
            continue;
        };
        let Some(name) = path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let tag = path
            .file_name()
            .map(|tag| tag.to_string_lossy().into_owned())
            .unwrap_or_default();
        models.push(InstalledAiModel {
            provider: AiModelProvider::Ollama,
            name,
            tag: Some(tag),
            installed_bytes: manifest.installed_bytes,
        });
    }

    models.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(models)
}

fn ollama_models_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("OLLAMA_MODELS") {
        let root = PathBuf::from(root);
        if root.is_dir() {
            return Some(root);
        }
    }
    let home = dirs::home_dir()?;
    let default_root = home.join(".ollama").join("models");
    default_root.is_dir().then_some(default_root)
}

#[derive(Debug)]
struct OllamaModelManifest {
    installed_bytes: u64,
}

fn read_model_manifest(path: &Path) -> Option<OllamaModelManifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let mut installed_bytes = 0u64;
    if let Some(config) = value.get("config").and_then(|c| c.get("size")) {
        installed_bytes += config.as_u64().unwrap_or(0);
    }
    if let Some(layers) = value.get("layers").and_then(|l| l.as_array()) {
        for layer in layers {
            if let Some(size) = layer.get("size").and_then(|s| s.as_u64()) {
                installed_bytes += size;
            }
        }
    }
    (installed_bytes > 0).then_some(OllamaModelManifest { installed_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mangodisk-ollama-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    fn write_manifest(root: &Path, registry: &str, model: &str, tag: &str, layers: u64) {
        let path = root.join("manifests").join(registry).join(model).join(tag);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.ollama.image.manifest",
            "config": { "mediaType": "application/vnd.ollama.image.model", "size": 1024 },
            "layers": [
                { "mediaType": "application/vnd.ollama.image.model", "size": layers, "digest": "sha256:0000" }
            ]
        });
        std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn discards_models_when_repository_is_absent() {
        let root = fixture_root("absent");
        let cancellation = crate::PlatformCancellation::new(|| false);
        let result = discover_ollama_models_in(&root, &cancellation).unwrap();
        assert!(result.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_walks_model_tags_and_sums_layers() {
        let root = fixture_root("discover");
        write_manifest(
            &root,
            "registry.ollama.ai/library",
            "llama3",
            "latest",
            2_048,
        );
        write_manifest(&root, "registry.ollama.ai/library", "llama3", "7b", 4_096);
        write_manifest(&root, "registry.ollama.ai/custom", "mymodel", "q4", 1_024);
        let cancellation = crate::PlatformCancellation::new(|| false);
        let models = discover_ollama_models_in(&root, &cancellation).unwrap();
        assert_eq!(models.len(), 3);
        let llama3_latest = models
            .iter()
            .find(|m| m.name == "llama3" && m.tag.as_deref() == Some("latest"))
            .unwrap();
        assert_eq!(llama3_latest.installed_bytes, 1024 + 2_048);
        assert_eq!(llama3_latest.provider, AiModelProvider::Ollama);
        let custom = models.iter().find(|m| m.name == "mymodel").unwrap();
        assert_eq!(custom.tag.as_deref(), Some("q4"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reads_manifest_layers_as_model_bytes() {
        let root = fixture_root("read");
        write_manifest(
            &root,
            "registry.ollama.ai/library",
            "llama3",
            "latest",
            2_048,
        );
        let manifest =
            read_model_manifest(&root.join("manifests/registry.ollama.ai/library/llama3/latest"))
                .unwrap();
        assert_eq!(manifest.installed_bytes, 1024 + 2_048);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ignores_corrupt_manifests() {
        let root = fixture_root("corrupt");
        let path = root.join("manifests/registry.ollama.ai/library/broken/latest");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(read_model_manifest(&path).is_none());
        std::fs::remove_dir_all(&root).ok();
    }
}
