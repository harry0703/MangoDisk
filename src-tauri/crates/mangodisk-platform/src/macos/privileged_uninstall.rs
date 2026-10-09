use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    MacosPrivilegedApplicationRemovalOutcome, Platform, PlatformError, PlatformErrorCode,
    PlatformResult,
};

use super::MacOsPlatform;

const OSASCRIPT_PATH: &str = "/usr/bin/osascript";
const AUTHORIZATION_SCRIPT: &str = include_str!("privileged_uninstall.applescript");
const SUCCESS_RESPONSE: &str = "mangodisk-privileged-remove-v1:completed";
const ERROR_RESPONSE_PREFIX: &str = "mangodisk-privileged-remove-v1:error:";
const USER_CANCELLED_APPLESCRIPT_ERROR: i32 = -128;
const ITEM_CHANGED_SHELL_STATUS: i32 = 42;
const RECOVERY_REQUIRED_SHELL_STATUS: i32 = 45;
const MAX_APPLICATION_CONTAINER_DEPTH: usize = 1;
const MAX_AUTHORIZATION_PROMPT_CHARS: usize = 120;
static NEXT_PRIVILEGED_STAGING_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhysicalIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivilegedCommandResponse {
    Completed,
    UserCancelled,
    ItemChanged,
    RecoveryRequired,
    Failed(Option<i32>),
}

/// Removes an application bundle through the macOS administrator prompt.
///
/// A root-owned bundle may reject an ordinary rename even when its parent is
/// writable. The complete identity check, same-volume staging rename, removal,
/// and checked restore therefore run inside one narrowly scoped root command.
/// Any state that may contain moved or partially deleted data is returned as a
/// typed recovery result instead of being collapsed into an ordinary failure.
pub(crate) fn remove_application_bundle_with_privileges(
    target: &Path,
    authorization_prompt: Option<&str>,
) -> PlatformResult<MacosPrivilegedApplicationRemovalOutcome> {
    validate_application_target(target)?;
    let authorization_prompt = validate_authorization_prompt(authorization_prompt)?;
    let expected_identity = physical_identity(target)?;
    let staging_root = privileged_staging_root(target)?;
    let staged_target = staging_root.join("target");
    let output = authorization_command(
        AUTHORIZATION_SCRIPT,
        target,
        &staging_root,
        &staged_target,
        expected_identity,
        authorization_prompt,
    )
    .output()
    .map_err(|error| PlatformError::io("launch administrator authorization", &error))?;

    if !output.status.success() {
        log::warn!(
            "macos_privileged_application_removal_failed reason=process_status status={}",
            output.status
        );
        return finalize_command_response(
            PrivilegedCommandResponse::Failed(output.status.code()),
            target,
            &staging_root,
            expected_identity,
        );
    }

    let response = match String::from_utf8(output.stdout) {
        Ok(response) => response,
        Err(_) => {
            log::warn!("macos_privileged_application_removal_failed reason=invalid_process_output");
            return finalize_command_response(
                PrivilegedCommandResponse::Failed(None),
                target,
                &staging_root,
                expected_identity,
            );
        }
    };
    finalize_command_response(
        parse_command_response(response.trim()),
        target,
        &staging_root,
        expected_identity,
    )
}

/// Reports whether the privileged boundary accepts the path shape.
///
/// Core uses the same predicate before advertising administrator-authorized
/// removal. Keeping this check in the platform layer prevents the catalog and
/// executor from drifting apart as supported macOS installation roots evolve.
pub(crate) fn application_target_is_supported(target: &Path) -> bool {
    match dirs::home_dir() {
        Some(home) => application_target_is_supported_with_home(target, &home),
        None => {
            has_bundle_extension(target)
                && application_target_is_below_root(target, Path::new("/Applications"))
        }
    }
}

fn application_target_is_supported_with_home(target: &Path, home: &Path) -> bool {
    has_bundle_extension(target)
        && (application_target_is_below_root(target, Path::new("/Applications"))
            || application_target_is_below_root(target, &home.join("Applications")))
}

