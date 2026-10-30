use super::*;
use mangodisk_platform::ApplicationProcessCloseResult;
use std::path::PathBuf;

fn image(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"C:\Apps\{name}.exe"))
    } else {
        PathBuf::from(format!("/Applications/{name}.app/Contents/MacOS/{name}"))
    }
}
fn process(pid: u32, name: &str) -> ProcessMemory {
    ProcessMemory {
        pid,
        name: name.into(),
        executable: Some(image(name)),
        used_bytes: Some(0),
        is_application: true,
    }
}
fn id(path: &Path) -> String {
    running_identity::id(Some(&running_identity::application_path(path)), 2)
}
fn run(
    processes: &[ProcessMemory],
    id: &str,
    native: ApplicationProcessCloseResult,
) -> ApplicationForceQuitResult {
    execute_resolved(id, "operation", processes, 1, Instant::now(), |path| {
        assert_eq!(path, running_identity::application_path(&image("Target")));
        Ok(native)
    })
    .unwrap()
}
#[test]
fn only_the_resolved_application_image_is_authorized_even_without_a_memory_reading() {
    let processes = vec![
        process(1, "MangoDisk"),
        process(2, "Target"),
        process(3, "Other"),
    ];
    let result = run(
        &processes,
        &id(&image("Target")),
        ApplicationProcessCloseResult {
            matched_process_count: 2,
            requested_process_count: 2,
            remaining_processes: vec![],
        },
    );
    assert_eq!(result.status, ApplicationForceQuitStatus::Completed);
    assert_eq!(result.requested_process_count, 2);
    assert_eq!(result.operation_id, "operation");
    assert_eq!(result.schema_version, 1);
}
#[test]
fn stale_self_unknown_and_protected_targets_never_reach_the_native_action() {
    let mut processes = vec![process(1, "MangoDisk"), process(2, "Target")];
    for (target, expected) in [
        (id(&image("MangoDisk")), ApplicationForceQuitStatus::Blocked),
        ("0".repeat(64), ApplicationForceQuitStatus::Unavailable),
    ] {
        let result = execute_resolved(&target, "operation", &processes, 1, Instant::now(), |_| {
            panic!("unsafe target")
        })
        .unwrap();
        assert_eq!(result.status, expected);
    }
    processes.remove(0);
    assert_eq!(
        execute_resolved(
            &id(&image("Target")),
            "operation",
            &processes,
            1,
            Instant::now(),
            |_| panic!("missing own identity")
        )
        .unwrap()
        .status,
        ApplicationForceQuitStatus::Blocked
    );
    let system = if cfg!(windows) {
        PathBuf::from(r"C:\Windows\explorer.exe")
    } else {
        PathBuf::from("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder")
    };
    processes = vec![
        process(1, "MangoDisk"),
        ProcessMemory {
            executable: Some(system.clone()),
            ..process(2, "System")
        },
    ];
    assert_eq!(
        execute_resolved(
            &id(&system),
            "operation",
            &processes,
            1,
            Instant::now(),
            |_| panic!("protected system image")
        )
        .unwrap()
        .status,
        ApplicationForceQuitStatus::Blocked
    );
}
#[test]
fn native_remainders_and_exited_targets_are_not_reported_as_completed() {
    let processes = vec![process(1, "MangoDisk"), process(2, "Target")];
    let result = run(
        &processes,
        &id(&image("Target")),
        ApplicationProcessCloseResult {
            matched_process_count: 1,
            requested_process_count: 1,
            remaining_processes: vec!["Target".into()],
        },
    );
    assert_eq!(result.status, ApplicationForceQuitStatus::StillRunning);
    assert_eq!(result.remaining_processes, vec!["Target"]);
    assert_eq!(
        run(
            &processes,
            &id(&image("Target")),
            ApplicationProcessCloseResult::default()
        )
        .status,
        ApplicationForceQuitStatus::Unavailable
    );
}
#[test]
fn invalid_transport_identities_are_rejected_before_sampling() {
    for value in ["", "arbitrary.exe", &"g".repeat(64), &"a".repeat(65)] {
        assert!(execute(value).is_err());
    }
}
#[test]
fn native_errors_remain_errors_instead_of_fabricated_success() {
    let processes = vec![process(1, "MangoDisk"), process(2, "Target")];
    assert!(execute_resolved(
        &id(&image("Target")),
        "operation",
        &processes,
        1,
        Instant::now(),
        |_| Err(mangodisk_platform::PlatformError::operation_failed(
            "permission denied"
        ))
    )
    .is_err());
}
