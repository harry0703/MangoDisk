use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use mangodisk_platform::{current_platform, Platform};

#[cfg(target_os = "macos")]
use mangodisk_platform::ScanPurpose;

#[cfg(target_os = "macos")]
const METADATA_OBSERVER_BATCH_FILES: u64 = 256;

pub(crate) struct MetadataFingerprintEntry {
    sort_key: Vec<u8>,
    digest: [u8; 32],
}

#[derive(Default)]
#[cfg(target_os = "macos")]
pub(crate) struct MetadataTreeSnapshot {
    pub(crate) bytes: u64,
    pub(crate) file_count: u64,
    pub(crate) skipped_count: u64,
    pub(crate) fingerprint: Option<[u8; 32]>,
}

/// Rebuilds the metadata snapshot for an analysis directory before deletion.
///
/// The snapshot uses the same skip policy and fingerprint algorithm as the
/// initial scan so equal-sized replacements cannot pass validation. It avoids
/// reading file contents because doing so could repeat full-scan I/O for a
/// large directory immediately before permanent deletion.
#[cfg(target_os = "macos")]
pub(crate) fn snapshot_metadata_tree(
    path: &Path,
    scan_root: &Path,
    purpose: ScanPurpose,
) -> MetadataTreeSnapshot {
    snapshot_metadata_tree_with_observer(path, scan_root, purpose, &|_, _, _| {})
}

/// Builds the same safety snapshot while publishing bounded direct-file batches.
///
/// Large cache directories can contain thousands of direct files and take
/// several seconds to fingerprint. Bounded batches keep progress visibly
/// active without adding an atomic update for every file. Each file is still
/// reported exactly once, so adapters can aggregate counts without overlap.
#[cfg(target_os = "macos")]
pub(crate) fn snapshot_metadata_tree_with_observer(
    path: &Path,
    scan_root: &Path,
    purpose: ScanPurpose,
    observer: &(dyn Fn(&Path, u64, u64) + Sync),
) -> MetadataTreeSnapshot {
    let Ok(entries) = fs::read_dir(path) else {
        return MetadataTreeSnapshot {
            skipped_count: 1,
            ..MetadataTreeSnapshot::default()
        };
    };
    // Publish directory entry before descending. A directory tree can spend
    // noticeable time walking subdirectories that each contain fewer than a
    // full file batch; a zero-sized observation updates the active path
    // without changing aggregate counters.
    observer(path, 0, 0);
    let mut snapshot = MetadataTreeSnapshot::default();
    let mut fingerprint_entries = Vec::new();
    let mut direct_file_count = 0_u64;
    let mut direct_bytes = 0_u64;
    for entry in entries {
        let Ok(entry) = entry else {
            snapshot.skipped_count += 1;
            continue;
        };
        let child_path = entry.path();
        if current_platform()
            .should_skip(&child_path, scan_root, purpose)
            .is_some()
        {
            snapshot.skipped_count += 1;
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&child_path) else {
            snapshot.skipped_count += 1;
            continue;
        };
        if is_link_like(&metadata) {
            snapshot.skipped_count += 1;
            continue;
        }
        if metadata.is_dir() {
            let child =
                snapshot_metadata_tree_with_observer(&child_path, scan_root, purpose, observer);
            snapshot.bytes = snapshot.bytes.saturating_add(child.bytes);
            snapshot.file_count = snapshot.file_count.saturating_add(child.file_count);
            snapshot.skipped_count = snapshot.skipped_count.saturating_add(child.skipped_count);
            if let Some(entry) =
                metadata_fingerprint_entry(&child_path, &metadata, child.fingerprint)
            {
                fingerprint_entries.push(entry);
            } else {
                snapshot.skipped_count += 1;
            }
        } else if metadata.is_file() {
            snapshot.bytes = snapshot.bytes.saturating_add(metadata.len());
            snapshot.file_count += 1;
            direct_bytes = direct_bytes.saturating_add(metadata.len());
            direct_file_count = direct_file_count.saturating_add(1);
            if let Some(entry) = metadata_fingerprint_entry(&child_path, &metadata, None) {
                fingerprint_entries.push(entry);
            } else {
                snapshot.skipped_count += 1;
            }
            if direct_file_count >= METADATA_OBSERVER_BATCH_FILES {
                observer(path, direct_file_count, direct_bytes);
                direct_file_count = 0;
                direct_bytes = 0;
            }
        } else {
            // Sockets, devices, and other special objects cannot be validated
            // with regular-file semantics. Reject the containing directory so
            // the deletion implementation never receives platform-specific
            // objects with ambiguous behavior.
            snapshot.skipped_count += 1;
        }
    }
    if direct_file_count > 0 {
        observer(path, direct_file_count, direct_bytes);
    }
    if snapshot.skipped_count == 0 {
        snapshot.fingerprint = Some(finalize_metadata_fingerprint(fingerprint_entries));
    }
    snapshot
}

