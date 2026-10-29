use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

#[cfg(debug_assertions)]
use std::net::IpAddr;

use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const FEEDBACK_SCHEMA_VERSION: u8 = 1;
const MAX_ATTACHMENT_COUNT: usize = 5;
const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
const MAX_LOG_FILE_COUNT: usize = 3;
const FEEDBACK_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FEEDBACK_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const PRODUCTION_FEEDBACK_URL: &str = "https://mangodisk.app/api/v1/feedbacks";

#[derive(Debug)]
pub enum FeedbackError {
    InvalidAttachment,
    TooManyAttachments,
    AttachmentTooLarge,
    AttachmentUnavailable,
    InvalidSubmission,
    DraftStorage,
    LogArchive,
    Network,
    ServerRejected,
}

impl fmt::Display for FeedbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAttachment => "invalid_feedback_attachment",
            Self::TooManyAttachments => "too_many_feedback_attachments",
            Self::AttachmentTooLarge => "feedback_attachment_too_large",
            Self::AttachmentUnavailable => "feedback_attachment_unavailable",
            Self::InvalidSubmission => "invalid_feedback_submission",
            Self::DraftStorage => "feedback_draft_storage_failed",
            Self::LogArchive => "feedback_log_archive_failed",
            Self::Network => "feedback_network_failed",
            Self::ServerRejected => "feedback_server_rejected",
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFeedbackAttachment {
    pub token: String,
    pub display_name: String,
    pub mime_type: String,
    pub size: usize,
}

#[derive(Debug, Clone)]
struct StagedFeedbackFile {
    descriptor: StagedFeedbackAttachment,
    path: PathBuf,
}

#[derive(Clone)]
pub struct FeedbackDraftStore {
    root: PathBuf,
    files: Arc<Mutex<HashMap<String, StagedFeedbackFile>>>,
}

