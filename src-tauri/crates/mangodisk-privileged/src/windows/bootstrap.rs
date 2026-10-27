//! User-scoped bootstrap staging shared by installed and portable desktop builds.
use super::{deployment, security};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{fs::MetadataExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::ERROR_SHARING_VIOLATION,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
};

const MAXIMUM_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct Bootstrap {
    path: PathBuf,
    // Keep directories, the operation lock, and verified bytes pinned until the
    // elevated child exits. Another instance must not replace an active image.
    _directories: Vec<File>,
    _operation: File,
    _image: File,
}
impl Bootstrap {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) fn prepare(bytes: &[u8]) -> io::Result<Bootstrap> {
    let root = security::local_app_data()?;
    prepare_in(&root, bytes).inspect_err(|error| {
        super::diagnostics::error(format!(
            "bootstrap preparation failed path={} native_code={:?} error={error}",
            root.join("MangoDisk/privileged").display(),
            error.raw_os_error()
        ));
    })
}

fn prepare_in(root: &Path, bytes: &[u8]) -> io::Result<Bootstrap> {
    if bytes.is_empty() || bytes.len() > MAXIMUM_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "embedded bootstrap exceeds installation bounds",
        ));
    }
    let mut directories = vec![pin_directory(root, FILE_SHARE_READ)?];
    let application = root.join("MangoDisk");
    let directory = application.join("privileged");
    for path in [&application, &directory] {
        match fs::create_dir(path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        directories.push(pin_directory(path, FILE_SHARE_READ)?);
    }
    let operation = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(directory.join("bootstrap.lock"))
        .map_err(|error| {
            if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) {
                io::Error::new(io::ErrorKind::WouldBlock, error)
            } else {
                error
            }
        })?;
    reject_reparse(&operation)?;
    // Windows rename needs a writable parent-directory handle. Each directory
    // now contains a pinned child (the next directory or bootstrap.lock), so it
    // cannot become empty and accept a junction. Acquire the replacement pins
    // before releasing the write-denying handles to leave no unprotected gap.
    directories = [root, &application, &directory]
        .into_iter()
        .map(|path| pin_directory(path, FILE_SHARE_READ | FILE_SHARE_WRITE))
        .collect::<io::Result<Vec<_>>>()?;
    let path = directory.join(deployment::EXECUTABLE_NAME);
    let existing = match open_image(&path) {
        Ok(mut file) => {
            if matches_bytes(&mut file, bytes)? {
                Some(file)
            } else {
                None
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut image = if let Some(file) = existing {
        file
    } else {
        let pending = directory.join("MangoDiskPrivileged.exe.next");
        // Only this fixed staging file is disposable. Never truncate an existing
        // file: it might be a hard link to another user-owned object.
        match fs::symlink_metadata(&pending) {
            Ok(metadata) if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "bootstrap staging file is a reparse point",
                ));
            }
            Ok(_) => fs::remove_file(&pending)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&pending, &path)?;
        open_image(&path)?
    };
    // Check the bytes through the pinned handle before requesting UAC, including
    // reused files. A valid signature alone does not identify this app's helper.
    if !matches_bytes(&mut image, bytes)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "staged bootstrap differs from the embedded image",
        ));
    }
    if !cfg!(debug_assertions) {
        deployment::verify_signature_open(&path, &image)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
    Ok(Bootstrap {
        path,
        _directories: directories,
        _operation: operation,
        _image: image,
    })
}

fn pin_directory(path: &Path, share_mode: u32) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(share_mode)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    reject_reparse(&file)?;
    if !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "bootstrap parent is not a directory",
        ));
    }
    Ok(file)
}

fn open_image(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    reject_reparse(&file)?;
    Ok(file)
}

fn reject_reparse(file: &File) -> io::Result<()> {
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "bootstrap path is a reparse point",
        ));
    }
    Ok(())
}