pub(crate) fn metadata_fingerprint_entry(
    path: &Path,
    metadata: &fs::Metadata,
    child_fingerprint: Option<[u8; 32]>,
) -> Option<MetadataFingerprintEntry> {
    // A path and size alone cannot detect an equal-sized replacement when the
    // modification time is unavailable. Mark the snapshot incomplete instead
    // of publishing a weak fingerprint.
    let modified_at_nanos = metadata.modified().ok().and_then(system_time_nanos)?;
    let sort_key = path
        .file_name()
        .map(path_component_bytes)
        .unwrap_or_default();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-metadata-entry-v1");
    hasher.update(&(sort_key.len() as u64).to_le_bytes());
    hasher.update(&sort_key);
    hasher.update(&[if metadata.is_dir() { 2 } else { 1 }]);
    hasher.update(&metadata.len().to_le_bytes());
    hasher.update(&modified_at_nanos.to_le_bytes());
    match child_fingerprint {
        Some(value) => {
            hasher.update(&[1]);
            hasher.update(&value);
        }
        None => {
            hasher.update(&[0]);
        }
    }
    Some(MetadataFingerprintEntry {
        sort_key,
        digest: *hasher.finalize().as_bytes(),
    })
}

pub(crate) fn finalize_metadata_fingerprint(
    mut entries: Vec<MetadataFingerprintEntry>,
) -> [u8; 32] {
    // read_dir does not guarantee ordering. Sort by the original file-name
    // bytes so filesystem-specific traversal order cannot produce a false
    // content-change result.
    entries.sort_unstable_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| left.digest.cmp(&right.digest))
    });
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-metadata-directory-v1");
    hasher.update(&(entries.len() as u64).to_le_bytes());
    for entry in entries {
        hasher.update(&entry.digest);
    }
    *hasher.finalize().as_bytes()
}

pub(crate) fn display_fingerprint(fingerprint: [u8; 32]) -> String {
    blake3::Hash::from_bytes(fingerprint).to_hex().to_string()
}

fn system_time_nanos(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

#[cfg(unix)]
fn path_component_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(windows)]
fn path_component_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>()
}

/// Scans and cleanup reject link-like entries and nonresident cloud placeholders.
///
/// Links can escape into user data or another volume. Cloud placeholders can fetch remote content
/// when opened, so the same platform safety boundary prevents background scans from materializing
/// files that the user intentionally kept online-only.
pub(crate) fn is_link_like(metadata: &fs::Metadata) -> bool {
    current_platform().is_link_like(metadata)
}

pub(crate) fn modified_ms(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
}

pub(crate) fn latest_timestamp(current: Option<u64>, candidate: Option<u64>) -> Option<u64> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.max(candidate)),
        (current, candidate) => current.or(candidate),
    }
}

