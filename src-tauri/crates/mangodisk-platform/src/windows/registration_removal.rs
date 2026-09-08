//! Removes an explicitly selected uninstall record and its owned registry children.
//! Registry transactions keep revalidation and deletion atomic across registry views. No path,
//! registry key, command, or file-deletion target is accepted from the desktop or elevated caller.

use std::{io, rc::Rc};

use winreg::{enums::*, transaction::Transaction, RegKey};

use crate::{
    application_uninstall_diagnostic_id, ApplicationInstallScope, PlatformError, PlatformErrorCode,
    PlatformResult,
};

#[path = "registration_removal/helper.rs"]
mod helper;
pub use helper::run_application_record_helper_mode;

const UNINSTALL_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const CHANGED: i32 = 1306;
const UNSUPPORTED: i32 = 50;

#[derive(Clone, Copy, Debug)]
#[repr(u32)]
enum Stage {
    Arguments = 1,
    Locate = 2,
    Inspect = 3,
    Delete = 4,
    Commit = 5,
    Verify = 6,
    Elevate = 7,
    Snapshot = 8,
    Scope = 9,
}

#[derive(Debug)]
struct Failure {
    stage: Stage,
    code: i32,
}

type Result<T> = std::result::Result<T, Failure>;

impl Failure {
    fn new(stage: Stage, error: io::Error) -> Self {
        Self {
            stage,
            code: error.raw_os_error().unwrap_or(31),
        }
    }

    fn changed(stage: Stage) -> Self {
        Self {
            stage,
            code: CHANGED,
        }
    }

    /// Keep policy/snapshot failures distinguishable from actual native access failures. These
    /// finite reasons also survive elevated execution through the stage/code exit protocol.
    fn reason(&self) -> &'static str {
        match (self.stage, self.code) {
            (_, 1223) => "user_cancelled",
            (_, 5) => "access_denied",
            (Stage::Arguments, _) => "invalid_arguments",
            (Stage::Snapshot, CHANGED) => "record_changed",
            (Stage::Scope, _) => "helper_scope_mismatch",
            (Stage::Inspect, UNSUPPORTED) => "registry_link",
            (Stage::Verify, CHANGED) => "record_still_present",
            (Stage::Elevate, _) => "helper_unavailable_or_result_lost",
            _ => "native_operation_failed",
        }
    }

    fn platform(self, application_id: &str) -> PlatformError {
        let reason = self.reason();
        if self.code == 1223 {
            log::info!(
                "windows_application_record_removal_cancelled application_id={application_id}"
            );
        } else {
            log::warn!("windows_application_record_removal_failed application_id={application_id} stage={:?} reason={reason} native_code={}", self.stage, self.code);
        }
        let code = match self.code {
            5 => PlatformErrorCode::AccessDenied,
            1223 => PlatformErrorCode::UserCancelled,
            CHANGED | 2 | 1018 => PlatformErrorCode::ItemChanged,
            UNSUPPORTED => PlatformErrorCode::Unsupported,
            _ => PlatformErrorCode::OperationFailed,
        };
        let error = PlatformError::new(
            code,
            format!(
                "application record removal failed stage={:?} reason={reason} native_code={}",
                self.stage, self.code
            ),
        );
        if matches!(self.stage, Stage::Verify | Stage::Elevate) && self.code != 1223 {
            error.with_possible_side_effects()
        } else {
            error
        }
    }
}

struct Target {
    parent: Rc<RegKey>,
    name: String,
    view: u32,
    scope: ApplicationInstallScope,
    fingerprint: String,
    children: Vec<String>,
}

