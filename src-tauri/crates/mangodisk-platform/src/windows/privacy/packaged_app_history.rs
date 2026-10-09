use std::{
    collections::BTreeSet,
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE},
    types::FromRegValue,
    RegKey,
};

use crate::{
    PlatformError, PlatformErrorCode, PlatformPrivacyApplicationNativeTraceKind,
    PlatformPrivacyDetailEntry, PlatformResult,
};

const PAINT_PACKAGE: &str = "Microsoft.Paint_8wekyb3d8bbwe";
const PAINT_RECENT_DOCUMENTS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Applets\Paint\Recent File List";
const NOTEPAD_PACKAGE: &str = "Microsoft.WindowsNotepad_8wekyb3d8bbwe";
const MAX_NOTEPAD_STATE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_NOTEPAD_STATE_FILES: usize = 10_000;

#[derive(Debug)]
pub(super) struct Snapshot {
    pub(super) item_count: u64,
    pub(super) revision: String,
}

#[derive(Debug)]
struct HistoryRecords {
    labels: Vec<String>,
    revision: String,
}

pub(super) fn snapshot(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> Option<PlatformResult<Snapshot>> {
    let result = match trace {
        PlatformPrivacyApplicationNativeTraceKind::PaintRecentDocuments => paint_records(),
        PlatformPrivacyApplicationNativeTraceKind::WindowsNotepadSessionHistory => {
            notepad_records()
        }
        _ => return None,
    };
    Some(result.map(|records| Snapshot {
        item_count: records.labels.len() as u64,
        revision: records.revision,
    }))
}

pub(super) fn details(
    trace: PlatformPrivacyApplicationNativeTraceKind,
    offset: u64,
    limit: u32,
) -> Option<PlatformResult<Vec<PlatformPrivacyDetailEntry>>> {
    let result = match trace {
        PlatformPrivacyApplicationNativeTraceKind::PaintRecentDocuments => paint_records(),
        PlatformPrivacyApplicationNativeTraceKind::WindowsNotepadSessionHistory => {
            notepad_records()
        }
        _ => return None,
    };
    Some(result.map(|records| page(records.labels, offset, limit)))
}

pub(super) fn clear(
    trace: PlatformPrivacyApplicationNativeTraceKind,
) -> Option<PlatformResult<bool>> {
    match trace {
        PlatformPrivacyApplicationNativeTraceKind::PaintRecentDocuments => Some(clear_paint()),
        PlatformPrivacyApplicationNativeTraceKind::WindowsNotepadSessionHistory => {
            Some(clear_notepad())
        }
        _ => None,
    }
}

fn local_data() -> PlatformResult<PathBuf> {
    dirs::data_local_dir()
        .ok_or_else(|| PlatformError::invalid_path("local application data is unavailable"))
}

fn paint_hive_path() -> PlatformResult<PathBuf> {
    Ok(local_data()?
        .join("Packages")
        .join(PAINT_PACKAGE)
        .join("SystemAppData/Helium/User.dat"))
}

/// Modern Paint stores its private registry below the package's `User.dat` application hive.
/// The same logical key is retained below HKCU by classic Paint, so inspecting both roots keeps
/// Windows 10 compatibility without guessing at unrelated shell history.
fn paint_records() -> PlatformResult<HistoryRecords> {
    let mut labels = Vec::new();
    let mut seen = BTreeSet::new();
    let mut revision = blake3::Hasher::new();
    revision.update(b"mangodisk-windows-paint-history-v1\0");

    if let Some(hive) = load_application_hive(&paint_hive_path()?, KEY_READ)? {
        collect_paint_records(
            &hive,
            PAINT_RECENT_DOCUMENTS,
            &mut labels,
            &mut seen,
            &mut revision,
        )?;
    } else {
        revision.update(b"missing-package-hive");
    }
    collect_paint_records(
        &RegKey::predef(HKEY_CURRENT_USER),
        PAINT_RECENT_DOCUMENTS,
        &mut labels,
        &mut seen,
        &mut revision,
    )?;

    Ok(HistoryRecords {
        labels,
        revision: revision.finalize().to_hex().to_string(),
    })
}

fn collect_paint_records(
    root: &RegKey,
    history_path: &str,
    labels: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    revision: &mut blake3::Hasher,
) -> PlatformResult<()> {
    let key = match root.open_subkey_with_flags(history_path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            revision.update(b"missing-history-key");
            return Ok(());
        }
        Err(error) => return Err(PlatformError::io("open Paint recent documents", &error)),
    };
    let mut records = Vec::new();
    for value in key.enum_values() {
        let (name, value) =
            value.map_err(|error| PlatformError::io("read Paint recent document", &error))?;
        let Some(position) = numbered_value_position(&name, "File") else {
            continue;
        };
        let label = String::from_reg_value(&value)
            .map_err(|error| PlatformError::io("decode Paint recent document", &error))?;
        revision.update(name.as_bytes());
        revision.update(&value.bytes);
        if !label.trim().is_empty() {
            records.push((position, label));
        }
    }
    records.sort_by_key(|(position, _)| *position);
    for (_, label) in records {
        let identity = label.to_lowercase();
        if seen.insert(identity) {
            labels.push(label);
        }
    }
    Ok(())
}

