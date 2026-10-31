//! Bounded, failure-only volume diagnostics shared by permanent-delete batches.

use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, SyncSender, TrySendError},
        OnceLock,
    },
    thread,
    time::Instant,
};

use super::PermanentDeleteFailure;

const MAX_FAILED_VOLUME_SAMPLES: usize = 3;
const MAX_PENDING_BATCHES: usize = 1;

struct VolumeDiagnosticBatch {
    domain: &'static str,
    operation_id: u64,
    scan_id: u64,
    failed_count: usize,
    paths: Vec<String>,
    queued_at: Instant,
}

impl VolumeDiagnosticBatch {
    fn capture(
        domain: &'static str,
        operation_id: u64,
        scan_id: u64,
        failures: &[PermanentDeleteFailure],
    ) -> Self {
        Self {
            domain,
            operation_id,
            scan_id,
            failed_count: failures.len(),
            paths: failures
                .iter()
                .take(MAX_FAILED_VOLUME_SAMPLES)
                .map(|failure| failure.path.clone())
                .collect(),
            queued_at: Instant::now(),
        }
    }
}

enum DiagnosticTask {
    Sample(VolumeDiagnosticBatch),
    #[cfg(test)]
    Barrier(mpsc::Sender<()>),
}

static WORKER: OnceLock<io::Result<SyncSender<DiagnosticTask>>> = OnceLock::new();

fn spawn_worker(
    mut process: impl FnMut(VolumeDiagnosticBatch) + Send + 'static,
) -> io::Result<SyncSender<DiagnosticTask>> {
    // One worker and one waiting batch bound resources even when a driver never returns.
    let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_BATCHES);
    thread::Builder::new()
        .name("volume-diagnostics".into())
        .spawn(move || {
            for task in receiver {
                match task {
                    DiagnosticTask::Sample(batch) => process(batch),
                    #[cfg(test)]
                    DiagnosticTask::Barrier(sender) => {
                        let _ = sender.send(());
                    }
                }
            }
        })?;
    Ok(sender)
}

struct VolumeSample<'a> {
    path: &'a str,
    root: Option<&'a Path>,
    filesystem: Result<&'a str, &'a io::Error>,
    stage: &'static str,
    cached: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct QueryCounts {
    volume_root: usize,
    filesystem: usize,
}

