//! Shared running-application identity, independent of ranking and icon presentation.
use std::path::{Path, PathBuf};

pub(crate) fn application_path(executable: &Path) -> PathBuf {
    executable
        .ancestors()
        .filter(|parent| parent.extension().is_some_and(|ext| ext == "app"))
        .last()
        .unwrap_or(executable)
        .to_path_buf()
}

pub(crate) fn id(path: Option<&Path>, pid: u32) -> String {
    let identity = path
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("process:{pid}"));
    blake3::hash(identity.as_bytes()).to_hex().to_string()
}

pub(crate) fn can_quit(path: &Path, own_path: Option<&Path>) -> bool {
    own_path != Some(path)
        && (is_bundle(path)
            || cfg!(windows)
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")))
}

pub(crate) fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "app")
}

/// Force actions fail closed when the sampler cannot identify this application.
pub(crate) fn can_force_quit(path: &Path, own_path: Option<&Path>) -> bool {
    own_path.is_some()
        && can_quit(path, own_path)
        && mangodisk_platform::application_force_quit::allowed(path)
}
