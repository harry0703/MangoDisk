//! Discover only positively identified Codex linked worktrees. A directory name
//! or its location below `.codex` never authorizes deleting project contents.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use mangodisk_platform::{current_platform, Platform};

use crate::{
    applications::catalog::ProcessSnapshot,
    filesystem::metadata::is_link_like,
    shared::{CoreError, CoreResult},
};

const MAX_WORKTREES: usize = 256;
const MAX_MARKER_BYTES: u64 = 16 * 1024;

pub(super) fn home() -> CoreResult<PathBuf> {
    let path = match std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        Some(value) => PathBuf::from(value),
        None => current_platform()
            .user_directories()?
            .home_directory()
            .join(".codex"),
    };
    if !path.is_absolute() || path.parent().is_none() {
        return Err(CoreError::invalid_input(
            "Codex home must be an absolute data directory",
        ));
    }
    Ok(path)
}

pub(super) fn discover(home: &Path, is_cancelled: &dyn Fn() -> bool) -> CoreResult<Vec<PathBuf>> {
    match fs::symlink_metadata(home) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(discovery_error(error)),
        Ok(metadata) if !metadata.is_dir() || is_link_like(&metadata) => {
            return Err(CoreError::invalid_input(
                "Codex home must be an unlinked directory",
            ));
        }
        Ok(_) => {}
    }
    // A custom CODEX_HOME may itself be a regular directory below a junction.
    // Validate ancestors before resolving paths so canonicalization cannot hide
    // that redirection. The platform still permits its fixed system aliases.
    current_platform()
        .validate_path_no_links(home)
        .map_err(|_| CoreError::invalid_input("Codex home has a linked ancestor"))?;
    let roots = [home.join("worktrees")];
    let mut found: Vec<PathBuf> = Vec::new();
    let mut remaining = 4096_usize;
    for root in roots {
        // Inspect only the documented worktrees/ID/checkout layout. Other
        // Codex data, including sessions and credentials, is never traversed.
        let mut pending = vec![(root, 0)];
        while let Some((path, depth)) = pending.pop() {
            if is_cancelled() {
                return Ok(Vec::new());
            }
            if remaining == 0 {
                return Err(CoreError::invalid_input(
                    "Codex worktree discovery exceeded its entry limit",
                ));
            }
            remaining -= 1;
            if !real_directory(&path) {
                continue;
            }
            if depth == 2 && is_linked_checkout(&path) {
                let canonical = current_platform()
                    .canonicalize_no_links(&path)
                    .map_err(|_| {
                        CoreError::invalid_input("Codex checkout has a linked ancestor")
                    })?;
                if !found
                    .iter()
                    .any(|item| current_platform().paths_equal(item, &canonical))
                {
                    found.push(canonical);
                }
                if found.len() > MAX_WORKTREES {
                    return Err(CoreError::invalid_input(
                        "Codex worktree discovery exceeded its limit",
                    ));
                }
            } else if depth < 2 {
                let children = fs::read_dir(&path).map_err(discovery_error)?;
                for (index, entry) in children.enumerate() {
                    if index >= 512 {
                        return Err(CoreError::invalid_input(
                            "Codex worktree location has too many entries",
                        ));
                    }
                    pending.push((entry.map_err(discovery_error)?.path(), depth + 1));
                }
            }
        }
    }
    Ok(found)
}

fn discovery_error(error: std::io::Error) -> CoreError {
    log::warn!(
        "codex_worktree_discovery_failed error_kind={:?}",
        error.kind()
    );
    CoreError::operation_failed("Codex worktree discovery is unavailable")
}

/// Shared by preview, execution, and the user-confirmed application close flow.
/// Global runtimes cannot be attributed to a checkout, so they must not block
/// worktree cleanup or become targets of a broad process termination request.
pub(super) fn application_process_names() -> Vec<String> {
    ["Codex", "ChatGPT"].map(String::from).to_vec()
}