fn matches_bytes(file: &mut File, bytes: &[u8]) -> io::Result<bool> {
    use std::io::{Seek, SeekFrom};
    if file.metadata()?.len() != bytes.len() as u64 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut actual = Vec::with_capacity(bytes.len());
    file.take(MAXIMUM_BYTES as u64 + 1)
        .read_to_end(&mut actual)?;
    Ok(actual == bytes)
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn stages_in_fixed_user_directory_and_reuses_identical_bytes() {
        let root = tempfile::tempdir().unwrap();
        let bootstrap = prepare_in(root.path(), b"embedded helper").unwrap();
        let path = root
            .path()
            .join("MangoDisk/privileged/MangoDiskPrivileged.exe");
        assert_eq!(bootstrap.path(), path);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        drop(bootstrap);
        let bootstrap = prepare_in(root.path(), b"embedded helper").unwrap();
        assert_eq!(fs::read(bootstrap.path()).unwrap(), b"embedded helper");
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn replaces_corrupt_or_previous_bytes_without_modifying_hard_link_target() {
        let root = tempfile::tempdir().unwrap();
        let bootstrap = prepare_in(root.path(), b"previous version").unwrap();
        let path = bootstrap.path().to_path_buf();
        drop(bootstrap);
        let unrelated = root.path().join("unrelated");
        fs::hard_link(&path, &unrelated).unwrap();
        let bootstrap = prepare_in(root.path(), b"next version").unwrap();
        assert_eq!(fs::read(bootstrap.path()).unwrap(), b"next version");
        assert_eq!(fs::read(unrelated).unwrap(), b"previous version");
        drop(bootstrap);
        fs::write(&path, b"fake version").unwrap();
        let bootstrap = prepare_in(root.path(), b"next version").unwrap();
        assert_eq!(fs::read(bootstrap.path()).unwrap(), b"next version");
    }

    #[test]
    fn active_bootstrap_blocks_other_instances_and_image_replacement() {
        let root = tempfile::tempdir().unwrap();
        let bootstrap = prepare_in(root.path(), b"embedded helper").unwrap();
        assert_eq!(
            prepare_in(root.path(), b"next version")
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(fs::write(bootstrap.path(), b"tampered").is_err());
        assert!(fs::remove_file(bootstrap.path()).is_err());
        assert!(
            fs::remove_file(bootstrap.path().parent().unwrap().join("bootstrap.lock")).is_err()
        );
        assert!(fs::rename(
            bootstrap.path().parent().unwrap(),
            root.path().join("moved")
        )
        .is_err());
    }

    #[test]
    fn pinned_empty_directory_rejects_mutation_handles() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("empty");
        fs::create_dir(&directory).unwrap();
        let pinned = pin_directory(&directory, FILE_SHARE_READ).unwrap();
        // A junction can be set on an empty directory without renaming it.
        // This models the interval before bootstrap.lock has been created.
        let mutation = OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&directory);
        assert_eq!(
            mutation
                .expect_err("a pinned directory must reject writes")
                .raw_os_error(),
            Some(ERROR_SHARING_VIOLATION as i32)
        );
        // Pinning the directory must still permit ordinary child-file creation.
        fs::write(directory.join("child"), b"unchanged child access").unwrap();
        drop(pinned);
    }

    #[test]
    fn recovers_interrupted_staging_without_touching_other_files() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("MangoDisk/privileged");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("MangoDiskPrivileged.exe.next"), b"partial").unwrap();
        fs::write(directory.join("unrelated"), b"untouched").unwrap();
        let bootstrap = prepare_in(root.path(), b"embedded helper").unwrap();
        assert_eq!(fs::read(bootstrap.path()).unwrap(), b"embedded helper");
        assert_eq!(fs::read(directory.join("unrelated")).unwrap(), b"untouched");
        assert!(!directory.join("MangoDiskPrivileged.exe.next").exists());
    }

    #[test]
    fn rejects_junction_before_writing_to_redirected_directory() {
        let root = tempfile::tempdir().unwrap();
        let redirected = tempfile::tempdir().unwrap();
        let application = root.path().join("MangoDisk");
        fs::create_dir(&application).unwrap();
        let junction = application.join("privileged");
        assert!(std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(redirected.path())
            .output()
            .unwrap()
            .status
            .success());
        assert_eq!(
            prepare_in(root.path(), b"embedded helper")
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(fs::read_dir(redirected.path()).unwrap().count(), 0);
        fs::remove_dir(junction).unwrap();
    }

    #[test]
    fn rejects_empty_and_oversized_payloads_before_creating_directories() {
        let root = tempfile::tempdir().unwrap();
        for bytes in [&[][..], &vec![0; MAXIMUM_BYTES + 1]] {
            assert_eq!(
                prepare_in(root.path(), bytes).err().unwrap().kind(),
                io::ErrorKind::InvalidData
            );
        }
        assert!(!root.path().join("MangoDisk").exists());
    }

    #[test]
    fn authenticode_rejects_an_unsigned_staged_image() {
        let root = tempfile::tempdir().unwrap();
        let bootstrap = prepare_in(root.path(), b"unsigned image").unwrap();
        assert!(deployment::verify_signature_open(bootstrap.path(), &bootstrap._image).is_err());
    }
}
