use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

mod cache;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use cache::FileIconCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeFileIconItemKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeFileIconMode {
    Automatic,
    Generic,
    Path,
}

#[derive(Debug, Clone)]
pub struct NativeFileIconRequest {
    pub path: String,
    pub kind: NativeFileIconItemKind,
    pub mode: NativeFileIconMode,
}

#[derive(Debug)]
pub struct NativeFileIconAssignment {
    pub path: String,
    pub kind: NativeFileIconItemKind,
    pub mode: NativeFileIconMode,
    pub icon_key: String,
}

#[derive(Debug)]
pub struct NativeFileIconAsset {
    pub icon_key: String,
    pub png: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct NativeFileIconLoadResult {
    pub assignments: Vec<NativeFileIconAssignment>,
    pub assets: Vec<NativeFileIconAsset>,
    pub unique_identities: usize,
    pub cache_hits: usize,
    pub system_lookups: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum IconQuery {
    Type {
        key: String,
        extension: Option<String>,
    },
    Path {
        key: String,
        path: PathBuf,
    },
}

impl IconQuery {
    pub fn key(&self) -> &str {
        match self {
            Self::Type { key, .. } | Self::Path { key, .. } => key,
        }
    }
}

pub struct NativeFileIconService;

impl NativeFileIconService {
    pub const MAX_REQUESTS: usize = 128;

    /// Resolves only presentation assets. Scan results remain image-free, and
    /// grouping by identity guarantees that repeated types such as PDF trigger
    /// one cache lookup and at most one operating-system icon query per batch.
    pub fn load(
        requests: Vec<NativeFileIconRequest>,
        cache_root: Option<PathBuf>,
    ) -> NativeFileIconLoadResult {
        let cache = FileIconCache::new(cache_root);
        let mut grouped = HashMap::<IconQuery, Vec<NativeFileIconRequest>>::new();
        let mut seen_requests = HashSet::new();
        for request in requests.into_iter().take(Self::MAX_REQUESTS) {
            if !seen_requests.insert((request.path.clone(), request.kind, request.mode)) {
                continue;
            }
            let path = PathBuf::from(&request.path);
            if !path.is_absolute() {
                continue;
            }
            grouped
                .entry(classify(&path, request.kind, request.mode))
                .or_default()
                .push(request);
        }

        let mut result = NativeFileIconLoadResult {
            unique_identities: grouped.len(),
            ..NativeFileIconLoadResult::default()
        };
        for (query, requests) in grouped {
            #[cfg(target_os = "macos")]
            let _extraction_guard = macos::extraction_guard();
            let provider_variant = platform_provider_variant(&query);
            let lookup = cache.lookup(&query, &provider_variant);
            let png = if let Some(png) = lookup.png {
                result.cache_hits += 1;
                Some(png)
            } else {
                result.system_lookups += 1;
                platform_load_png(&query)
                    .filter(|png| cache::valid_png(png))
                    .inspect(|png| cache.store(&lookup.key, png))
            };
            let Some(png) = png else {
                continue;
            };
            let icon_key = query.key().to_string();
            result
                .assignments
                .extend(
                    requests
                        .into_iter()
                        .map(|request| NativeFileIconAssignment {
                            path: request.path,
                            kind: request.kind,
                            mode: request.mode,
                            icon_key: icon_key.clone(),
                        }),
                );
            result.assets.push(NativeFileIconAsset { icon_key, png });
        }
        result
    }
}

fn classify(path: &Path, kind: NativeFileIconItemKind, mode: NativeFileIconMode) -> IconQuery {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    let path_specific = match mode {
        NativeFileIconMode::Automatic => path_specific(path, kind, extension.as_deref()),
        NativeFileIconMode::Generic => false,
        NativeFileIconMode::Path => true,
    };
    if path_specific {
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.as_os_str().as_encoded_bytes());
        return IconQuery::Path {
            key: format!("path:{}", hasher.finalize().to_hex()),
            path: path.to_path_buf(),
        };
    }
    if kind == NativeFileIconItemKind::Directory {
        return IconQuery::Type {
            key: "kind:folder".to_string(),
            extension: None,
        };
    }
    IconQuery::Type {
        key: extension
            .as_deref()
            .map(|value| format!("ext:{value}"))
            .unwrap_or_else(|| "kind:file".to_string()),
        extension,
    }
}

fn path_specific(path: &Path, kind: NativeFileIconItemKind, extension: Option<&str>) -> bool {
    if path.parent().is_none() {
        return true;
    }
    if matches!(
        extension,
        Some(
            "app"
                | "appex"
                | "bundle"
                | "exe"
                | "ico"
                | "icns"
                | "lnk"
                | "plugin"
                | "prefpane"
                | "url"
        )
    ) {
        return true;
    }
    if kind != NativeFileIconItemKind::Directory {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        if path.join("Icon\r").is_file() {
            return true;
        }
    }
    #[cfg(windows)]
    {
        if path.join("desktop.ini").is_file() {
            return true;
        }
    }
    false
}

#[cfg(target_os = "macos")]
fn platform_provider_variant(query: &IconQuery) -> Vec<u8> {
    macos::provider_variant(query)
}

#[cfg(windows)]
fn platform_provider_variant(query: &IconQuery) -> Vec<u8> {
    windows::provider_variant(query)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_provider_variant(_query: &IconQuery) -> Vec<u8> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn platform_load_png(query: &IconQuery) -> Option<Vec<u8>> {
    macos::load_png(query)
}

#[cfg(windows)]
fn platform_load_png(query: &IconQuery) -> Option<Vec<u8>> {
    windows::load_png(query)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_load_png(_query: &IconQuery) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn ordinary_documents_share_a_type_identity() {
        let first = classify(
            Path::new("/tmp/first.PDF"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        let second = classify(
            Path::new("/tmp/second.pdf"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        assert_eq!(first, second);
        assert_eq!(first.key(), "ext:pdf");
    }

    #[test]
    fn executable_icons_remain_path_specific() {
        let first = classify(
            Path::new("/tmp/first.exe"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        let second = classify(
            Path::new("/tmp/second.exe"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        assert_ne!(first, second);
        assert!(first.key().starts_with("path:"));
    }

    #[test]
    fn ordinary_directories_share_the_folder_identity() {
        let query = classify(
            Path::new("/tmp/ordinary-folder"),
            NativeFileIconItemKind::Directory,
            NativeFileIconMode::Automatic,
        );
        assert_eq!(query.key(), "kind:folder");
    }

    #[test]
    fn generic_directory_mode_ignores_path_specific_folder_metadata() {
        let root = Path::new("/");
        let generic = classify(
            root,
            NativeFileIconItemKind::Directory,
            NativeFileIconMode::Generic,
        );
        let path = classify(
            root,
            NativeFileIconItemKind::Directory,
            NativeFileIconMode::Path,
        );

        assert_eq!(generic.key(), "kind:folder");
        assert!(path.key().starts_with("path:"));
    }

    #[test]
    fn file_extensions_cannot_collide_with_generic_item_kinds() {
        let directory = classify(
            Path::new("/tmp/ordinary-folder"),
            NativeFileIconItemKind::Directory,
            NativeFileIconMode::Automatic,
        );
        let folder_extension = classify(
            Path::new("/tmp/example.folder"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        let extensionless = classify(
            Path::new("/tmp/extensionless"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        let file_extension = classify(
            Path::new("/tmp/example.file"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );

        assert_ne!(directory.key(), folder_extension.key());
        assert_ne!(extensionless.key(), file_extension.key());
        assert_eq!(folder_extension.key(), "ext:folder");
        assert_eq!(file_extension.key(), "ext:file");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires desktop shell icon services"]
    fn concurrent_macos_batches_extract_each_icon_once() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-concurrent-icons-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let root = root.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    NativeFileIconService::load(
                        vec![NativeFileIconRequest {
                            path: "/System/Library/CoreServices/Finder.app".into(),
                            kind: NativeFileIconItemKind::Directory,
                            mode: NativeFileIconMode::Path,
                        }],
                        Some(root),
                    )
                })
            })
            .collect();
        let batches: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(batches
            .iter()
            .all(|batch| batch.assets.len() == 1 && batch.assignments.len() == 1));
        assert_eq!(
            batches
                .iter()
                .map(|batch| batch.system_lookups)
                .sum::<usize>(),
            1
        );
        assert_eq!(
            batches.iter().map(|batch| batch.cache_hits).sum::<usize>(),
            3
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    #[ignore = "requires desktop shell icon services"]
    fn benchmark_native_type_icon_cache() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mangodisk-native-icon-benchmark-{}-{nonce}",
            std::process::id()
        ));
        let cache_root = root.join("cache");
        let requests = (0..128)
            .map(|index| {
                let extension = match index % 3 {
                    0 => "pdf",
                    1 => "mp3",
                    _ => "zip",
                };
                NativeFileIconRequest {
                    path: root
                        .join(format!("sample-{index}.{extension}"))
                        .to_string_lossy()
                        .into_owned(),
                    kind: NativeFileIconItemKind::File,
                    mode: NativeFileIconMode::Automatic,
                }
            })
            .collect::<Vec<_>>();

        let cold_started = Instant::now();
        let cold = NativeFileIconService::load(requests.clone(), Some(cache_root.clone()));
        let cold_elapsed = cold_started.elapsed();
        let warm_started = Instant::now();
        let warm = NativeFileIconService::load(requests, Some(cache_root));
        let warm_elapsed = warm_started.elapsed();
        let baseline_query = classify(
            &root.join("baseline.pdf"),
            NativeFileIconItemKind::File,
            NativeFileIconMode::Automatic,
        );
        let baseline_started = Instant::now();
        let baseline_lookups = 10;
        for _ in 0..baseline_lookups {
            assert!(platform_load_png(&baseline_query).is_some());
        }
        let baseline_elapsed = baseline_started.elapsed();

        println!(
            "native_file_icons cold_us={} warm_us={} uncached_10_us={} requested=128 identities={} cold_system={} warm_system={} warm_cache_hits={}",
            cold_elapsed.as_micros(),
            warm_elapsed.as_micros(),
            baseline_elapsed.as_micros(),
            cold.unique_identities,
            cold.system_lookups,
            warm.system_lookups,
            warm.cache_hits
        );
        assert_eq!(cold.unique_identities, 3);
        assert_eq!(cold.system_lookups, 3);
        assert_eq!(warm.system_lookups, 0);
        assert_eq!(warm.cache_hits, 3);
        assert_eq!(warm.assignments.len(), 128);

        if let Err(error) = std::fs::remove_dir_all(&root) {
            log::debug!("file_icon_benchmark_cleanup_failed error={error}");
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    #[ignore = "requires a standard desktop application"]
    fn resolves_a_real_path_specific_icon() {
        #[cfg(target_os = "macos")]
        let path = PathBuf::from("/System/Applications/TextEdit.app");
        #[cfg(windows)]
        let path = PathBuf::from(r"C:\Windows\System32\notepad.exe");
        if !path.exists() {
            eprintln!("native_file_icon_path_test status=skipped reason=missing_sample");
            return;
        }

        let result = NativeFileIconService::load(
            vec![NativeFileIconRequest {
                path: path.to_string_lossy().into_owned(),
                kind: NativeFileIconItemKind::File,
                mode: NativeFileIconMode::Automatic,
            }],
            None,
        );
        assert_eq!(result.system_lookups, 1);
        assert_eq!(result.assignments.len(), 1);
        assert_eq!(result.assets.len(), 1);
    }
}
