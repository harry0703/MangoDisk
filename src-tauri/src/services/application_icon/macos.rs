use std::{
    collections::HashSet,
    env,
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use icns::IconFamily;

use super::{cache::ApplicationIconCache, ApplicationIcon, ApplicationIconLoadResult};

const MAX_INFO_PLIST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ICNS_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PNG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RESOURCE_ENTRIES: usize = 512;
const TARGET_ICON_PIXELS: u32 = 128;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const CGBI_CHUNK: &[u8] = b"CgBI";
static NORMALIZATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct BundleLayout {
    info_plist: PathBuf,
    resources: PathBuf,
}

pub(super) fn load(paths: Vec<String>, cache_root: Option<PathBuf>) -> ApplicationIconLoadResult {
    let cache = ApplicationIconCache::new(cache_root);
    let mut result = ApplicationIconLoadResult::default();

    for path in paths {
        let Some((icon, cache_hit)) = load_icon(path, &cache) else {
            continue;
        };
        result.icons.push(icon);
        if cache_hit {
            result.cache_hits += 1;
        } else {
            result.decoded_icons += 1;
        }
    }
    result
}

fn load_icon(path: String, cache: &ApplicationIconCache) -> Option<(ApplicationIcon, bool)> {
    let bundle = PathBuf::from(&path);
    if !bundle.is_absolute()
        || !bundle.is_dir()
        || !bundle
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return None;
    }

    for candidate in icon_candidates(&bundle) {
        let Some(lookup) = cache.lookup(&candidate, &[]) else {
            continue;
        };
        if let Some(png) = lookup.png {
            return Some((application_icon(path, png), true));
        }
        if let Some(png) = decode_icon(&candidate) {
            cache.store(&lookup.key, &png);
            return Some((application_icon(path, png), false));
        }
    }
    None
}

fn application_icon(path: String, png: Vec<u8>) -> ApplicationIcon {
    ApplicationIcon::new(
        path,
        format!("data:image/png;base64,{}", STANDARD.encode(png)),
    )
}

fn declared_icon_names(info_plist: &Path) -> Vec<String> {
    if fs::metadata(info_plist)
        .map(|metadata| metadata.len() > MAX_INFO_PLIST_BYTES)
        .unwrap_or(true)
    {
        return Vec::new();
    }

    let Ok(value) = plist::Value::from_file(info_plist) else {
        return Vec::new();
    };
    let Some(dictionary) = value.as_dictionary() else {
        return Vec::new();
    };

    let mut names = Vec::new();
    collect_declared_icon_names(dictionary, &mut names);
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.to_ascii_lowercase()));
    names
}

fn collect_declared_icon_names(dictionary: &plist::Dictionary, names: &mut Vec<String>) {
    for key in ["CFBundleIconFile", "CFBundleIconName"] {
        if let Some(name) = dictionary.get(key).and_then(plist::Value::as_string) {
            names.push(name.to_string());
        }
    }
    if let Some(values) = dictionary
        .get("CFBundleIconFiles")
        .and_then(plist::Value::as_array)
    {
        names.extend(
            values
                .iter()
                .filter_map(plist::Value::as_string)
                .map(str::to_string),
        );
    }
    for key in ["CFBundleIcons", "CFBundleIcons~ipad"] {
        let Some(icon_dictionary) = dictionary.get(key).and_then(plist::Value::as_dictionary)
        else {
            continue;
        };
        let Some(primary_icon) = icon_dictionary
            .get("CFBundlePrimaryIcon")
            .and_then(plist::Value::as_dictionary)
        else {
            continue;
        };
        collect_declared_icon_names(primary_icon, names);
    }
}

fn bundle_layout(bundle: &Path) -> Option<BundleLayout> {
    let standard_info = bundle.join("Contents/Info.plist");
    if standard_info.is_file() {
        return Some(BundleLayout {
            info_plist: standard_info,
            resources: bundle.join("Contents/Resources"),
        });
    }
    let flat_info = bundle.join("Info.plist");
    flat_info.is_file().then(|| BundleLayout {
        info_plist: flat_info,
        resources: bundle.to_path_buf(),
    })
}

