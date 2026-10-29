use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use tauri::{ipc::InvokeBody, AppHandle, Manager, State};

use crate::services::feedback::{
    FeedbackDraftStore, FeedbackSubmissionService, StagedFeedbackAttachment, SubmitFeedbackRequest,
    SubmitFeedbackResult,
};

use super::error::{into_command_result, run_blocking, CommandResult};

const FILE_NAME_HEADER: &str = "x-mangodisk-file-name";
const MIME_TYPE_HEADER: &str = "x-mangodisk-mime-type";

#[tauri::command]
pub async fn stage_feedback_attachment(
    request: tauri::ipc::Request<'_>,
    store: State<'_, FeedbackDraftStore>,
) -> CommandResult<StagedFeedbackAttachment> {
    let data = match request.body() {
        InvokeBody::Raw(data) => data.clone(),
        InvokeBody::Json(_) => {
            return into_command_result(
                "stage_feedback_attachment",
                Err(crate::services::feedback::FeedbackError::InvalidAttachment),
            );
        }
    };
    let display_name = request
        .headers()
        .get(FILE_NAME_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_else(|| "attachment".to_string());
    let mime_type = request
        .headers()
        .get(MIME_TYPE_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let store = store.inner().clone();

    run_blocking("stage_feedback_attachment", move || {
        store.stage(display_name, mime_type, data)
    })
    .await
}

#[tauri::command]
pub async fn discard_feedback_attachments(
    tokens: Vec<String>,
    store: State<'_, FeedbackDraftStore>,
) -> CommandResult<()> {
    let store = store.inner().clone();
    run_blocking("discard_feedback_attachments", move || {
        store.discard(&tokens);
        Ok::<(), crate::services::feedback::FeedbackError>(())
    })
    .await
}

#[tauri::command]
pub async fn submit_feedback(
    app: AppHandle,
    mut request: SubmitFeedbackRequest,
    store: State<'_, FeedbackDraftStore>,
) -> CommandResult<SubmitFeedbackResult> {
    let log_directory = into_command_result("submit_feedback", app.path().app_log_dir())?;
    let app_version = app.package_info().version.to_string();
    let mut metadata = crate::services::client_request_metadata::collect(&app);
    metadata.locale = request.locale.clone();
    metadata.request_id = Some(request.request_id.clone());
    // Reuse the frontend's freshly persisted identity if the native store read was unavailable.
    metadata.install_id = metadata.install_id.or_else(|| request.install_id.clone());
    request.install_id = metadata.install_id.clone();
    into_command_result(
        "submit_feedback",
        FeedbackSubmissionService::submit(
            store.inner(),
            &log_directory,
            &app_version,
            request,
            &metadata,
        )
        .await,
    )
}