pub(super) fn remove(application_id: &str, dry_run: bool) -> PlatformResult<()> {
    // Validate before emitting the ID; untrusted command input must not become log content.
    if !valid_id(application_id) {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "invalid application record ID",
        ));
    }
    let result = (|| {
        let transaction = Transaction::new().map_err(|error| Failure::new(Stage::Locate, error))?;
        let targets = locate(application_id, &transaction)?;
        if targets.is_empty() {
            log::info!("windows_application_record_removed application_id={application_id} already_absent=true files_deleted=0");
            return Ok(());
        }
        log::info!("windows_application_record_removal_resolved application_id={application_id} sources={} child_keys={}", targets.len(), targets.iter().map(|target| target.children.len()).sum::<usize>());
        let fingerprint = fingerprint(&targets);
        let machine = targets
            .iter()
            .all(|target| target.scope == ApplicationInstallScope::Machine);
        drop(targets);
        drop(transaction);
        if dry_run {
            log::info!("windows_application_record_removal_preview application_id={application_id} machine={machine}");
            return Ok(());
        }
        match execute(application_id, &fingerprint, false) {
            Err(error) if error.code == 5 && machine => {
                // Alternate-credential UAC must never reinterpret another user's HKCU hive.
                // Only machine records can enter this helper; current-user ACL failures stay local.
                helper::elevate(application_id, &fingerprint)?;
                verify_absent(application_id)
            }
            result => result,
        }
    })();
    result.map_err(|error: Failure| error.platform(application_id))
}

fn execute(application_id: &str, expected: &str, machine_only: bool) -> Result<()> {
    let transaction = Transaction::new().map_err(|error| Failure::new(Stage::Locate, error))?;
    let targets = locate(application_id, &transaction)?;
    // Another actor may have already removed the selected record. Repeating an explicit removal
    // is successful when the source is absent, without depending on the old catalog revision.
    if targets.is_empty() {
        return Ok(());
    }
    if machine_only
        && targets
            .iter()
            .any(|target| target.scope != ApplicationInstallScope::Machine)
    {
        return Err(Failure::changed(Stage::Scope));
    }
    if fingerprint(&targets) != expected {
        return Err(Failure::changed(Stage::Snapshot));
    }
    for target in &targets {
        let key = match open_plain(&target.parent, &target.name, target.view, &transaction) {
            Ok(key) => key,
            Err(error) if matches!(error.raw_os_error(), Some(2 | 1018)) => continue,
            Err(error) => return Err(Failure::new(Stage::Delete, error)),
        };
        // Snapshot traversal is parent-first, so reversing it removes descendants before parents.
        // Only children discovered without following registry links can enter this exact subtree.
        for child in target.children.iter().rev() {
            key.delete_subkey_transacted_with_flags(child, &transaction, target.view)
                .map_err(|error| Failure::new(Stage::Delete, error))?;
        }
        drop(key);
        match target.parent.delete_subkey_transacted_with_flags(
            &target.name,
            &transaction,
            target.view,
        ) {
            Ok(()) => {}
            // HKCU can expose the same physical key in both views. Its second deletion observes
            // this transaction's pending deletion; commit still detects external write conflicts.
            Err(error) if matches!(error.raw_os_error(), Some(2 | 1018)) => {}
            Err(error) => return Err(Failure::new(Stage::Delete, error)),
        }
    }
    drop(targets);
    transaction
        .commit()
        .map_err(|error| Failure::new(Stage::Commit, error))?;
    verify_absent(application_id)?;
    log::info!("windows_application_record_removed application_id={application_id} verified=true files_deleted=0");
    Ok(())
}

fn fingerprint(targets: &[Target]) -> String {
    let mut digest = blake3::Hasher::new();
    for target in targets {
        digest.update(&target.view.to_le_bytes());
        digest.update(target.fingerprint.as_bytes());
    }
    digest.finalize().to_hex().to_string()
}

fn locate(application_id: &str, transaction: &Transaction) -> Result<Vec<Target>> {
    let mut targets = Vec::new();
    for (root, scope) in [
        (HKEY_CURRENT_USER, ApplicationInstallScope::CurrentUser),
        (HKEY_LOCAL_MACHINE, ApplicationInstallScope::Machine),
    ] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Some(parent) = open_parent(root, view, transaction)? else {
                continue;
            };
            let parent = Rc::new(parent);
            for name in parent.enum_keys() {
                let name = name.map_err(|error| Failure::new(Stage::Locate, error))?;
                if record_id(&name, scope) != application_id {
                    continue;
                }
                let key = open_plain(&parent, &name, view, transaction)
                    .map_err(|error| Failure::new(Stage::Inspect, error))?;
                let (fingerprint, children) = snapshot_tree(key, view, transaction)?;
                targets.push(Target {
                    parent: Rc::clone(&parent),
                    name,
                    view,
                    scope,
                    fingerprint,
                    children,
                });
            }
        }
    }
    Ok(targets)
}

