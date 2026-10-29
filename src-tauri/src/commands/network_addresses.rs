//! Thin IPC adapter for the application-owned public-address cache.
use super::error::CommandResult;
use crate::resident::public_network::{PublicNetwork, Request, Snapshot};
use std::sync::Arc;

#[tauri::command]
pub async fn monitoring_network_addresses(
    state: tauri::State<'_, Arc<PublicNetwork>>,
    request: Request,
) -> CommandResult<Snapshot> {
    Ok(state.inner().request(request).await)
}