fn has_bundle_extension(target: &Path) -> bool {
    target
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

fn application_target_is_below_root(target: &Path, root: &Path) -> bool {
    let Ok(relative) = target.strip_prefix(root) else {
        return false;
    };
    let Some(parent) = relative.parent() else {
        return false;
    };
    parent
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        && parent.components().count() <= MAX_APPLICATION_CONTAINER_DEPTH
}

fn validate_application_target(target: &Path) -> PlatformResult<()> {
    if !application_target_is_supported(target) {
        return Err(PlatformError::invalid_path(
            "privileged removal target is outside a supported application root",
        ));
    }
    let metadata = fs::symlink_metadata(target)
        .map_err(|error| PlatformError::io("read privileged removal target", &error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(PlatformError::invalid_path(
            "privileged removal target is not a physical directory",
        ));
    }
    MacOsPlatform.validate_path_no_links(target)
}

fn validate_authorization_prompt(prompt: Option<&str>) -> PlatformResult<Option<&str>> {
    let Some(prompt) = prompt.map(str::trim) else {
        return Ok(None);
    };
    if prompt.is_empty()
        || prompt.chars().count() > MAX_AUTHORIZATION_PROMPT_CHARS
        || prompt.chars().any(char::is_control)
    {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "administrator authorization prompt is invalid",
        ));
    }
    Ok(Some(prompt))
}

fn authorization_command(
    script: &str,
    target: &Path,
    staging_root: &Path,
    staged_target: &Path,
    expected_identity: PhysicalIdentity,
    authorization_prompt: Option<&str>,
) -> Command {
    let mut command = Command::new(OSASCRIPT_PATH);
    command
        .args(["-e", script, "--"])
        .arg(target)
        .arg(staging_root)
        .arg(staged_target)
        .arg(format!(
            "{}:{}",
            expected_identity.device, expected_identity.inode
        ))
        // Dynamic values remain process arguments. None of them are inserted
        // into AppleScript source, and the script shell-quotes every value that
        // crosses into the privileged command.
        .arg(authorization_prompt.unwrap_or_default())
        .arg(ITEM_CHANGED_SHELL_STATUS.to_string())
        .arg(RECOVERY_REQUIRED_SHELL_STATUS.to_string())
        .arg(SUCCESS_RESPONSE)
        .arg(ERROR_RESPONSE_PREFIX);
    command
}

fn parse_command_response(response: &str) -> PrivilegedCommandResponse {
    if response == SUCCESS_RESPONSE {
        return PrivilegedCommandResponse::Completed;
    }
    let error_number = response
        .strip_prefix(ERROR_RESPONSE_PREFIX)
        .and_then(|code| code.parse::<i32>().ok());
    match error_number {
        Some(USER_CANCELLED_APPLESCRIPT_ERROR) => PrivilegedCommandResponse::UserCancelled,
        Some(ITEM_CHANGED_SHELL_STATUS) => PrivilegedCommandResponse::ItemChanged,
        Some(RECOVERY_REQUIRED_SHELL_STATUS) => PrivilegedCommandResponse::RecoveryRequired,
        status => PrivilegedCommandResponse::Failed(status),
    }
}

fn finalize_command_response(
    response: PrivilegedCommandResponse,
    target: &Path,
    staging_root: &Path,
    expected_identity: PhysicalIdentity,
) -> PlatformResult<MacosPrivilegedApplicationRemovalOutcome> {
    let (Ok(target_present), Ok(staging_present)) =
        (path_is_present(target), path_is_present(staging_root))
    else {
        log::error!("macos_privileged_application_removal_failed reason=postcondition_unavailable");
        return Ok(MacosPrivilegedApplicationRemovalOutcome::RecoveryRequired);
    };
    match response {
        PrivilegedCommandResponse::Completed if !target_present && !staging_present => {
            log::info!("macos_privileged_application_removal_completed");
            Ok(MacosPrivilegedApplicationRemovalOutcome::Completed)
        }
        PrivilegedCommandResponse::UserCancelled
            if !staging_present && physical_identity(target).ok() == Some(expected_identity) =>
        {
            log::info!("macos_privileged_application_removal_cancelled");
            Ok(MacosPrivilegedApplicationRemovalOutcome::UserCancelled)
        }
        PrivilegedCommandResponse::ItemChanged if target_present && !staging_present => {
            log::warn!("macos_privileged_application_removal_stopped reason=item_changed");
            Ok(MacosPrivilegedApplicationRemovalOutcome::ItemChanged)
        }
        PrivilegedCommandResponse::Failed(status) if target_present && !staging_present => {
            log::warn!(
                "macos_privileged_application_removal_failed reason=shell_status shell_status={}",
                status.unwrap_or(-1)
            );
            Err(PlatformError::operation_failed(
                "administrator-authorized application removal failed",
            ))
        }
        _ => {
            log::error!(
                "macos_privileged_application_removal_failed reason=recovery_required target_present={target_present} staging_present={staging_present}"
            );
            Ok(MacosPrivilegedApplicationRemovalOutcome::RecoveryRequired)
        }
    }
}

fn physical_identity(path: &Path) -> PlatformResult<PhysicalIdentity> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| PlatformError::io("capture privileged removal identity", &error))?;
    Ok(PhysicalIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn path_is_present(path: &Path) -> PlatformResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(PlatformError::io(
            "inspect privileged removal postcondition",
            &error,
        )),
    }
}