/// Preserves the operating system path representation for internal identity comparisons.
/// Windows canonical paths may retain their verbatim prefix here; public results must use
/// `display_path` so adapters never receive mixed representations for the same scan.
pub(crate) fn native_path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn display_path(path: &Path) -> String {
    current_platform().display_path(path)
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Keep the full target path: identical filenames in different directories are
/// common, and a hash cannot tell support which target failed. Never use this
/// diagnostic representation for filesystem access or identity comparisons.
pub fn diagnostic_path(path: &Path) -> String {
    mangodisk_platform::diagnostics::text(&display_path(path))
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use std::{fs, thread, time::Duration};

    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn display_paths_remove_windows_verbatim_prefixes() {
        assert_eq!(
            native_path_string(Path::new(r"\\?\C:\fixture\sample.bin")),
            r"\\?\C:\fixture\sample.bin"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\C:\fixture\sample.bin")),
            r"C:\fixture\sample.bin"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\unc\server\share\sample.bin")),
            r"\\server\share\sample.bin"
        );
    }

    #[test]
    fn diagnostic_paths_identify_the_target_and_escape_record_boundaries() {
        let first = diagnostic_path(Path::new("/fixture/first/Cache"));
        let second = diagnostic_path(Path::new("/fixture/second/Cache"));
        assert!(first.contains("/fixture/first/Cache"));
        assert!(second.contains("/fixture/second/Cache"));
        assert_ne!(first, second);
        assert!(!diagnostic_path(Path::new("/fixture/line\nbreak")).contains('\n'));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn metadata_fingerprint_detects_equal_sized_nested_replacements() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-fingerprint-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let nested = root.join("nested");
        let file = nested.join("sample.bin");
        fs::create_dir_all(&nested).expect("the temporary directory should be created");
        fs::write(&file, b"before").expect("the initial fixture file should be written");

        let before = snapshot_metadata_tree(&root, &root, ScanPurpose::Analysis);
        // Filesystems may have coarse modification timestamps. A short delay
        // keeps the test focused on equal-sized replacement metadata rather
        // than timestamp resolution.
        thread::sleep(Duration::from_millis(20));
        fs::write(&file, b"after!")
            .expect("the fixture should be replaced with equal-sized content");
        let after = snapshot_metadata_tree(&root, &root, ScanPurpose::Analysis);

        assert_eq!(before.bytes, after.bytes);
        assert_eq!(before.file_count, after.file_count);
        assert_ne!(before.fingerprint, after.fingerprint);
        fs::remove_dir_all(root).expect("the temporary directory should be removed");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn observed_directory_batches_count_each_file_once() {
        use std::sync::Mutex;

        let root = std::env::temp_dir().join(format!(
            "mangodisk-metadata-observer-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("the observer fixture should be created");
        for index in 0..=METADATA_OBSERVER_BATCH_FILES {
            fs::write(root.join(format!("root-{index}.bin")), b"root")
                .expect("the root fixture should be written");
        }
        fs::write(nested.join("nested.bin"), b"nested")
            .expect("the nested fixture should be written");
        let observed = Mutex::new((0_u64, 0_u64, Vec::new()));

        let snapshot = snapshot_metadata_tree_with_observer(
            &root,
            &root,
            ScanPurpose::Analysis,
            &|path, file_count, bytes| {
                let mut observed = observed
                    .lock()
                    .expect("the observer fixture lock should remain valid");
                observed.0 = observed.0.saturating_add(file_count);
                observed.1 = observed.1.saturating_add(bytes);
                observed.2.push(path.to_path_buf());
            },
        );
        let observed = observed
            .lock()
            .expect("the observer fixture lock should remain valid");

        assert_eq!(observed.0, snapshot.file_count);
        assert_eq!(observed.1, snapshot.bytes);
        assert!(observed.2.contains(&root));
        assert!(observed.2.contains(&nested));
        assert!(
            observed
                .2
                .iter()
                .filter(|path| path.as_path() == root.as_path())
                .count()
                >= 2
        );

        fs::remove_dir_all(root).expect("the observer fixture should be removed");
    }
}