fn clear_paint() -> PlatformResult<bool> {
    let hive_path = paint_hive_path()?;
    if let Some(hive) = load_application_hive(&hive_path, KEY_QUERY_VALUE | KEY_SET_VALUE)? {
        clear_paint_root(&hive, PAINT_RECENT_DOCUMENTS)?;
    }
    clear_paint_root(&RegKey::predef(HKEY_CURRENT_USER), PAINT_RECENT_DOCUMENTS)?;
    let remaining = paint_records()?.labels.len();
    log::info!("windows_paint_history_cleared remaining_count={remaining}");
    Ok(remaining == 0)
}

fn clear_paint_root(root: &RegKey, history_path: &str) -> PlatformResult<()> {
    let key = match root.open_subkey_with_flags(history_path, KEY_QUERY_VALUE | KEY_SET_VALUE) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(PlatformError::io("open Paint recent documents", &error)),
    };
    let names = key
        .enum_values()
        .map(|value| value.map(|(name, _)| name))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| PlatformError::io("enumerate Paint recent documents", &error))?;
    for name in names {
        if numbered_value_position(&name, "File").is_none() {
            continue;
        }
        key.delete_value(&name).map_err(|error| {
            PlatformError::io("clear Paint recent document", &error).with_possible_side_effects()
        })?;
    }
    Ok(())
}

fn load_application_hive(path: &Path, permissions: u32) -> PlatformResult<Option<RegKey>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(PlatformError::io(
                "inspect application registry hive",
                &error,
            ))
        }
    };
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(PlatformError::invalid_path(
            "application registry hive is not a safe regular file",
        ));
    }
    RegKey::load_app_key_with_flags(path, permissions, 0)
        .map(Some)
        .map_err(|error| PlatformError::io("load application registry hive", &error))
}

fn numbered_value_position(name: &str, prefix: &str) -> Option<u32> {
    let suffix = name.get(prefix.len()..)?;
    name.get(..prefix.len())?
        .eq_ignore_ascii_case(prefix)
        .then_some(())?;
    (!suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

fn notepad_state_root() -> PlatformResult<PathBuf> {
    Ok(local_data()?
        .join("Packages")
        .join(NOTEPAD_PACKAGE)
        .join("LocalState"))
}

/// Current Notepad restores open files and unsaved drafts from TabState rather than maintaining a
/// separate low-risk MRU list. Treating this source as editor history keeps it manual and
/// data-loss-sensitive in Core while still exposing file-backed tabs with their complete paths.
fn notepad_records() -> PlatformResult<HistoryRecords> {
    let root = notepad_state_root()?;
    notepad_records_at(&root)
}

fn notepad_records_at(root: &Path) -> PlatformResult<HistoryRecords> {
    let tab_root = root.join("TabState");
    let state_files = notepad_state_files(root)?;
    let mut labels = Vec::new();
    let mut revision = blake3::Hasher::new();
    revision.update(b"mangodisk-windows-notepad-session-v1\0");

    for path in state_files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| PlatformError::invalid_path("Notepad state file name is invalid"))?;
        let bytes = read_bounded_state_file(&path)?;
        revision.update(name.as_bytes());
        revision.update(blake3::hash(&bytes).as_bytes());
        if path.parent() != Some(tab_root.as_path()) || !is_notepad_tab_record(name) {
            continue;
        }
        let identifier = name.trim_end_matches(".bin");
        labels.push(
            notepad_document_path(&bytes)
                .unwrap_or_else(|| format!("Unsaved tab · {}", &identifier[..8])),
        );
    }

    Ok(HistoryRecords {
        labels,
        revision: revision.finalize().to_hex().to_string(),
    })
}