/// Snapshot only registry metadata, not application files or uninstall command validity. Value
/// writes update the key timestamp; traversing every child also detects nested-record changes.
/// Iteration avoids a recursion-depth limit, and no value-size limit blocks valid registrations.
fn snapshot_tree(
    root: RegKey,
    view: u32,
    transaction: &Transaction,
) -> Result<(String, Vec<String>)> {
    let mut pending = vec![(String::new(), root)];
    let mut children = Vec::new();
    let mut digest = blake3::Hasher::new();
    while let Some((path, key)) = pending.pop() {
        let metadata = key
            .query_info()
            .map_err(|error| Failure::new(Stage::Inspect, error))?;
        digest.update(&(path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update(&metadata.last_write_time.dwLowDateTime.to_le_bytes());
        digest.update(&metadata.last_write_time.dwHighDateTime.to_le_bytes());
        digest.update(&metadata.values.to_le_bytes());
        digest.update(&metadata.sub_keys.to_le_bytes());
        let mut names = key
            .enum_keys()
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| Failure::new(Stage::Inspect, error))?;
        names.sort();
        for name in names.into_iter().rev() {
            let child = open_plain(&key, &name, view, transaction)
                .map_err(|error| Failure::new(Stage::Inspect, error))?;
            let child_path = if path.is_empty() {
                name
            } else {
                format!(r"{path}\{name}")
            };
            pending.push((child_path, child));
        }
        if !path.is_empty() {
            children.push(path);
        }
    }
    Ok((digest.finalize().to_hex().to_string(), children))
}

fn open_parent(root: winreg::HKEY, view: u32, transaction: &Transaction) -> Result<Option<RegKey>> {
    let mut key = RegKey::predef(root);
    for component in UNINSTALL_PATH.split('\\') {
        match open_plain(&key, component, view, transaction) {
            Ok(next) => key = next,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Failure::new(Stage::Locate, error)),
        }
    }
    Ok(Some(key))
}

fn open_plain(
    parent: &RegKey,
    name: &str,
    view: u32,
    transaction: &Transaction,
) -> io::Result<RegKey> {
    let key = parent.open_subkey_transacted_with_options_flags(
        name,
        transaction,
        REG_OPTION_OPEN_LINK,
        KEY_READ | view,
    )?;
    match key.get_raw_value("SymbolicLinkValue") {
        Ok(value) if value.vtype == REG_LINK => Err(io::Error::from_raw_os_error(UNSUPPORTED)),
        Ok(_) => Ok(key),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(key),
        Err(error) => Err(error),
    }
}

fn verify_absent(application_id: &str) -> Result<()> {
    let transaction = Transaction::new().map_err(|error| Failure::new(Stage::Verify, error))?;
    for (root, scope) in [
        (HKEY_CURRENT_USER, ApplicationInstallScope::CurrentUser),
        (HKEY_LOCAL_MACHINE, ApplicationInstallScope::Machine),
    ] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Some(parent) =
                open_parent(root, view, &transaction).map_err(|error| Failure {
                    stage: Stage::Verify,
                    ..error
                })?
            {
                for name in parent.enum_keys() {
                    let name = name.map_err(|error| Failure::new(Stage::Verify, error))?;
                    if record_id(&name, scope) == application_id {
                        return Err(Failure::changed(Stage::Verify));
                    }
                }
            }
        }
    }
    Ok(())
}

fn record_id(name: &str, scope: ApplicationInstallScope) -> String {
    application_uninstall_diagnostic_id(&format!(
        "windows-registry:{}:{}",
        if scope == ApplicationInstallScope::Machine {
            "machine"
        } else {
            "current-user"
        },
        name.to_ascii_lowercase()
    ))
}

fn valid_id(value: &str) -> bool {
    value.strip_prefix("application-").is_some_and(|suffix| {
        suffix.len() == 24 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[cfg(test)]
mod tests;