fn icon_candidates(bundle: &Path) -> Vec<PathBuf> {
    let Some(layout) = bundle_layout(bundle) else {
        return Vec::new();
    };
    let Ok(canonical_resources) = fs::canonicalize(&layout.resources) else {
        return Vec::new();
    };

    let resource_files = fs::read_dir(&layout.resources)
        .into_iter()
        .flatten()
        .take(MAX_RESOURCE_ENTRIES)
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for name in declared_icon_names(&layout.info_plist) {
        let declared = Path::new(&name);
        if declared.extension().is_some() {
            candidates.push(layout.resources.join(declared));
            continue;
        }
        candidates.push(layout.resources.join(format!("{name}.icns")));
        candidates.push(layout.resources.join(format!("{name}.png")));

        // Flat iOS bundles commonly declare `AppIcon60x60` while shipping
        // scale-qualified files such as `AppIcon60x60@2x.png`.
        let mut scaled = resource_files
            .iter()
            .filter(|path| {
                let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                    return false;
                };
                let lower_name = name.to_ascii_lowercase();
                let lower_file_name = file_name.to_ascii_lowercase();
                lower_file_name.starts_with(&format!("{lower_name}@"))
                    && is_supported_icon_file(path)
            })
            .cloned()
            .collect::<Vec<_>>();
        scaled.sort();
        scaled.reverse();
        candidates.extend(scaled);
    }
    candidates.push(layout.resources.join("AppIcon.icns"));

    // Some bundles omit the Info.plist icon keys. A bounded, sorted fallback
    // avoids arbitrary traversal and unstable directory ordering.
    let mut fallback_icons = resource_files
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("icns"))
        })
        .collect::<Vec<_>>();
    fallback_icons.sort();
    candidates.extend(fallback_icons);

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|candidate| fs::canonicalize(candidate).ok())
        .filter(|candidate| candidate.starts_with(&canonical_resources))
        .filter(|candidate| seen.insert(candidate.clone()))
        .filter(|candidate| {
            fs::metadata(candidate)
                .map(|metadata| {
                    metadata.is_file()
                        && is_supported_icon_file(candidate)
                        && metadata.len() <= maximum_icon_bytes(candidate)
                })
                .unwrap_or(false)
        })
        .collect()
}

fn is_supported_icon_file(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("icns") || extension.eq_ignore_ascii_case("png")
    })
}

fn maximum_icon_bytes(path: &Path) -> u64 {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        MAX_PNG_BYTES
    } else {
        MAX_ICNS_BYTES
    }
}

fn decode_icon(icon_path: &Path) -> Option<Vec<u8>> {
    if icon_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        let png = fs::read(icon_path)
            .ok()
            .filter(|png| png.starts_with(PNG_SIGNATURE) && png.len() as u64 <= MAX_PNG_BYTES)?;
        return if png_has_chunk(&png, CGBI_CHUNK) {
            normalize_apple_png(icon_path)
        } else {
            Some(png)
        };
    }

    let file = File::open(icon_path).ok()?;
    let family = IconFamily::read(BufReader::new(file)).ok()?;
    let mut icon_types = family.available_icons();
    icon_types.sort_by_key(|icon_type| {
        icon_type
            .pixel_width()
            .abs_diff(TARGET_ICON_PIXELS)
            .saturating_add(icon_type.pixel_height().abs_diff(TARGET_ICON_PIXELS))
    });

    icon_types.into_iter().find_map(|icon_type| {
        let image = family.get_icon_with_type(icon_type).ok()?;
        let mut png = Vec::new();
        image.write_png(&mut png).ok()?;
        Some(png)
    })
}

fn png_has_chunk(png: &[u8], expected: &[u8]) -> bool {
    if !png.starts_with(PNG_SIGNATURE) {
        return false;
    }
    let mut offset = PNG_SIGNATURE.len();
    while offset.checked_add(12).is_some_and(|end| end <= png.len()) {
        let length = u32::from_be_bytes([
            png[offset],
            png[offset + 1],
            png[offset + 2],
            png[offset + 3],
        ]) as usize;
        if &png[offset + 4..offset + 8] == expected {
            return true;
        }
        let Some(next) = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
        else {
            return false;
        };
        if next > png.len() {
            return false;
        }
        offset = next;
    }
    false
}