fn visit_failed_volume_samples(
    paths: &[String],
    mut resolve_root: impl FnMut(&Path) -> io::Result<PathBuf>,
    mut read_filesystem: impl FnMut(&Path) -> io::Result<String>,
    mut report: impl FnMut(VolumeSample<'_>),
) -> QueryCounts {
    // Keep the cache local: removable media may reuse a drive letter in the next batch.
    let mut volumes: Vec<(PathBuf, io::Result<String>)> = Vec::new();
    let mut counts = QueryCounts::default();
    for path in paths.iter().take(MAX_FAILED_VOLUME_SAMPLES) {
        counts.volume_root += 1;
        match resolve_root(Path::new(path)) {
            Err(error) => report(VolumeSample {
                path,
                root: None,
                filesystem: Err(&error),
                stage: "volume_root",
                cached: false,
            }),
            Ok(root) => {
                let existing = volumes.iter().position(|(known, _)| known == &root);
                let index = existing.unwrap_or_else(|| {
                    counts.filesystem += 1;
                    let result = read_filesystem(&root);
                    volumes.push((root, result));
                    volumes.len() - 1
                });
                let (root, result) = &volumes[index];
                report(VolumeSample {
                    path,
                    root: Some(root),
                    filesystem: result.as_deref(),
                    stage: "volume_information",
                    cached: existing.is_some(),
                });
            }
        }
    }
    counts
}

pub(crate) fn log_failed_volume_samples(
    domain: &'static str,
    operation_id: u64,
    scan_id: u64,
    failures: &[PermanentDeleteFailure],
) {
    if failures.is_empty() || !log::log_enabled!(log::Level::Warn) {
        return;
    }
    let batch = VolumeDiagnosticBatch::capture(domain, operation_id, scan_id, failures);
    // Retain selected objects in the dispatch record: a stalled driver or process exit
    // may prevent the worker from emitting its completion record.
    let path_sample = batch
        .paths
        .iter()
        .map(|path| mangodisk_platform::diagnostics::bounded_message(path, 8192))
        .collect::<Vec<_>>();
    let worker = WORKER.get_or_init(|| spawn_worker(log_volume_batch));
    let (outcome, reason, native_code, error) = match worker {
        Err(error) => (
            "skipped",
            "worker_unavailable",
            error.raw_os_error(),
            error.to_string(),
        ),
        Ok(sender) => match sender.try_send(DiagnosticTask::Sample(batch)) {
            Ok(()) => ("queued", "ready", None, String::new()),
            Err(TrySendError::Full(_)) => ("skipped", "queue_full", None, String::new()),
            Err(TrySendError::Disconnected(_)) => {
                ("skipped", "worker_disconnected", None, String::new())
            }
        },
    };
    log::warn!(
        "permanent_delete_volume_diagnostics_dispatched domain={} operation_id={} scan_id={} scope=failed_path_sample failed_count={} sampled_count={} omitted_count={} path_sample={:?} outcome={} reason={} native_code={:?} error={}",
        domain, operation_id, scan_id, failures.len(), path_sample.len(),
        failures.len().saturating_sub(path_sample.len()), path_sample,
        outcome, reason, native_code, mangodisk_platform::diagnostics::text(&error),
    );
}

fn log_volume_batch(batch: VolumeDiagnosticBatch) {
    let queue_wait_us = batch.queued_at.elapsed().as_micros();
    let started = Instant::now();
    log::warn!(
        "permanent_delete_volume_diagnostics_started domain={} operation_id={} scan_id={} scope=failed_path_sample failed_count={} sampled_count={} queue_wait_us={}",
        batch.domain, batch.operation_id, batch.scan_id, batch.failed_count,
        batch.paths.len(), queue_wait_us,
    );
    let counts = visit_failed_volume_samples(
        &batch.paths,
        mangodisk_platform::volume_filesystem::volume_root,
        mangodisk_platform::volume_filesystem::filesystem_name,
        |sample| {
            let root = sample.root.map(|root| root.display().to_string());
            let (outcome, filesystem, native_code, error) = match sample.filesystem {
                Ok(filesystem) => ("ready", filesystem, None, String::new()),
                Err(error) => (
                    "query_failed",
                    "unknown",
                    error.raw_os_error(),
                    error.to_string(),
                ),
            };
            log::warn!(
                "permanent_delete_volume_sample domain={} operation_id={} scan_id={} scope=failed_path_sample path={} volume_root={} filesystem={} outcome={} stage={} native_code={:?} error={} cached={}",
                batch.domain, batch.operation_id, batch.scan_id,
                mangodisk_platform::diagnostics::text(&sample.path),
                mangodisk_platform::diagnostics::text(&root.as_deref().unwrap_or("unknown")),
                mangodisk_platform::diagnostics::text(&filesystem), outcome, sample.stage, native_code,
                mangodisk_platform::diagnostics::text(&error), sample.cached,
            );
        },
    );
    log::warn!(
        "permanent_delete_volume_diagnostics_finished domain={} operation_id={} scan_id={} failed_count={} sampled_count={} omitted_count={} volume_root_queries={} filesystem_queries={} queue_wait_us={} elapsed_us={}",
        batch.domain, batch.operation_id, batch.scan_id, batch.failed_count, counts.volume_root,
        batch.failed_count.saturating_sub(counts.volume_root), counts.volume_root, counts.filesystem,
        queue_wait_us, started.elapsed().as_micros(),
    );
}

#[cfg(test)]
fn wait_for_volume_diagnostics() {
    use std::time::Duration;

    let Some(worker) = WORKER.get() else {
        return;
    };
    let worker = worker.as_ref().expect("diagnostic worker unavailable");
    let (sender, receiver) = mpsc::channel();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match worker.try_send(DiagnosticTask::Barrier(sender.clone())) {
            Ok(()) => break,
            Err(TrySendError::Full(_)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            }
            _ => panic!("diagnostic queue did not become available"),
        }
    }
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("diagnostic worker did not become idle");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("C:\\fixture\\{index}.bin"))
            .collect()
    }

    #[test]
    fn successful_batches_make_no_volume_queries() {
        assert_eq!(
            visit_failed_volume_samples(
                &[],
                |_| panic!("unexpected root query"),
                |_| panic!("unexpected filesystem query"),
                |_| panic!("unexpected log")
            ),
            QueryCounts::default()
        );
    }

    #[test]
    fn ten_thousand_failures_are_bounded_and_share_one_filesystem_query() {
        let mut reported = 0;
        let counts = visit_failed_volume_samples(
            &paths(10_000),
            |_| Ok(PathBuf::from("C:\\")),
            |_| Ok("NTFS".into()),
            |sample| {
                assert_eq!(sample.filesystem.unwrap(), "NTFS");
                assert_eq!(sample.cached, reported > 0);
                reported += 1;
            },
        );
        assert_eq!(reported, 3);
        assert_eq!(
            counts,
            QueryCounts {
                volume_root: 3,
                filesystem: 1
            }
        );
    }

    #[test]
    fn mounted_volumes_on_one_drive_are_not_conflated() {
        let mut index = 0;
        let mut filesystems = Vec::new();
        let counts = visit_failed_volume_samples(
            &paths(3),
            |_| {
                let root = if index == 1 { "C:\\mount\\" } else { "C:\\" };
                index += 1;
                Ok(PathBuf::from(root))
            },
            |root| {
                Ok(if root == Path::new("C:\\mount\\") {
                    "exFAT"
                } else {
                    "NTFS"
                }
                .into())
            },
            |sample| filesystems.push(sample.filesystem.unwrap().to_owned()),
        );
        assert_eq!(filesystems, ["NTFS", "exFAT", "NTFS"]);
        assert_eq!(counts.filesystem, 2);
    }

    #[test]
    fn native_errors_retain_the_stage_and_are_not_retried_per_file() {
        let mut index = 0;
        let counts = visit_failed_volume_samples(
            &paths(3),
            |_| {
                index += 1;
                if index == 1 {
                    Err(io::Error::from_raw_os_error(3))
                } else {
                    Ok(PathBuf::from("C:\\"))
                }
            },
            |_| Err(io::Error::from_raw_os_error(21)),
            |sample| {
                let expected = if sample.root.is_none() {
                    ("volume_root", 3)
                } else {
                    ("volume_information", 21)
                };
                assert_eq!(sample.stage, expected.0);
                assert_eq!(
                    sample.filesystem.unwrap_err().raw_os_error(),
                    Some(expected.1)
                );
            },
        );
        assert_eq!(
            counts,
            QueryCounts {
                volume_root: 3,
                filesystem: 1
            }
        );
    }

    #[test]
    fn successive_batches_do_not_reuse_removable_volume_information() {
        for expected in ["exFAT", "FAT32"] {
            let counts = visit_failed_volume_samples(
                &paths(1),
                |_| Ok(PathBuf::from("G:\\")),
                |_| Ok(expected.into()),
                |sample| assert_eq!(sample.filesystem.unwrap(), expected),
            );
            assert_eq!(counts.filesystem, 1);
        }
    }

    #[test]
    fn queued_samples_own_only_three_paths_and_retain_batch_context() {
        let mut failures = paths(10_000)
            .into_iter()
            .map(|path| PermanentDeleteFailure {
                path,
                details: crate::filesystem::models::PermanentDeleteFailureDetails {
                    message: "delete failed".into(),
                    ..Default::default()
                },
            })
            .collect::<Vec<_>>();
        let batch = VolumeDiagnosticBatch::capture("duplicates", 7, 11, &failures);
        failures.clear();
        assert_eq!(batch.domain, "duplicates");
        assert_eq!((batch.operation_id, batch.scan_id), (7, 11));
        assert_eq!(batch.failed_count, 10_000);
        assert_eq!(batch.paths, paths(3));
    }

    #[test]
    fn stalled_queries_allow_one_pending_batch_and_reject_overload() {
        use std::time::Duration;

        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (finished_sender, finished_receiver) = mpsc::channel();
        let worker = spawn_worker(move |batch| {
            if batch.operation_id == 1 {
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
            }
            finished_sender.send(batch.operation_id).unwrap();
        })
        .unwrap();
        let batch = |operation_id| {
            DiagnosticTask::Sample(VolumeDiagnosticBatch {
                domain: "duplicates",
                operation_id,
                scan_id: 11,
                failed_count: 1,
                paths: paths(1),
                queued_at: Instant::now(),
            })
        };
        assert!(worker.try_send(batch(1)).is_ok());
        entered_receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(worker.try_send(batch(2)).is_ok());
        assert!(matches!(
            worker.try_send(batch(3)),
            Err(TrySendError::Full(_))
        ));
        release_sender.send(()).unwrap();
        for expected in [1, 2] {
            assert_eq!(
                finished_receiver
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap(),
                expected
            );
        }
        assert!(finished_receiver.try_recv().is_err());
    }
}

#[cfg(test)]
#[path = "permanent_delete_diagnostics_validation_tests.rs"]
mod validation_tests;