fn clear_notepad() -> PlatformResult<bool> {
    let root = notepad_state_root()?;
    clear_notepad_at(&root)
}

fn clear_notepad_at(root: &Path) -> PlatformResult<bool> {
    let files = notepad_state_files(root)?;
    let mut removed_count = 0_u64;
    for path in files {
        fs::remove_file(&path).map_err(|error| {
            PlatformError::io("clear Notepad session history", &error).with_possible_side_effects()
        })?;
        removed_count = removed_count.saturating_add(1);
    }
    let remaining = notepad_records_at(root)?.labels.len();
    log::info!(
        "windows_notepad_session_history_cleared removed_file_count={removed_count} remaining_count={remaining}"
    );
    Ok(remaining == 0)
}

fn notepad_state_files(root: &Path) -> PlatformResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for directory in [root.join("TabState"), root.join("WindowState")] {
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(PlatformError::io("inspect Notepad state directory", &error)),
        };
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(PlatformError::invalid_path(
                "Notepad state directory is not a safe regular directory",
            ));
        }
        for entry in fs::read_dir(&directory)
            .map_err(|error| PlatformError::io("read Notepad state directory", &error))?
        {
            let entry =
                entry.map_err(|error| PlatformError::io("read Notepad state entry", &error))?;
            let path = entry.path();
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.to_ascii_lowercase().ends_with(".bin") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| PlatformError::io("inspect Notepad state entry", &error))?;
            if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(PlatformError::invalid_path(
                    "Notepad state entry is not a safe regular file",
                ));
            }
            if files.len() >= MAX_NOTEPAD_STATE_FILES {
                return Err(PlatformError::new(
                    PlatformErrorCode::InvalidData,
                    "Notepad state exceeds the supported file count",
                ));
            }
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn read_bounded_state_file(path: &Path) -> PlatformResult<Vec<u8>> {
    let metadata = fs::metadata(path)
        .map_err(|error| PlatformError::io("inspect Notepad state file", &error))?;
    if metadata.len() > MAX_NOTEPAD_STATE_FILE_BYTES {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "Notepad state file exceeds the supported size",
        ));
    }
    fs::read(path).map_err(|error| PlatformError::io("read Notepad state file", &error))
}