pub(super) fn blocking_processes() -> Result<Vec<String>, String> {
    let names = application_process_names();
    ProcessSnapshot::capture().map(|snapshot| snapshot.matching_processes(&names))
}

pub(super) fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir() && !is_link_like(&metadata))
}

/// Protection is independent of successful discovery: a failed enumeration or
/// a missing ownership marker must not turn configured worktrees into ordinary
/// projects. Location only requires protection; deletion still requires valid
/// reciprocal ownership. Verified checkouts outside the current home retain
/// the same protection when found by native discovery or the project index.
pub(super) fn protected_checkout(project: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(home) = home {
        let worktrees = home.join("worktrees");
        if let Some(relative) = current_platform().relative_path(project, &worktrees) {
            return Some(worktrees.join(relative.components().take(2).collect::<PathBuf>()));
        }
    }
    project
        .ancestors()
        .find(|path| is_managed_checkout(path))
        .map(Path::to_path_buf)
}

fn small_file(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || is_link_like(&metadata) || metadata.len() > MAX_MARKER_BYTES {
        return None;
    }
    let mut contents = String::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_MARKER_BYTES + 1)
        .read_to_string(&mut contents)
        .ok()?;
    (contents.len() as u64 <= MAX_MARKER_BYTES).then_some(contents)
}

// Outside the configured worktree location, require additional Codex evidence
// before applying Codex-specific process protection to an ordinary Git checkout.
fn is_managed_checkout(path: &Path) -> bool {
    let Some(admin) = linked_checkout_admin(path) else {
        return false;
    };
    small_file(&admin.join("codex-thread.json"))
        .and_then(|marker| serde_json::from_str::<serde_json::Value>(&marker).ok())
        .is_some_and(|marker| {
            marker
                .get("ownerThreadId")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| !id.is_empty())
        })
}

/// Configured worktrees are identified by location and reciprocal Git pointers,
/// not optional Codex metadata whose presence varies between installations.
/// Execution uses the same Git validation before touching specific artifacts.
pub(super) fn is_linked_checkout(path: &Path) -> bool {
    linked_checkout_admin(path).is_some()
}