fn normalize_apple_png(icon_path: &Path) -> Option<Vec<u8>> {
    // App Store iOS bundles may contain Apple's CgBI PNG variant, which is
    // not portable to every WebView decoder. `sips` converts it through the
    // native image stack; a private temporary directory prevents output-path
    // substitution and the normalized bytes then enter the regular disk cache.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let sequence = NORMALIZATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_directory = env::temp_dir().join(format!(
        "mangodisk-icon-normalize-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&temporary_directory).ok()?;
    let output_path = temporary_directory.join("icon.png");
    let command = Command::new("/usr/bin/sips")
        .args(["-s", "format", "png"])
        .arg(icon_path)
        .arg("--out")
        .arg(&output_path)
        .output();
    let png = command
        .ok()
        .filter(|output| output.status.success())
        .and_then(|_| fs::read(&output_path).ok())
        .filter(|png| {
            png.starts_with(PNG_SIGNATURE)
                && png.len() as u64 <= MAX_PNG_BYTES
                && !png_has_chunk(png, CGBI_CHUNK)
        });
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(temporary_directory);
    png
}

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mangodisk-icon-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn flat_bundle_resolves_nested_scale_qualified_png_icon() {
        let root = fixture_path("flat-bundle");
        let bundle = root.join("Wrapped.app");
        fs::create_dir_all(&bundle).expect("flat bundle fixture must be created");

        let mut primary_icon = plist::Dictionary::new();
        primary_icon.insert(
            "CFBundleIconFiles".to_string(),
            plist::Value::Array(vec![plist::Value::String("AppIcon60x60".to_string())]),
        );
        let mut icons = plist::Dictionary::new();
        icons.insert(
            "CFBundlePrimaryIcon".to_string(),
            plist::Value::Dictionary(primary_icon),
        );
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIcons".to_string(), plist::Value::Dictionary(icons));
        plist::Value::Dictionary(info)
            .to_file_xml(bundle.join("Info.plist"))
            .expect("flat bundle Info.plist fixture must be written");
        let icon = bundle.join("AppIcon60x60@2x.png");
        fs::write(&icon, PNG_SIGNATURE).expect("PNG icon fixture must be written");

        let candidates = icon_candidates(&bundle);

        assert_eq!(
            candidates,
            vec![fs::canonicalize(icon).expect("icon fixture should canonicalize")]
        );
        fs::remove_dir_all(root).expect("flat bundle fixture must be removed");
    }

    #[test]
    fn png_icon_is_returned_without_native_icns_decoding() {
        let root = fixture_path("png-decode");
        let icon = root.join("icon.png");
        fs::create_dir_all(&root).expect("PNG fixture directory must be created");
        fs::write(&icon, [PNG_SIGNATURE, b"fixture"].concat())
            .expect("PNG icon fixture must be written");

        assert_eq!(
            decode_icon(&icon),
            Some([PNG_SIGNATURE, b"fixture"].concat())
        );
        fs::remove_dir_all(root).expect("PNG fixture directory must be removed");
    }

    #[test]
    fn apple_png_chunk_is_detected_without_matching_payload_bytes() {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&0_u32.to_be_bytes());
        png.extend_from_slice(CGBI_CHUNK);
        png.extend_from_slice(&0_u32.to_be_bytes());

        assert!(png_has_chunk(&png, CGBI_CHUNK));
        assert!(!png_has_chunk(
            &[PNG_SIGNATURE, b"payload-CgBI"].concat(),
            CGBI_CHUNK
        ));
    }

    /// Verifies the native CgBI conversion path against any matching App Store
    /// application installed on the current development machine.
    #[test]
    #[ignore = "reads locally installed macOS applications"]
    fn real_apple_optimized_png_is_normalized_for_webview_rendering() {
        let bundles = fs::read_dir("/Applications")
            .expect("the macOS Applications directory must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
            .flat_map(|bundle| {
                let mut bundles = vec![bundle.clone()];
                if let Ok(wrapped) = fs::canonicalize(bundle.join("WrappedBundle")) {
                    bundles.push(wrapped);
                }
                bundles
            });

        for bundle in bundles {
            for candidate in icon_candidates(&bundle) {
                let Some(source) = fs::read(&candidate).ok().filter(|png| {
                    png.len() as u64 <= MAX_PNG_BYTES && png_has_chunk(png, CGBI_CHUNK)
                }) else {
                    continue;
                };
                assert!(source.starts_with(PNG_SIGNATURE));
                let normalized =
                    decode_icon(&candidate).expect("native CgBI conversion should succeed");
                assert!(normalized.starts_with(PNG_SIGNATURE));
                assert!(!png_has_chunk(&normalized, CGBI_CHUNK));
                return;
            }
        }
        panic!("at least one installed CgBI application icon is required");
    }

    /// Exercises native decoding and a second independent cache reader against
    /// locally installed applications. It remains opt-in because the installed
    /// catalog is host-specific and unsuitable for deterministic CI.
    #[test]
    #[ignore = "reads locally installed macOS applications"]
    fn real_application_icons_are_reused_from_disk_cache() {
        let applications = fs::read_dir("/Applications")
            .expect("the macOS Applications directory must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
            .take(12)
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            !applications.is_empty(),
            "at least one installed application is required"
        );

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos();
        let cache_root = std::env::temp_dir().join(format!(
            "mangodisk-real-icon-cache-{}-{nonce}",
            std::process::id()
        ));

        let cold_started = Instant::now();
        let cold = load(applications.clone(), Some(cache_root.clone()));
        let cold_elapsed = cold_started.elapsed();
        assert!(
            !cold.icons.is_empty(),
            "at least one native icon must decode"
        );
        assert_eq!(cold.cache_hits, 0);
        assert_eq!(cold.decoded_icons, cold.icons.len());

        let warm_started = Instant::now();
        let warm = load(applications, Some(cache_root.clone()));
        let warm_elapsed = warm_started.elapsed();
        assert_eq!(warm.icons.len(), cold.icons.len());
        assert_eq!(warm.cache_hits, warm.icons.len());
        assert_eq!(warm.decoded_icons, 0);

        eprintln!(
            "real_application_icon_cache cold_ms={} warm_ms={} icons={}",
            cold_elapsed.as_millis(),
            warm_elapsed.as_millis(),
            warm.icons.len()
        );
        fs::remove_dir_all(cache_root).expect("test cache root must be removed");
    }
}
