//! Plain JSON configuration, intentionally independent of OS credential stores.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use super::AiError;

const MAX_DOCUMENT_BYTES: u64 = 8192;

#[derive(Debug, Clone, Copy)]
enum FileStage {
    Open,
    Read,
    CreateDirectory,
    CreateTemporary,
    Write,
    Sync,
    Persist,
    Delete,
}

fn io_failure(stage: FileStage, error: std::io::Error) -> AiError {
    // Raw IO errors can include paths or custom messages. Retain only the
    // native code and classification, with the precise failing operation.
    log::warn!(
        "ai_configuration_io_failed stage={stage:?} kind={:?} os_code={:?}",
        error.kind(),
        error.raw_os_error()
    );
    AiError::ConfigurationUnavailable
}

fn path() -> Result<PathBuf, AiError> {
    crate::shared::application_paths()
        .map(|paths| paths.data_directory().join("ai.json"))
        .map_err(|_| {
            log::warn!("ai_configuration_path_unavailable");
            AiError::ConfigurationUnavailable
        })
}

pub(super) fn read() -> Result<Option<String>, AiError> {
    read_from(&path()?)
}
pub(super) fn write(document: &str) -> Result<(), AiError> {
    write_to(&path()?, document)
}
pub(super) fn delete() -> Result<(), AiError> {
    delete_from(&path()?)
}

fn read_from(path: &Path) -> Result<Option<String>, AiError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_failure(FileStage::Open, error)),
    };
    let mut document = String::new();
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_string(&mut document)
        .map_err(|error| io_failure(FileStage::Read, error))?;
    if document.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(AiError::InvalidConfiguration);
    }
    Ok(Some(document))
}

fn write_to(path: &Path, document: &str) -> Result<(), AiError> {
    if document.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(AiError::InvalidConfiguration);
    }
    let parent = path.parent().ok_or(AiError::ConfigurationUnavailable)?;
    fs::create_dir_all(parent).map_err(|error| io_failure(FileStage::CreateDirectory, error))?;
    // The temporary file is private (0600 on Unix) and lives beside the target.
    // Persist replaces an existing document on both Windows and macOS without
    // truncating the last valid configuration if writing fails.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| io_failure(FileStage::CreateTemporary, error))?;
    temporary
        .write_all(document.as_bytes())
        .map_err(|error| io_failure(FileStage::Write, error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| io_failure(FileStage::Sync, error))?;
    temporary
        .persist(path)
        .map_err(|error| io_failure(FileStage::Persist, error.error))?;
    Ok(())
}

fn delete_from(path: &Path) -> Result<(), AiError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_failure(FileStage::Delete, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_round_trips_replaces_and_deletes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data").join("ai.json");
        assert_eq!(read_from(&path).unwrap(), None);
        write_to(&path, r#"{"apiKey":"synthetic"}"#).unwrap();
        assert_eq!(
            read_from(&path).unwrap().unwrap(),
            r#"{"apiKey":"synthetic"}"#
        );
        write_to(&path, r#"{"apiKey":"replacement"}"#).unwrap();
        assert_eq!(
            read_from(&path).unwrap().unwrap(),
            r#"{"apiKey":"replacement"}"#
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        delete_from(&path).unwrap();
        delete_from(&path).unwrap();
        assert_eq!(read_from(&path).unwrap(), None);
    }

    #[test]
    fn invalid_writes_preserve_the_previous_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ai.json");
        write_to(&path, "original").unwrap();
        assert_eq!(
            write_to(&path, &"x".repeat(8193)),
            Err(AiError::InvalidConfiguration)
        );
        assert_eq!(read_from(&path).unwrap().as_deref(), Some("original"));
        fs::write(&path, "x".repeat(8193)).unwrap();
        assert_eq!(read_from(&path), Err(AiError::InvalidConfiguration));
        assert_eq!(
            read_from(directory.path()),
            Err(AiError::ConfigurationUnavailable)
        );
    }
}
