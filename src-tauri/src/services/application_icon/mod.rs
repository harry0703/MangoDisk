use std::path::PathBuf;

use serde::Serialize;

#[cfg(any(windows, target_os = "macos"))]
mod cache;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationIcon {
    path: String,
    data_url: String,
}

#[derive(Default)]
pub struct ApplicationIconLoadResult {
    pub icons: Vec<ApplicationIcon>,
    pub cache_hits: usize,
    pub decoded_icons: usize,
}

pub struct ApplicationIconService;

impl ApplicationIconService {
    pub const MAX_REQUESTS: usize = 32;

    /// Resolves icons only after application details are opened. Keeping large
    /// image data out of catalog results protects scan latency and keeps the
    /// primary IPC response compact. Native PNG data is cached separately so
    /// later processes do not need to decode unchanged application resources.
    pub fn load(paths: Vec<String>, cache_root: Option<PathBuf>) -> ApplicationIconLoadResult {
        let paths = paths.into_iter().take(Self::MAX_REQUESTS).collect();
        load_application_icons(paths, cache_root)
    }
}

#[cfg(target_os = "macos")]
fn load_application_icons(
    paths: Vec<String>,
    cache_root: Option<PathBuf>,
) -> ApplicationIconLoadResult {
    macos::load(paths, cache_root)
}

#[cfg(windows)]
fn load_application_icons(
    paths: Vec<String>,
    cache_root: Option<PathBuf>,
) -> ApplicationIconLoadResult {
    windows::load(paths, cache_root)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn load_application_icons(
    _paths: Vec<String>,
    _cache_root: Option<PathBuf>,
) -> ApplicationIconLoadResult {
    ApplicationIconLoadResult::default()
}

#[cfg(any(target_os = "macos", windows))]
impl ApplicationIcon {
    fn new(path: String, data_url: String) -> Self {
        Self { path, data_url }
    }
}