fn privileged_staging_root(target: &Path) -> PlatformResult<PathBuf> {
    let parent = target.parent().ok_or_else(|| {
        PlatformError::invalid_path("privileged removal application parent is unavailable")
    })?;
    let id = NEXT_PRIVILEGED_STAGING_ID.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".mangodisk-delete-{}-{id}", std::process::id())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_roots_and_case_insensitive_bundle_extensions_stay_aligned() {
        let home = Path::new("/Users/example");

        assert!(application_target_is_supported_with_home(
            Path::new("/Applications/Example.APP"),
            home
        ));
        assert!(application_target_is_supported_with_home(
            Path::new("/Users/example/Applications/Example.app"),
            home
        ));
        assert!(!application_target_is_supported_with_home(
            Path::new("/tmp/Example.app"),
            home
        ));
        assert!(!application_target_is_supported_with_home(
            Path::new("/Applications/A/B/Example.app"),
            home
        ));
    }

    #[test]
    fn command_response_preserves_cancel_change_and_recovery_states() {
        assert_eq!(
            parse_command_response("mangodisk-privileged-remove-v1:error:-128"),
            PrivilegedCommandResponse::UserCancelled
        );
        assert_eq!(
            parse_command_response("mangodisk-privileged-remove-v1:error:42"),
            PrivilegedCommandResponse::ItemChanged
        );
        assert_eq!(
            parse_command_response("mangodisk-privileged-remove-v1:error:45"),
            PrivilegedCommandResponse::RecoveryRequired
        );
    }

    #[test]
    fn authorization_prompt_is_bounded_and_passed_as_script_data() {
        assert_eq!(
            validate_authorization_prompt(Some("  MangoDisk needs approval.  "))
                .expect("a concise prompt should be accepted"),
            Some("MangoDisk needs approval.")
        );
        assert!(validate_authorization_prompt(Some("\n")).is_err());
        assert!(validate_authorization_prompt(Some("line one\nline two")).is_err());
        assert!(validate_authorization_prompt(Some(
            &"a".repeat(MAX_AUTHORIZATION_PROMPT_CHARS + 1)
        ))
        .is_err());

        let script = AUTHORIZATION_SCRIPT;
        assert!(script.contains("set promptText to item 5 of argv"));
        assert!(script.contains(
            "do shell script removeCommand with prompt promptText with administrator privileges"
        ));
        assert!(!script.contains("MangoDisk needs approval."));
    }

    #[test]
    fn authorization_script_compiles_with_macos_tool() {
        let compiled_script = std::env::temp_dir().join(format!(
            "mangodisk-authorization-script-{}-{}.scpt",
            std::process::id(),
            NEXT_PRIVILEGED_STAGING_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let output = Command::new("/usr/bin/osacompile")
            .arg("-e")
            .arg(AUTHORIZATION_SCRIPT)
            .arg("-o")
            .arg(&compiled_script)
            .output()
            .expect("launch the macOS AppleScript compiler");
        let diagnostic = String::from_utf8_lossy(&output.stderr).into_owned();
        let _ = fs::remove_file(compiled_script);

        assert!(
            output.status.success(),
            "authorization script should compile: {diagnostic}"
        );
    }

    #[test]
    fn authorization_transaction_handles_unicode_quotes_and_shell_characters() {
        let root = disposable_root("quoted-path");
        let target = root.join(
            "\u{4e2d}\u{6587} \u{65e5}\u{672c}\u{8a9e} \u{1f600} space 'single' \"double\" back\\slash ; $() `tick` [glob] tab\treturn\rline\nbreak.app",
        );
        let staging_root = root.join("staging");
        let staged_target = staging_root.join("target");
        fs::create_dir_all(target.join("Contents"))
            .expect("create the complex application path fixture");
        fs::write(target.join("Contents/payload.txt"), b"fixture")
            .expect("create the application payload fixture");
        let expected_identity = physical_identity(&target).expect("capture the fixture identity");

        // The production script stays byte-for-byte identical except for the
        // authorization clause. This lets the test execute the real staging,
        // identity, deletion, and cleanup transaction inside an owned temporary
        // directory without displaying a system prompt or gaining privileges.
        assert_eq!(
            AUTHORIZATION_SCRIPT
                .matches(" with administrator privileges")
                .count(),
            2
        );
        let test_script = AUTHORIZATION_SCRIPT.replace(" with administrator privileges", "");
        let output = authorization_command(
            &test_script,
            &target,
            &staging_root,
            &staged_target,
            expected_identity,
            None,
        )
        .output()
        .expect("execute the non-privileged transaction fixture");
        let response = String::from_utf8(output.stdout)
            .expect("the transaction fixture should return UTF-8 output");
        let parsed_response = parse_command_response(response.trim());
        let target_present = path_is_present(&target).expect("inspect the target postcondition");
        let staging_present =
            path_is_present(&staging_root).expect("inspect the staging postcondition");
        let diagnostic = String::from_utf8_lossy(&output.stderr).into_owned();
        fs::remove_dir_all(&root).expect("remove the complex path fixture root");

        assert!(
            output.status.success(),
            "transaction script should execute: {diagnostic}"
        );
        assert_eq!(parsed_response, PrivilegedCommandResponse::Completed);
        assert!(!target_present);
        assert!(!staging_present);
    }

    #[test]
    fn completed_response_requires_both_paths_to_be_absent() {
        let root = disposable_root("completed");
        let target = root.join("Example.app");
        let staging = root.join("staging");

        assert_eq!(
            finalize_command_response(
                PrivilegedCommandResponse::Completed,
                &target,
                &staging,
                PhysicalIdentity {
                    device: 0,
                    inode: 0,
                },
            )
            .expect("absent postconditions should complete"),
            MacosPrivilegedApplicationRemovalOutcome::Completed
        );
    }

    #[test]
    fn cancellation_requires_the_original_identity_and_no_staging_data() {
        let root = disposable_root("cancelled");
        let target = root.join("Example.app");
        fs::create_dir_all(&target).expect("create the target fixture");
        let expected_identity = physical_identity(&target).expect("capture the target identity");

        assert_eq!(
            finalize_command_response(
                PrivilegedCommandResponse::UserCancelled,
                &target,
                &root.join("staging"),
                expected_identity,
            )
            .expect("an unchanged target should remain cancelled"),
            MacosPrivilegedApplicationRemovalOutcome::UserCancelled
        );

        fs::rename(&target, root.join("Original.app"))
            .expect("retain the original target identity");
        fs::create_dir(&target).expect("create a replacement target");
        assert_eq!(
            finalize_command_response(
                PrivilegedCommandResponse::UserCancelled,
                &target,
                &root.join("staging"),
                expected_identity,
            )
            .expect("a changed target should require recovery review"),
            MacosPrivilegedApplicationRemovalOutcome::RecoveryRequired
        );
        fs::remove_dir_all(root).expect("remove the cancellation fixture");
    }

    #[test]
    fn item_change_requires_the_staging_directory_to_be_absent() {
        let root = disposable_root("item-changed");
        let target = root.join("Example.app");
        let staging = root.join("staging");
        fs::create_dir_all(&target).expect("create the changed target fixture");
        let expected_identity = physical_identity(&target).expect("capture the target identity");

        assert_eq!(
            finalize_command_response(
                PrivilegedCommandResponse::ItemChanged,
                &target,
                &staging,
                expected_identity,
            )
            .expect("a restored changed item should keep its typed outcome"),
            MacosPrivilegedApplicationRemovalOutcome::ItemChanged
        );
        fs::create_dir(&staging).expect("create leftover staging state");
        assert_eq!(
            finalize_command_response(
                PrivilegedCommandResponse::ItemChanged,
                &target,
                &staging,
                expected_identity,
            )
            .expect("leftover staging state should require recovery"),
            MacosPrivilegedApplicationRemovalOutcome::RecoveryRequired
        );
        fs::remove_dir_all(root).expect("remove the item-change fixture");
    }

    #[test]
    fn any_staged_data_escalates_an_ordinary_failure_to_recovery_required() {
        let root = disposable_root("recovery");
        let target = root.join("Example.app");
        let staging = root.join("staging");
        fs::create_dir_all(&staging).expect("create the staged-data fixture");

        assert_eq!(
            finalize_command_response(
                PrivilegedCommandResponse::Failed(Some(43)),
                &target,
                &staging,
                PhysicalIdentity {
                    device: 0,
                    inode: 0,
                },
            )
            .expect("staged data should produce a typed recovery result"),
            MacosPrivilegedApplicationRemovalOutcome::RecoveryRequired
        );
        fs::remove_dir_all(root).expect("remove the staged-data fixture");
    }

    #[test]
    fn privileged_staging_stays_next_to_the_application() {
        let target = Path::new("/Applications/Utilities/Example.app");
        let staging_root = privileged_staging_root(target).expect("build the staging path");

        assert_eq!(staging_root.parent(), target.parent());
        assert!(staging_root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".mangodisk-delete-")));
    }

    fn disposable_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mangodisk-privileged-uninstall-{label}-{}-{}",
            std::process::id(),
            NEXT_PRIVILEGED_STAGING_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