impl FeedbackDraftStore {
    pub fn initialize(cache_directory: &Path) -> Self {
        let draft_directory = cache_directory.join("feedback-drafts");
        let root = draft_directory.join(Uuid::new_v4().to_string());
        Self {
            root,
            files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn cleanup_stale_drafts(&self) {
        let Some(draft_directory) = self.root.parent() else {
            return;
        };
        // Draft files are disposable and valid only for the process that created
        // them. This runs outside application setup and always skips the current
        // process root, so cleanup cannot delay startup or race with new drafts.
        match fs::read_dir(draft_directory) {
            Ok(entries) => {
                for entry in entries.filter_map(Result::ok) {
                    if entry.path() == self.root {
                        continue;
                    }
                    let result = if entry.path().is_dir() {
                        fs::remove_dir_all(entry.path())
                    } else {
                        fs::remove_file(entry.path())
                    };
                    if let Err(error) = result {
                        log::warn!("feedback_stale_draft_cleanup_failed error={error}");
                    }
                }
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                log::warn!("feedback_stale_draft_cleanup_skipped reason=directory_unavailable error={error}");
            }
            Err(_) => {}
        }
    }

    pub fn stage(
        &self,
        display_name: String,
        mime_type: String,
        data: Vec<u8>,
    ) -> Result<StagedFeedbackAttachment, FeedbackError> {
        if data.is_empty() || !attachment_matches_mime_type(&mime_type, &data) {
            return Err(FeedbackError::InvalidAttachment);
        }
        if data.len() > MAX_ATTACHMENT_BYTES {
            return Err(FeedbackError::AttachmentTooLarge);
        }

        // Feedback is optional application functionality. Defer filesystem
        // creation until the first attachment so an unavailable cache never
        // prevents the rest of MangoDisk from starting.
        fs::create_dir_all(&self.root).map_err(|_| FeedbackError::DraftStorage)?;
        let token = Uuid::new_v4().to_string();
        let path = self.root.join(format!("{token}.bin"));
        fs::write(&path, &data).map_err(|_| FeedbackError::DraftStorage)?;
        let descriptor = StagedFeedbackAttachment {
            token: token.clone(),
            display_name: safe_display_name(&display_name),
            mime_type,
            size: data.len(),
        };

        let mut files = self.files.lock().map_err(|_| FeedbackError::DraftStorage)?;
        if files.len() >= MAX_ATTACHMENT_COUNT {
            let _ = fs::remove_file(path);
            return Err(FeedbackError::TooManyAttachments);
        }
        files.insert(
            token,
            StagedFeedbackFile {
                descriptor: descriptor.clone(),
                path,
            },
        );
        Ok(descriptor)
    }

    fn resolve(&self, tokens: &[String]) -> Result<Vec<StagedFeedbackFile>, FeedbackError> {
        if tokens.len() > MAX_ATTACHMENT_COUNT {
            return Err(FeedbackError::TooManyAttachments);
        }
        if tokens.iter().collect::<HashSet<_>>().len() != tokens.len() {
            return Err(FeedbackError::InvalidSubmission);
        }
        let files = self.files.lock().map_err(|_| FeedbackError::DraftStorage)?;
        tokens
            .iter()
            .map(|token| {
                files
                    .get(token)
                    .cloned()
                    .ok_or(FeedbackError::AttachmentUnavailable)
            })
            .collect()
    }

    pub fn discard(&self, tokens: &[String]) {
        let Ok(mut files) = self.files.lock() else {
            log::warn!("feedback_draft_cleanup_failed reason=state_lock_poisoned");
            return;
        };
        for token in tokens {
            if let Some(file) = files.remove(token) {
                if let Err(error) = fs::remove_file(file.path) {
                    log::warn!(
                        "feedback_draft_cleanup_failed reason=file_remove_failed error={error}"
                    );
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackRequest {
    pub request_id: String,
    pub install_id: Option<String>,
    pub category: String,
    pub content: String,
    pub email: Option<String>,
    pub locale: String,
    pub include_logs: bool,
    pub attachment_tokens: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackResult {
    pub id: String,
    pub created_at: String,
    pub submitted_log_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackMetadata<'a> {
    schema_version: u8,
    request_id: &'a str,
    install_id: Option<&'a str>,
    category: &'a str,
    content: &'a str,
    email: Option<&'a str>,
    platform: &'static str,
    architecture: &'static str,
    app_version: &'a str,
    os_version: String,
    locale: &'a str,
}

#[derive(Deserialize)]
struct FeedbackApiResponse {
    success: bool,
    data: Option<FeedbackApiData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackApiData {
    id: String,
    created_at: String,
}

struct PreparedFeedbackFile {
    descriptor: StagedFeedbackAttachment,
    data: Vec<u8>,
}

struct PreparedFeedbackSubmission {
    files: Vec<PreparedFeedbackFile>,
    log_archive: Option<Vec<u8>>,
    submitted_log_count: usize,
}

pub struct FeedbackSubmissionService;

impl FeedbackSubmissionService {
    pub async fn submit(
        store: &FeedbackDraftStore,
        log_directory: &Path,
        app_version: &str,
        request: SubmitFeedbackRequest,
        metadata: &mangodisk_core::http_client::OfficialRequestMetadata,
    ) -> Result<SubmitFeedbackResult, FeedbackError> {
        validate_submission(&request)?;
        let started_at = Instant::now();
        let request_id = request.request_id.clone();
        log::info!(
            "feedback_submission_started request_id={} attachment_count={} include_logs={}",
            request_id,
            request.attachment_tokens.len(),
            request.include_logs
        );
        let prepared = prepare_feedback_submission(
            store.clone(),
            request.attachment_tokens.clone(),
            log_directory.to_path_buf(),
            request.include_logs,
        )
        .await?;
        let PreparedFeedbackSubmission {
            files,
            log_archive,
            submitted_log_count,
        } = prepared;
        let attachment_count = files.len();

        let mut form = Form::new().part(
            "payload",
            Part::text(
                serde_json::to_string(&FeedbackMetadata {
                    schema_version: FEEDBACK_SCHEMA_VERSION,
                    request_id: &request.request_id,
                    install_id: request.install_id.as_deref(),
                    category: &request.category,
                    content: request.content.trim(),
                    email: request
                        .email
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty()),
                    platform: tauri_plugin_os::platform(),
                    architecture: tauri_plugin_os::arch(),
                    app_version,
                    os_version: tauri_plugin_os::version().to_string(),
                    locale: &request.locale,
                })
                .map_err(|_| FeedbackError::InvalidSubmission)?,
            )
            .mime_str("application/json")
            .map_err(|_| FeedbackError::InvalidSubmission)?,
        );

        for file in files {
            let part = Part::bytes(file.data)
                .file_name(file.descriptor.display_name)
                .mime_str(&file.descriptor.mime_type)
                .map_err(|_| FeedbackError::InvalidAttachment)?;
            form = form.part("attachment", part);
        }

        if let Some(archive) = log_archive {
            log::info!(
                "feedback_log_archive_created request_id={} file_count={} archive_bytes={}",
                request_id,
                submitted_log_count,
                archive.len()
            );
            form = form.part(
                "diagnosticLog",
                Part::bytes(archive)
                    .file_name("MangoDisk-diagnostics.zip")
                    .mime_str("application/zip")
                    .map_err(|_| FeedbackError::LogArchive)?,
            );
        }

        let client = client().build().map_err(|_| FeedbackError::Network)?;
        let response = send(&client, &feedback_endpoint(), form, metadata).await?;
        if !response.status().is_success() {
            log::warn!(
                "feedback_submission_rejected request_id={} status={}",
                request_id,
                response.status().as_u16()
            );
            return Err(FeedbackError::ServerRejected);
        }
        let payload = response
            .json::<FeedbackApiResponse>()
            .await
            .map_err(|_| FeedbackError::ServerRejected)?;
        let data = payload
            .success
            .then_some(payload.data)
            .flatten()
            .ok_or(FeedbackError::ServerRejected)?;

        store.discard(&request.attachment_tokens);
        log::info!(
            "feedback_submission_completed request_id={} feedback_id={} attachment_count={} log_count={} elapsed_ms={}",
            request_id,
            data.id,
            attachment_count,
            submitted_log_count,
            started_at.elapsed().as_millis()
        );
        Ok(SubmitFeedbackResult {
            id: data.id,
            created_at: data.created_at,
            submitted_log_count,
        })
    }
}

async fn prepare_feedback_submission(
    store: FeedbackDraftStore,
    attachment_tokens: Vec<String>,
    log_directory: PathBuf,
    include_logs: bool,
) -> Result<PreparedFeedbackSubmission, FeedbackError> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = store
            .resolve(&attachment_tokens)?
            .into_iter()
            .map(|file| {
                let data =
                    fs::read(&file.path).map_err(|_| FeedbackError::AttachmentUnavailable)?;
                Ok(PreparedFeedbackFile {
                    descriptor: file.descriptor,
                    data,
                })
            })
            .collect::<Result<Vec<_>, FeedbackError>>()?;
        let (log_archive, submitted_log_count) = if include_logs {
            match create_recent_log_archive(&log_directory) {
                Ok(result) => result,
                Err(error) => {
                    // Feedback remains useful without diagnostics. A log packaging
                    // failure must not discard the user's report or attachments.
                    log::warn!("feedback_log_archive_skipped reason={error}");
                    (None, 0)
                }
            }
        } else {
            (None, 0)
        };
        Ok(PreparedFeedbackSubmission {
            files,
            log_archive,
            submitted_log_count,
        })
    })
    .await
    .map_err(|_| FeedbackError::DraftStorage)?
}

fn client() -> reqwest::ClientBuilder {
    mangodisk_core::http_client::builder()
        .connect_timeout(FEEDBACK_CONNECT_TIMEOUT)
        .timeout(FEEDBACK_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
}

async fn send(
    client: &reqwest::Client,
    endpoint: &str,
    form: Form,
    metadata: &mangodisk_core::http_client::OfficialRequestMetadata,
) -> Result<reqwest::Response, FeedbackError> {
    super::client_request_metadata::log_request("feedback", metadata);
    client
        .post(endpoint)
        .headers(metadata.headers())
        .multipart(form)
        .send()
        .await
        .map_err(|_| FeedbackError::Network)
}

fn validate_submission(request: &SubmitFeedbackRequest) -> Result<(), FeedbackError> {
    // Rust `char` counts Unicode scalar values, while HTML maxlength and the
    // website API count UTF-16 code units. Use the wire contract's semantics.
    let content_length = request.content.trim().encode_utf16().count();
    let valid_category = matches!(request.category.as_str(), "issue" | "suggestion" | "other");
    if Uuid::parse_str(&request.request_id).is_err()
        || request
            .install_id
            .as_deref()
            .is_some_and(|id| Uuid::parse_str(id).is_err())
        || !valid_category
        || !(10..=5000).contains(&content_length)
        || request.locale.trim().is_empty()
        || request.locale.trim().len() > 32
        || request
            .email
            .as_deref()
            .is_some_and(|email| !email.trim().is_empty() && !looks_like_email(email))
    {
        return Err(FeedbackError::InvalidSubmission);
    }
    Ok(())
}

fn looks_like_email(value: &str) -> bool {
    let value = value.trim();
    if value.len() > 254 || value.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = value.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty()
        && domain.split('.').count() >= 2
        && domain
            .split('.')
            .all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'))
}

fn create_recent_log_archive(
    log_directory: &Path,
) -> Result<(Option<Vec<u8>>, usize), FeedbackError> {
    let entries = fs::read_dir(log_directory).map_err(|_| FeedbackError::LogArchive)?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let is_log = entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log"));
            (metadata.is_file() && is_log).then(|| {
                (
                    entry.path(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.0.file_name().cmp(&left.0.file_name()))
    });
    candidates.truncate(MAX_LOG_FILE_COUNT);
    if candidates.is_empty() {
        return Ok((None, 0));
    }

    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut written_count = 0;
    for (path, _) in candidates {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(error) => {
                log::warn!("feedback_log_file_skipped reason=read_failed error={error}");
                continue;
            }
        };
        writer
            .start_file(file_name, options)
            .map_err(|_| FeedbackError::LogArchive)?;
        writer
            .write_all(&data)
            .map_err(|_| FeedbackError::LogArchive)?;
        written_count += 1;
    }
    if written_count == 0 {
        return Ok((None, 0));
    }
    let archive = writer
        .finish()
        .map_err(|_| FeedbackError::LogArchive)?
        .into_inner();
    Ok((Some(archive), written_count))
}

fn feedback_endpoint() -> String {
    #[cfg(debug_assertions)]
    {
        let override_url = std::env::var("MANGODISK_FEEDBACK_API_URL")
            .ok()
            .or_else(|| option_env!("MANGODISK_FEEDBACK_API_URL").map(str::to_string));
        if let Some(value) = override_url.and_then(|value| loopback_feedback_endpoint(&value)) {
            return value;
        }
    }
    PRODUCTION_FEEDBACK_URL.to_string()
}

#[cfg(debug_assertions)]
fn loopback_feedback_endpoint(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    let host = url.host_str()?;
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    is_loopback.then(|| url.to_string())
}

fn safe_display_name(value: &str) -> String {
    let base_name = value.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned = base_name
        .chars()
        .filter(|character| !character.is_control())
        .take(180)
        .collect::<String>();
    if cleaned.trim().is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

fn attachment_matches_mime_type(mime_type: &str, data: &[u8]) -> bool {
    match mime_type {
        "image/png" => data.starts_with(&[0x89, 0x50, 0x4e, 0x47]),
        "image/jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => {
            data.starts_with(b"RIFF") && data.get(8..12).is_some_and(|value| value == b"WEBP")
        }
        "image/gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        "application/pdf" => data.starts_with(b"%PDF"),
        "application/zip"
        | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06")
        }
        "application/msword" | "application/vnd.ms-excel" | "application/vnd.ms-powerpoint" => {
            data.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
        }
        "application/rtf" => data.starts_with(b"{\\rtf"),
        "text/plain" | "text/markdown" | "text/csv" | "application/json" | "application/yaml"
        | "application/toml" => !data.contains(&0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mangodisk-feedback-{name}-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn real_feedback_transport_reuses_metadata_and_keeps_optional_identity_nonfatal() {
        use crate::services::client_request_metadata::tests::{assert_headers, metadata, serve};
        for install_id in [None, Some(uuid::Uuid::new_v4().to_string())] {
            let mut metadata = metadata();
            metadata.install_id = install_id;
            let (url, server) = serve(|_| vec![("200 OK", b"{}".to_vec(), String::new())]);
            let client = client().no_proxy().build().unwrap();
            let response = send(
                &client,
                &url,
                Form::new().text("payload", "fixture"),
                &metadata,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let requests = server.join().unwrap();
            assert_headers(&requests[0], &metadata);
            assert!(requests[0].contains("multipart/form-data; boundary="));
            assert!(requests[0].contains("fixture"));
        }
    }

    #[test]
    fn attachment_validation_rejects_spoofed_images() {
        assert!(attachment_matches_mime_type("image/png", b"\x89PNGdata"));
        assert!(!attachment_matches_mime_type("image/png", b"not an image"));
        assert!(attachment_matches_mime_type(
            "text/plain",
            b"diagnostic text"
        ));
        assert!(!attachment_matches_mime_type("text/plain", b"binary\0data"));
    }

    #[test]
    fn attachment_validation_accepts_documents_and_rejects_wrong_containers() {
        for mime_type in [
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ] {
            assert!(attachment_matches_mime_type(mime_type, b"PK\x03\x04data"));
            assert!(!attachment_matches_mime_type(mime_type, b"plain text"));
        }
        for mime_type in [
            "application/msword",
            "application/vnd.ms-excel",
            "application/vnd.ms-powerpoint",
        ] {
            assert!(attachment_matches_mime_type(
                mime_type,
                b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1data"
            ));
            assert!(!attachment_matches_mime_type(mime_type, b"PK\x03\x04data"));
        }
        for mime_type in [
            "text/markdown",
            "text/csv",
            "application/json",
            "application/yaml",
            "application/toml",
        ] {
            assert!(attachment_matches_mime_type(mime_type, b"diagnostic text"));
            assert!(!attachment_matches_mime_type(mime_type, b"binary\0data"));
        }
        assert!(attachment_matches_mime_type(
            "application/rtf",
            b"{\\rtf1 report}"
        ));
        assert!(!attachment_matches_mime_type(
            "application/rtf",
            b"plain text"
        ));
        assert!(attachment_matches_mime_type("image/gif", b"GIF89adata"));
        assert!(attachment_matches_mime_type("image/gif", b"GIF87adata"));
        assert!(!attachment_matches_mime_type("image/gif", b"not an image"));
        assert!(!attachment_matches_mime_type(
            "application/octet-stream",
            b"data"
        ));
    }

    #[test]
    fn draft_store_enforces_count_and_removes_files() {
        let directory = test_directory("draft-store");
        let store = FeedbackDraftStore::initialize(&directory);
        let mut tokens = Vec::new();
        for index in 0..MAX_ATTACHMENT_COUNT {
            let item = store
                .stage(
                    format!("image-{index}.png"),
                    "image/png".into(),
                    b"\x89PNGdata".to_vec(),
                )
                .expect("attachment must stage");
            tokens.push(item.token);
        }
        assert!(matches!(
            store.stage(
                "extra.png".into(),
                "image/png".into(),
                b"\x89PNGdata".to_vec()
            ),
            Err(FeedbackError::TooManyAttachments)
        ));
        store.discard(&tokens);
        assert_eq!(fs::read_dir(&store.root).unwrap().count(), 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn draft_store_removes_stale_process_directories() {
        let directory = test_directory("stale-draft");
        let stale_directory = directory.join("feedback-drafts").join("stale-process");
        fs::create_dir_all(&stale_directory).unwrap();
        fs::write(stale_directory.join("attachment.bin"), b"stale").unwrap();

        let store = FeedbackDraftStore::initialize(&directory);
        store.cleanup_stale_drafts();

        assert!(!stale_directory.exists());
        assert!(!store.root.exists());
        store
            .stage(
                "image.png".into(),
                "image/png".into(),
                b"\x89PNGdata".to_vec(),
            )
            .expect("first attachment must create the process draft directory");
        assert!(store.root.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unavailable_draft_storage_does_not_fail_initialization() {
        let directory = test_directory("unavailable-draft");
        fs::write(&directory, b"not a directory").unwrap();

        let store = FeedbackDraftStore::initialize(&directory);

        assert!(matches!(
            store.stage(
                "image.png".into(),
                "image/png".into(),
                b"\x89PNGdata".to_vec()
            ),
            Err(FeedbackError::DraftStorage)
        ));
        fs::remove_file(directory).unwrap();
    }

    #[test]
    fn draft_resolution_rejects_duplicate_tokens() {
        let directory = test_directory("duplicate-token");
        let store = FeedbackDraftStore::initialize(&directory);
        let item = store
            .stage(
                "image.png".into(),
                "image/png".into(),
                b"\x89PNGdata".to_vec(),
            )
            .unwrap();

        assert!(matches!(
            store.resolve(&[item.token.clone(), item.token]),
            Err(FeedbackError::InvalidSubmission)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn log_archive_contains_only_three_recent_log_files() {
        let directory = test_directory("logs");
        fs::create_dir_all(&directory).unwrap();
        for name in ["a.log", "b.log", "c.log", "d.txt", "e.log"] {
            fs::write(directory.join(name), name).unwrap();
        }

        let (archive, count) = create_recent_log_archive(&directory).expect("archive must build");
        assert_eq!(count, 3);
        let mut archive = zip::ZipArchive::new(Cursor::new(archive.unwrap())).unwrap();
        assert_eq!(archive.len(), 3);
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            assert!(!file.name().contains('/') && !file.name().contains('\\'));
            let mut content = String::new();
            file.read_to_string(&mut content).unwrap();
            assert_eq!(content, file.name());
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn metadata_serializes_the_existing_installation_identity() {
        let install_id = Uuid::new_v4().to_string();
        let mut metadata = FeedbackMetadata {
            schema_version: FEEDBACK_SCHEMA_VERSION,
            request_id: "2e71a0e7-ae6d-4f65-b67f-1910c90e4473",
            install_id: Some(&install_id),
            category: "issue",
            content: "A useful feedback report",
            email: None,
            platform: "macos",
            architecture: "aarch64",
            app_version: "1.1.5",
            os_version: "15.7".into(),
            locale: "en-US",
        };
        let value = serde_json::to_value(&metadata).unwrap();
        assert_eq!(value["installId"], install_id);
        metadata.install_id = None;
        assert!(serde_json::to_value(&metadata).unwrap()["installId"].is_null());
    }

    #[test]
    fn submission_validation_matches_the_website_contract() {
        let mut request = SubmitFeedbackRequest {
            request_id: Uuid::new_v4().to_string(),
            install_id: Some(Uuid::new_v4().to_string()),
            category: "issue".into(),
            content: "A useful feedback report".into(),
            email: Some("person@example.com".into()),
            locale: "en-US".into(),
            include_logs: true,
            attachment_tokens: Vec::new(),
        };
        assert!(validate_submission(&request).is_ok());

        request.install_id = Some("invalid-install-id".into());
        assert!(matches!(
            validate_submission(&request),
            Err(FeedbackError::InvalidSubmission)
        ));
        request.install_id = None;
        assert!(validate_submission(&request).is_ok());

        request.email = Some("person@team@example.com".into());
        assert!(matches!(
            validate_submission(&request),
            Err(FeedbackError::InvalidSubmission)
        ));
        request.email = None;
        request.content = "😀".repeat(2_501);
        assert!(matches!(
            validate_submission(&request),
            Err(FeedbackError::InvalidSubmission)
        ));
        request.content = "A useful feedback report".into();
        request.locale = "x".repeat(33);
        assert!(matches!(
            validate_submission(&request),
            Err(FeedbackError::InvalidSubmission)
        ));
    }

    #[test]
    fn debug_endpoint_override_accepts_parsed_loopback_hosts_only() {
        assert!(loopback_feedback_endpoint("http://localhost:3000/api/feedback").is_some());
        assert!(loopback_feedback_endpoint("http://127.0.0.1:3000/api/feedback").is_some());
        assert!(loopback_feedback_endpoint("http://[::1]:3000/api/feedback").is_some());
        assert!(loopback_feedback_endpoint("https://localhost:3000/api/feedback").is_none());
        assert!(loopback_feedback_endpoint("http://localhost:3000@evil.example/api").is_none());
    }
}