fn is_notepad_tab_record(name: &str) -> bool {
    let Some(identifier) = name.strip_suffix(".bin") else {
        return false;
    };
    if identifier.len() != 36
        || identifier.as_bytes().get(8) != Some(&b'-')
        || identifier.as_bytes().get(13) != Some(&b'-')
        || identifier.as_bytes().get(18) != Some(&b'-')
        || identifier.as_bytes().get(23) != Some(&b'-')
    {
        return false;
    }
    identifier
        .bytes()
        .enumerate()
        .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn notepad_document_path(bytes: &[u8]) -> Option<String> {
    if bytes.get(..4)? != [b'N', b'P', 0, 1] {
        return None;
    }
    let (character_count, consumed) = decode_seven_bit_integer(bytes.get(4..)?)?;
    let start = 4_usize.checked_add(consumed)?;
    let byte_count = usize::try_from(character_count).ok()?.checked_mul(2)?;
    let encoded = bytes.get(start..start.checked_add(byte_count)?)?;
    let units = encoded
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    let path = String::from_utf16(&units).ok()?;
    (!path.trim().is_empty()).then_some(path)
}

fn decode_seven_bit_integer(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut value = 0_u32;
    for (index, byte) in bytes.iter().copied().take(5).enumerate() {
        value |= u32::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

fn page(labels: Vec<String>, offset: u64, limit: u32) -> Vec<PlatformPrivacyDetailEntry> {
    labels
        .into_iter()
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(limit as usize)
        .map(|label| PlatformPrivacyDetailEntry {
            label,
            item_count: 1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-packaged-history-{name}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn notepad_record(path: Option<&str>) -> Vec<u8> {
        let mut bytes = vec![b'N', b'P', 0, u8::from(path.is_some())];
        if let Some(path) = path {
            let units = path.encode_utf16().collect::<Vec<_>>();
            let mut length = units.len() as u32;
            loop {
                let mut byte = (length & 0x7f) as u8;
                length >>= 7;
                if length != 0 {
                    byte |= 0x80;
                }
                bytes.push(byte);
                if length == 0 {
                    break;
                }
            }
            for unit in units {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn notepad_parser_returns_the_complete_saved_document_path() {
        let path = r"C:\Users\Fixture\Documents\notes.txt";
        assert_eq!(
            notepad_document_path(&notepad_record(Some(path))),
            Some(path.into())
        );
        assert_eq!(notepad_document_path(&notepad_record(None)), None);
    }

    #[test]
    fn notepad_snapshot_counts_logical_tabs_and_cleanup_removes_only_state_files() {
        let root = fixture_root("notepad");
        let tab_state = root.join("TabState");
        let window_state = root.join("WindowState");
        fs::create_dir_all(&tab_state).unwrap();
        fs::create_dir_all(&window_state).unwrap();
        fs::write(
            tab_state.join("11111111-1111-1111-1111-111111111111.bin"),
            notepad_record(Some(r"C:\Fixture\notes.txt")),
        )
        .unwrap();
        fs::write(
            tab_state.join("22222222-2222-2222-2222-222222222222.bin"),
            notepad_record(None),
        )
        .unwrap();
        fs::write(
            tab_state.join("22222222-2222-2222-2222-222222222222.0.bin"),
            b"draft",
        )
        .unwrap();
        fs::write(window_state.join("window.bin"), b"window").unwrap();
        fs::write(root.join("keep.txt"), b"keep").unwrap();

        let records = notepad_records_at(&root).unwrap();
        assert_eq!(
            records.labels,
            [
                r"C:\Fixture\notes.txt".to_owned(),
                "Unsaved tab · 22222222".to_owned(),
            ]
        );
        assert!(clear_notepad_at(&root).unwrap());
        assert!(root.join("keep.txt").is_file());
        assert!(notepad_state_files(&root).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn paint_value_names_require_the_exact_numbered_file_shape() {
        assert_eq!(numbered_value_position("File1", "File"), Some(1));
        assert_eq!(numbered_value_position("file12", "File"), Some(12));
        assert_eq!(numbered_value_position("File", "File"), None);
        assert_eq!(numbered_value_position("File1Meta", "File"), None);
    }

    #[test]
    fn paint_snapshot_orders_records_and_cleanup_preserves_unrelated_settings() {
        let history_path = format!(
            r"Software\MangoDisk\Tests\Paint-{}-{}\Recent File List",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (history, _) = root.create_subkey(&history_path).unwrap();
        history.set_value("File2", &r"C:\Fixture\two.png").unwrap();
        history.set_value("File1", &r"C:\Fixture\one.png").unwrap();
        history.set_value("ViewMode", &42_u32).unwrap();

        let mut labels = Vec::new();
        let mut seen = BTreeSet::new();
        let mut revision = blake3::Hasher::new();
        collect_paint_records(&root, &history_path, &mut labels, &mut seen, &mut revision).unwrap();
        assert_eq!(labels, [r"C:\Fixture\one.png", r"C:\Fixture\two.png"]);

        clear_paint_root(&root, &history_path).unwrap();
        assert!(history.get_value::<u32, _>("ViewMode").is_ok());
        assert!(history.get_raw_value("File1").is_err());
        assert!(history.get_raw_value("File2").is_err());
        drop(history);
        let base = history_path
            .strip_suffix(r"\Recent File List")
            .expect("fixture path must contain the history leaf");
        root.delete_subkey_all(base).unwrap();
    }
}
