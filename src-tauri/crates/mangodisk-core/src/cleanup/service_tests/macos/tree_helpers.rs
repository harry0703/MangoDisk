    fn direct_directory_children(root: &Path) -> Vec<PathBuf> {
        if !root.is_dir() {
            return Vec::new();
        }
        let mut children = fs::read_dir(root)
            .expect("the dynamic partition root must be readable")
            .map(|entry| entry.expect("the partition entry must be readable").path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        children.sort();
        children
    }

    /// Hashes a real application tree without following symbolic links. QQLive
    /// component packages legitimately contain internal links, so the test
    /// records each link path and target as opaque bytes while never reading
    /// through it. A before/after digest therefore detects link replacement as
    /// well as regular-file changes without crossing the preserved boundary.
    fn digest_macos_tree_without_following_links(path: &Path) -> String {
        digest_macos_tree_with_exclusions_without_following_links(path, &[])
    }

    /// Applies the same opaque-link hashing policy while excluding fixed direct
    /// children owned by the cleanup rule. This is needed for mixed-purpose
    /// sandbox Library roots: the selected Caches child must not influence the
    /// preservation digest, while unrelated framework links are still covered.
    fn digest_macos_tree_with_exclusions_without_following_links(
        path: &Path,
        excluded_direct_children: &[&str],
    ) -> String {
        fn collect_entries(
            root: &Path,
            path: &Path,
            excluded_direct_children: &[&str],
            entries: &mut Vec<PathBuf>,
        ) {
            let metadata = fs::symlink_metadata(path)
                .expect("the preserved macOS application state must remain readable");
            entries.push(path.to_path_buf());
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return;
            }
            let mut children = fs::read_dir(path)
                .expect("the preserved application directory must remain readable")
                .map(|entry| entry.expect("the directory entry must be readable").path())
                .filter(|child| {
                    child.parent() != Some(root)
                        || !excluded_direct_children
                            .iter()
                            .any(|excluded| child.file_name().is_some_and(|name| name == *excluded))
                })
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                collect_entries(root, &child, excluded_direct_children, entries);
            }
        }

        let mut entries = Vec::new();
        collect_entries(path, path, excluded_direct_children, &mut entries);
        let mut hasher = Sha256::new();
        for entry in entries {
            let relative = entry.strip_prefix(path).unwrap_or(entry.as_path());
            hasher.update(relative.as_os_str().as_encoded_bytes());
            let metadata = fs::symlink_metadata(&entry)
                .expect("the preserved application entry must remain readable");
            if metadata.file_type().is_symlink() {
                hasher.update(b"symlink");
                let target = fs::read_link(&entry)
                    .expect("the preserved application link target must remain readable");
                hasher.update(target.as_os_str().as_encoded_bytes());
            } else if metadata.is_file() {
                hasher.update(b"file");
                hasher.update(
                    fs::read(&entry).expect("the preserved application file must remain readable"),
                );
            } else if metadata.is_dir() {
                hasher.update(b"directory");
            }
        }
        hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn digest_macos_tree(path: &Path, excluded_direct_children: &[&str]) -> String {
        fn collect_files(
            root: &Path,
            path: &Path,
            excluded_direct_children: &[&str],
            files: &mut Vec<PathBuf>,
        ) {
            let metadata = fs::symlink_metadata(path)
                .expect("the preserved macOS application state must remain readable");
            assert!(
                !metadata.file_type().is_symlink(),
                "the preserved application state must not cross a symbolic link"
            );
            if metadata.is_file() {
                files.push(path.to_path_buf());
                return;
            }
            // IDE profiles can retain a Unix-domain socket after shutdown.
            // Sockets and FIFOs have no durable byte content to hash and cannot
            // be opened as directories; ignore them while continuing to reject
            // symbolic links and hash every regular persisted file.
            if !metadata.is_dir() {
                return;
            }
            let mut children = fs::read_dir(path)
                .expect("the preserved application directory must remain readable")
                .map(|entry| entry.expect("the directory entry must be readable").path())
                .filter(|child| {
                    child.parent() != Some(root)
                        || !excluded_direct_children
                            .iter()
                            .any(|excluded| child.file_name().is_some_and(|name| name == *excluded))
                })
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                collect_files(root, &child, excluded_direct_children, files);
            }
        }

        let mut files = Vec::new();
        collect_files(path, path, excluded_direct_children, &mut files);
        let mut hasher = Sha256::new();
        for file in files {
            let relative = file.strip_prefix(path).unwrap_or(file.as_path());
            hasher.update(relative.as_os_str().as_encoded_bytes());
            hasher.update(
                fs::read(file).expect("the preserved application file must remain readable"),
            );
        }
        hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
    }