fn linked_checkout_admin(path: &Path) -> Option<PathBuf> {
    let pointer = small_file(&path.join(".git"))?;
    let gitdir = pointer.trim().strip_prefix("gitdir: ")?;
    let admin = path.join(gitdir);
    if !real_directory(&admin)
        || admin
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|name| name != "worktrees")
    {
        return None;
    }
    current_platform().validate_path_no_links(path).ok()?;
    current_platform().validate_path_no_links(&admin).ok()?;
    // The reciprocal Git pointer prevents a copied `.git` file from lending
    // another checkout's provenance to an unrelated directory.
    let back_pointer = small_file(&admin.join("gitdir"))?;
    let left = current_platform()
        .canonicalize_no_links(Path::new(back_pointer.trim()))
        .ok()?;
    let right = current_platform()
        .canonicalize_no_links(&path.join(".git"))
        .ok()?;
    current_platform()
        .paths_equal(&left, &right)
        .then_some(admin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_codex_applications_are_related_to_worktree_cleanup() {
        let runtimes = ProcessSnapshot::from_process_names(
            [
                "node", "python", "python3", "cargo", "rustc", "java", "dotnet", "cmake", "make",
                "ninja",
            ]
            .map(String::from)
            .to_vec(),
        );
        assert!(runtimes
            .matching_processes(&application_process_names())
            .is_empty());
        let apps = ProcessSnapshot::from_process_names(vec![
            "Codex.exe".into(),
            "ChatGPT.exe".into(),
            "node.exe".into(),
        ]);
        assert_eq!(
            apps.matching_processes(&application_process_names()),
            vec!["Codex", "ChatGPT"]
        );
    }

    #[test]
    fn configured_home_is_absolute_or_rejected() {
        if let Ok(home) = home() {
            assert!(home.is_absolute());
            assert!(home.parent().is_some());
        }
    }

    #[test]
    fn protection_uses_the_worktree_boundary_without_authorizing_unknown_checkouts() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path();
        let checkout = home.join("worktrees/1234/project");
        assert_eq!(
            protected_checkout(&checkout.join("nested/package"), Some(home)),
            Some(checkout.clone())
        );
        assert!(!is_managed_checkout(&checkout));
        for relative in [
            "worktrees-other/project",
            "sessions/project",
            "ordinary/project",
        ] {
            assert!(protected_checkout(&home.join(relative), Some(home)).is_none());
        }
    }

    #[test]
    fn discovery_requires_reciprocal_codex_ownership_and_does_not_visit_sessions() {
        let fixture = tempfile::tempdir().expect("fixture must exist");
        let checkout = fixture.path().join("worktrees/1234/project");
        let admin = fixture.path().join("repository/.git/worktrees/project");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&admin).unwrap();
        fs::write(
            checkout.join(".git"),
            format!("gitdir: {}", admin.display()),
        )
        .unwrap();
        fs::write(
            admin.join("gitdir"),
            checkout.join(".git").to_str().unwrap(),
        )
        .unwrap();
        assert!(is_linked_checkout(&checkout));
        assert!(!is_managed_checkout(&checkout));
        assert_eq!(
            discover(fixture.path(), &|| false).unwrap(),
            vec![fs::canonicalize(&checkout).unwrap()]
        );
        fs::write(
            admin.join("codex-thread.json"),
            r#"{"ownerThreadId":"fixture"}"#,
        )
        .unwrap();
        fs::create_dir_all(fixture.path().join("sessions/project/target")).unwrap();
        fs::write(
            fixture.path().join("sessions/project/Cargo.toml"),
            "[package]",
        )
        .unwrap();
        assert_eq!(
            discover(fixture.path(), &|| false).unwrap(),
            vec![fs::canonicalize(&checkout).unwrap()]
        );
        assert!(discover(fixture.path(), &|| true).unwrap().is_empty());
        // Optional metadata cannot veto a valid checkout within the configured
        // location, including installations that do not create this file.
        fs::write(admin.join("codex-thread.json"), "invalid optional metadata").unwrap();
        assert!(!is_managed_checkout(&checkout));
        assert_eq!(discover(fixture.path(), &|| false).unwrap().len(), 1);
        fs::write(admin.join("gitdir"), admin.join("wrong").to_str().unwrap()).unwrap();
        assert!(!is_linked_checkout(&checkout));
        assert!(discover(fixture.path(), &|| false).unwrap().is_empty());
    }

    #[test]
    fn malformed_and_oversized_markers_are_rejected() {
        let fixture = tempfile::tempdir().unwrap();
        let marker = fixture.path().join("marker");
        fs::write(&marker, vec![b'x'; MAX_MARKER_BYTES as usize + 1]).unwrap();
        assert!(small_file(&marker).is_none());
        fs::write(fixture.path().join(".git"), "not a linked checkout").unwrap();
        assert!(!is_managed_checkout(fixture.path()));
    }

    #[test]
    fn linked_home_or_worktree_container_is_not_followed() {
        let fixture = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        directory_link(outside.path(), &fixture.path().join("worktrees"));
        assert!(discover(fixture.path(), &|| false).unwrap().is_empty());
        let linked_home = fixture.path().join("linked-home");
        directory_link(outside.path(), &linked_home);
        assert!(discover(&linked_home, &|| false).is_err());
    }

    #[test]
    fn linked_ancestor_of_custom_codex_home_is_rejected() {
        let fixture = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join("codex")).unwrap();
        let link = fixture.path().join("linked-parent");
        directory_link(outside.path(), &link);
        assert!(discover(&link.join("codex"), &|| false).is_err());
    }

    #[cfg(unix)]
    fn directory_link(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[cfg(windows)]
    fn directory_link(target: &Path, link: &Path) {
        // Junctions exercise the Windows reparse-point policy without requiring
        // elevation or enabling Developer Mode on the test machine.
        let output = std::process::Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(output.status.success(), "test junction must be created");
    }
}
