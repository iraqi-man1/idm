//! Download commands invoked by the UI.

use std::path::Path;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;
use velox_core::DownloadManager;
use velox_types::{
    AddDownloadRequest, BatchAddResult, BatchFailure, ChecksumAlgorithm, ChecksumResult,
    ChecksumSpec, DownloadInfo, DownloadStatus, LogLine, ProgressSnapshot, StartMode, UrlInfo,
};

use crate::error::{CmdResult, CommandError};
use crate::AppState;

fn parse_id(id: &str) -> CmdResult<Uuid> {
    Ok(Uuid::parse_str(id)?)
}

fn info(state: &AppState, id: Uuid) -> CmdResult<DownloadInfo> {
    state
        .manager
        .get(id)
        .ok_or_else(|| CommandError::new("not_found", "download not found"))
}

#[tauri::command]
pub fn list_downloads(state: State<'_, AppState>) -> Vec<DownloadInfo> {
    state.manager.list()
}

#[tauri::command]
pub fn get_download(state: State<'_, AppState>, id: String) -> CmdResult<DownloadInfo> {
    info(&state, parse_id(&id)?)
}

#[tauri::command]
pub async fn probe_url(
    state: State<'_, AppState>,
    request: AddDownloadRequest,
) -> CmdResult<UrlInfo> {
    Ok(state.manager.probe_url(&request).await?)
}

#[tauri::command]
pub async fn add_download(
    state: State<'_, AppState>,
    request: AddDownloadRequest,
) -> CmdResult<DownloadInfo> {
    Ok(state.manager.add(request).await?)
}

/// Add many URLs at once (batch import / "download all links").
#[tauri::command]
pub async fn add_batch(
    state: State<'_, AppState>,
    urls: Vec<String>,
    save_dir: Option<String>,
    start: StartMode,
    queue_id: Option<String>,
    referer: Option<String>,
) -> CmdResult<BatchAddResult> {
    let mut added = Vec::new();
    let mut failed = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for url in urls
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
    {
        if !seen.insert(url.clone()) {
            continue;
        }
        if let Err(e) = velox_types::protocol::check_download_url("url", &url) {
            failed.push(BatchFailure {
                url,
                error: e.to_string(),
            });
            continue;
        }
        let req = AddDownloadRequest {
            url: url.clone(),
            save_dir: save_dir.clone(),
            start,
            queue_id: queue_id.clone(),
            referer: referer.clone(),
            source: velox_types::AddSource::Batch,
            ..Default::default()
        };
        match state.manager.add(req).await {
            Ok(d) => added.push(d),
            Err(e) => failed.push(BatchFailure {
                url,
                error: e.to_string(),
            }),
        }
    }
    Ok(BatchAddResult { added, failed })
}

#[tauri::command]
pub fn start_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.manager.start(parse_id(&id)?)?)
}

#[tauri::command]
pub async fn pause_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.manager.pause(parse_id(&id)?).await?)
}

#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.manager.cancel(parse_id(&id)?).await?)
}

#[tauri::command]
pub async fn restart_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.manager.restart(parse_id(&id)?).await?)
}

