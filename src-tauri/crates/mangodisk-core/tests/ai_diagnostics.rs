//! Exercise the production diagnostics through public Core boundaries. This
//! separate test binary owns its logger without changing the application's logger.
use std::{
    cell::RefCell,
    io::{Read, Write},
    net::TcpListener,
    sync::{mpsc, Once},
    thread,
    time::{Duration, Instant},
};

use mangodisk_core::{
    ai::{explain, AiConfiguration, AiConfigurationUpdate, AiError, AiRequest, ReasoningMode},
    configure_application_paths, ApplicationPaths,
};
use tokio::sync::watch;

thread_local! {
    static RECORDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn begin_logs() {
    struct TestLogger;
    impl log::Log for TestLogger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target().starts_with("mangodisk_core::ai") {
                RECORDS.with(|records| {
                    records
                        .borrow_mut()
                        .push(format!("{} {}", record.level(), record.args()))
                });
            }
        }
        fn flush(&self) {}
    }
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_logger(&TestLogger).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
    RECORDS.with(|records| records.borrow_mut().clear());
}

fn logs() -> String {
    RECORDS.with(|records| records.borrow().join("\n"))
}

struct Server {
    endpoint: String,
    release: mpsc::Sender<()>,
    task: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(status: u16, body: &str, stall: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let (release, released) = mpsc::channel();
        let body = body.to_owned();
        let task = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "request did not connect");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("fixture listener failed: {}", error.kind()),
                }
            };
            // Windows can inherit the listener's nonblocking mode. The fixture
            // uses bounded blocking reads after accept on every platform.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 4096];
                let length = socket.read(&mut bytes).unwrap();
                assert!(length > 0);
                request.extend_from_slice(&bytes[..length]);
                assert!(request.len() < 16384);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let length = body.len() + if stall { 10000 } else { 0 };
            let _ = write!(socket, "HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}");
            if stall {
                let _ = released.recv_timeout(Duration::from_secs(5));
            }
        });
        Self {
            endpoint,
            release,
            task: Some(task),
        }
    }

    fn config(&self) -> AiConfiguration {
        AiConfiguration {
            schema_version: 1,
            mode: mangodisk_core::ai::AiServiceMode::Custom,
            free_consent: false,
            endpoint: self.endpoint.clone(),
            model: "fixture".into(),
            api_key: "synthetic-secret".into(),
            reasoning: ReasoningMode::Default,
            temperature: None,
            max_tokens: None,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.release.send(());
        if let Some(task) = self.task.take() {
            if let Err(error) = task.join() {
                // Preserve the original assertion when cleanup follows a panic;
                // a second panic in Drop would abort the entire test executable.
                if !thread::panicking() {
                    std::panic::resume_unwind(error);
                }
            }
        }
    }
}

fn request() -> AiRequest {
    AiRequest {
        context: None,
        language: "en-US".into(),
    }
}

#[tokio::test]
async fn provider_errors_preserve_status_and_exclude_untrusted_fields() {
    let oversized = "synthetic-secret".repeat(700);
    for (status, body, expected, evidence) in [
        (
            400,
            r#"{"error":{"code":"unsupported_parameter","param":"thinking","message":"synthetic-secret"}}"#,
            AiError::ProviderRejected,
            "code: UnsupportedParameter, parameter: Thinking",
        ),
        (
            401,
            r#"{"error":{"code":"synthetic-secret","param":"/private/secret","message":"synthetic-secret"}}"#,
            AiError::Unauthorized,
            "code: Unknown, parameter: Unknown",
        ),
        (
            429,
            "<html>synthetic-secret</html>",
            AiError::QuotaExceeded,
            "body: Invalid",
        ),
        (
            500,
            oversized.as_str(),
            AiError::ProviderRejected,
            "body: TooLarge",
        ),
    ] {
        begin_logs();
        let server = Server::new(status, body, false);
        let (_cancel, receiver) = watch::channel(false);
        let result = explain(server.config(), request(), "http-fixture", receiver, |_| {
            true
        })
        .await;
        assert_eq!(result.unwrap_err(), expected);
        let output = logs();
        assert!(output.contains(evidence), "missing diagnostic: {output}");
        assert!(!output.contains("synthetic-secret"));
        assert!(!output.contains("/private/secret"));
        assert!(!output.contains(&server.endpoint));
    }
}

#[tokio::test]
async fn stalled_error_body_preserves_original_http_failure() {
    begin_logs();
    let server = Server::new(401, "", true);
    let (_cancel, receiver) = watch::channel(false);
    let result = explain(
        server.config(),
        request(),
        "stall-fixture",
        receiver,
        |_| true,
    )
    .await;
    assert_eq!(result.unwrap_err(), AiError::Unauthorized);
    assert!(logs().contains("body: Timeout"));
}

#[tokio::test]
async fn cancellation_records_partial_progress_without_logging_answer_or_reasoning() {
    begin_logs();
    let server = Server::new(200, "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"private-reasoning\",\"content\":\"private-answer\"}}]}\n\n", true);
    let (cancel, receiver) = watch::channel(false);
    let result = explain(
        server.config(),
        request(),
        "cancel-fixture",
        receiver,
        |_| {
            cancel.send_replace(true);
            true
        },
    )
    .await;
    assert_eq!(result.unwrap_err(), AiError::Cancelled);
    let output = logs();
    assert_eq!(output.matches("ai_stream_finished").count(), 1);
    assert!(output.contains("text_bytes=14 reasoning_bytes=17"));
    assert!(output.contains("outcome=cancelled reason=Some(Cancelled)"));
    assert!(!output.contains("WARN"));
    assert!(!output.contains("private-answer"));
    assert!(!output.contains("private-reasoning"));
}

#[tokio::test]
async fn disconnected_error_body_preserves_status_and_records_read_failure() {
    begin_logs();
    let server = Server::new(401, "", true);
    // Advertise a body, then close after headers without delivering that body.
    server.release.send(()).unwrap();
    let (_cancel, receiver) = watch::channel(false);
    let result = explain(
        server.config(),
        request(),
        "disconnect-fixture",
        receiver,
        |_| true,
    )
    .await;
    assert_eq!(result.unwrap_err(), AiError::Unauthorized);
    assert!(logs().contains("body: ReadFailed"));
}

#[tokio::test]
async fn completed_stream_records_success_and_usage_once() {
    begin_logs();
    let server = Server::new(200, "data: {\"choices\":[{\"delta\":{\"content\":\"private-answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n", false);
    let (_cancel, receiver) = watch::channel(false);
    let result = explain(
        server.config(),
        request(),
        "success-fixture",
        receiver,
        |_| true,
    )
    .await;
    assert_eq!(result.unwrap().completion_tokens, Some(3));
    let output = logs();
    assert_eq!(output.matches("ai_stream_finished").count(), 1);
    assert!(output.contains("outcome=completed reason=None"));
    assert!(output.contains("prompt_tokens=Some(12) completion_tokens=Some(3)"));
    assert!(output.contains("done=true finish_reason=stop"));
    assert!(!output.contains("private-answer"));
}

#[tokio::test]
async fn sse_errors_and_incomplete_streams_have_terminal_diagnostics() {
    for (body, expected, evidence) in [
        ("data: {\"error\":{\"code\":\"context_length_exceeded\",\"param\":\"messages\",\"message\":\"synthetic-secret\"}}\n\n", AiError::ProviderRejected, "code: ContextLengthExceeded, parameter: Messages"),
        ("data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n", AiError::IncompleteStream, "outcome=failed reason=Some(IncompleteStream)"),
    ] {
        begin_logs();
        let server = Server::new(200, body, false);
        let (_cancel, receiver) = watch::channel(false);
        let result = explain(server.config(), request(), "stream-fixture", receiver, |_| true).await;
        assert_eq!(result.unwrap_err(), expected);
        assert!(logs().contains(evidence));
        assert!(!logs().contains("synthetic-secret"));
    }
}

#[test]
fn configuration_io_diagnostics_identify_stage_without_paths() {
    begin_logs();
    let root = tempfile::tempdir().unwrap();
    let paths = ApplicationPaths::from_base_directories(
        root.path().join("data"),
        root.path().join("cache"),
    )
    .unwrap();
    let config_path = paths.data_directory().join("ai.json");
    configure_application_paths(paths).unwrap();
    std::fs::create_dir_all(&config_path).unwrap();
    let update = || AiConfigurationUpdate {
        mode: mangodisk_core::ai::AiServiceMode::Custom,
        free_consent: false,
        endpoint: "https://example.com/v1".into(),
        model: "fixture".into(),
        api_key: Some("synthetic-secret".into()),
        reasoning: ReasoningMode::Default,
        temperature: None,
        max_tokens: None,
    };
    assert!(matches!(
        AiConfiguration::save(update()),
        Err(AiError::ConfigurationUnavailable)
    ));
    assert!(logs().contains("stage=Persist"));
    assert!(matches!(
        AiConfiguration::delete(),
        Err(AiError::ConfigurationUnavailable)
    ));
    assert!(logs().contains("stage=Delete"));
    // Invalid UTF-8 deterministically exercises read failure on both platforms.
    std::fs::remove_dir(&config_path).unwrap();
    std::fs::write(&config_path, [0xff]).unwrap();
    assert!(matches!(
        AiConfiguration::load(),
        Err(AiError::ConfigurationUnavailable)
    ));
    assert!(logs().contains("stage=Read kind=InvalidData"));
    assert!(logs().contains("os_code="));
    assert!(!logs().contains("synthetic-secret"));
    assert!(!logs().contains(root.path().to_str().unwrap()));
    AiConfiguration::save(update()).unwrap();
    AiConfiguration::delete().unwrap();
    // A regular file occupying the data-directory location is a reproducible
    // directory-creation failure that does not depend on administrator privileges.
    let parent = config_path.parent().unwrap();
    std::fs::remove_dir(parent).unwrap();
    std::fs::write(parent, "synthetic-secret").unwrap();
    assert!(matches!(
        AiConfiguration::save(update()),
        Err(AiError::ConfigurationUnavailable)
    ));
    assert!(logs().contains("stage=CreateDirectory"));
    assert!(!logs().contains("synthetic-secret"));
    assert!(!logs().contains(root.path().to_str().unwrap()));
}