#[tauri::command]
pub async fn remove_downloads(
    state: State<'_, AppState>,
    ids: Vec<String>,
    delete_files: bool,
) -> CmdResult<()> {
    let mut first_err = None;
    for id in ids {
        let id = parse_id(&id)?;
        if let Err(e) = state.manager.remove(id, delete_files).await {
            first_err.get_or_insert(e);
        }
    }
    match first_err {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn start_many(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    for id in ids {
        state.manager.start(parse_id(&id)?)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pause_many(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    for id in ids {
        state.manager.pause(parse_id(&id)?).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pause_all(state: State<'_, AppState>) -> CmdResult<()> {
    state.manager.pause_all().await;
    Ok(())
}

/// Start every paused or failed (recoverable) download.
pub fn resume_all_impl(mgr: &DownloadManager) {
    for d in mgr.list() {
        let resumable_failure = d.status == DownloadStatus::Failed
            && matches!(
                d.error_kind,
                Some(velox_types::ErrorKind::Network) | Some(velox_types::ErrorKind::Http)
            );
        if d.status == DownloadStatus::Paused || resumable_failure {
            if let Err(e) = mgr.start(d.id) {
                tracing::warn!(id = %d.id, error = %e, "resume failed");
            }
        }
    }
}

#[tauri::command]
pub fn resume_all(state: State<'_, AppState>) {
    resume_all_impl(&state.manager);
}

#[tauri::command]
pub fn open_file(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let d = info(&state, parse_id(&id)?)?;
    if d.status != DownloadStatus::Completed {
        return Err(CommandError::new(
            "not_completed",
            "the download is not complete",
        ));
    }
    let path = d.file_path();
    if !path.exists() {
        return Err(CommandError::new(
            "missing",
            format!("{} no longer exists", path.display()),
        ));
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| CommandError::new("open_failed", e.to_string()))
}

#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let d = info(&state, parse_id(&id)?)?;
    let path = d.file_path();
    if path.exists() {
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| CommandError::new("open_failed", e.to_string()))
    } else {
        let dir = Path::new(&d.save_dir);
        if !dir.exists() {
            return Err(CommandError::new(
                "missing",
                format!("{} does not exist", dir.display()),
            ));
        }
        app.opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| CommandError::new("open_failed", e.to_string()))
    }
}

#[tauri::command]
pub async fn move_download(
    state: State<'_, AppState>,
    id: String,
    directory: String,
) -> CmdResult<DownloadInfo> {
    Ok(state
        .manager
        .move_completed(parse_id(&id)?, &directory)
        .await?)
}

#[tauri::command]
pub fn rename_download(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> CmdResult<DownloadInfo> {
    Ok(state.manager.rename(parse_id(&id)?, &name)?)
}

#[tauri::command]
pub fn update_download_url(state: State<'_, AppState>, id: String, url: String) -> CmdResult<()> {
    velox_types::protocol::check_download_url("url", &url)
        .map_err(|e| CommandError::new("invalid_url", e.0))?;
    Ok(state.manager.update_url(parse_id(&id)?, &url)?)
}

#[tauri::command]
pub fn set_download_speed_limit(
    state: State<'_, AppState>,
    id: String,
    limit: u64,
) -> CmdResult<()> {
    Ok(state.manager.set_speed_limit(parse_id(&id)?, limit)?)
}

#[tauri::command]
pub fn set_download_connections(
    state: State<'_, AppState>,
    id: String,
    connections: u8,
) -> CmdResult<DownloadInfo> {
    let id = parse_id(&id)?;
    Ok(state
        .manager
        .modify(id, |r| r.max_connections = connections.clamp(1, 32))?)
}

#[tauri::command]
pub async fn verify_checksum(
    state: State<'_, AppState>,
    id: String,
    algorithm: Option<ChecksumAlgorithm>,
    expected: String,
) -> CmdResult<ChecksumResult> {
    let expected = expected.trim().to_ascii_lowercase();
    if expected.is_empty() || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CommandError::new(
            "invalid_checksum",
            "the checksum must be a hexadecimal string",
        ));
    }
    let algorithm = algorithm
        .or_else(|| ChecksumAlgorithm::from_hex_len(expected.len()))
        .ok_or_else(|| CommandError::new("invalid_checksum", "unknown checksum length"))?;
    let (ok, actual) = state
        .manager
        .verify_checksum(
            parse_id(&id)?,
            Some(ChecksumSpec {
                algorithm,
                expected,
            }),
        )
        .await?;
    Ok(ChecksumResult { ok, actual })
}

#[tauri::command]
pub fn get_download_log(state: State<'_, AppState>, id: String) -> CmdResult<Vec<LogLine>> {
    Ok(state
        .manager
        .log(parse_id(&id)?)?
        .into_iter()
        .map(|l| LogLine {
            ts: l.ts,
            level: l.level,
            message: l.message,
        })
        .collect())
}

#[tauri::command]
pub fn get_speed_history(state: State<'_, AppState>, id: String) -> CmdResult<Vec<u64>> {
    Ok(state.manager.speed_history(parse_id(&id)?))
}

#[tauri::command]
pub fn get_progress(state: State<'_, AppState>) -> Vec<ProgressSnapshot> {
    state.manager.progress_snapshots()
}

#[tauri::command]
pub fn find_duplicates(state: State<'_, AppState>, url: String) -> Vec<String> {
    state
        .manager
        .find_duplicates(&url)
        .into_iter()
        .map(|id| id.to_string())
        .collect()
}
