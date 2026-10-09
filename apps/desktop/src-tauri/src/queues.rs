//! Queue commands and scheduler notifications.

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;
use velox_scheduler::Scheduler;
use velox_types::{PostAction, PowerHold, QueueInfo, QueueUpdate, SchedulerEvent};

use crate::error::{CmdResult, CommandError};
use crate::AppState;

/// Forward scheduler events to the UI; finished queues may trigger their
/// post-completion action.
pub fn spawn_events(app: AppHandle, scheduler: &Scheduler) {
    let mut rx = scheduler.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let _ = app.emit("app://scheduler", &ev);
            if let SchedulerEvent::QueueFinished {
                name,
                post_action,
                completed,
                failed,
                ..
            } = &ev
            {
                let settings = app.state::<AppState>().manager.settings();
                if settings.general.notify_on_complete {
                    let body = if *failed > 0 {
                        format!("{completed} completed, {failed} failed")
                    } else {
                        format!("{completed} completed")
                    };
                    let _ = app
                        .notification()
                        .builder()
                        .title(format!("Queue \"{name}\" finished"))
                        .body(body)
                        .show();
                }
                if *post_action != PostAction::None {
                    crate::post_action::begin(&app, *post_action, name);
                }
            }
        }
    });
}

#[tauri::command]
pub fn list_queues(s: State<'_, Scheduler>) -> CmdResult<Vec<QueueInfo>> {
    Ok(s.queues()?)
}

#[tauri::command]
pub fn create_queue(s: State<'_, Scheduler>, name: String) -> CmdResult<QueueInfo> {
    Ok(s.create_queue(&name)?)
}

#[tauri::command]
pub fn update_queue(
    s: State<'_, Scheduler>,
    id: String,
    update: QueueUpdate,
) -> CmdResult<QueueInfo> {
    Ok(s.update_queue(&id, update)?)
}

#[tauri::command]
pub async fn delete_queue(s: State<'_, Scheduler>, id: String) -> CmdResult<()> {
    Ok(s.delete_queue(&id).await?)
}

#[tauri::command]
pub async fn start_queue(s: State<'_, Scheduler>, id: String) -> CmdResult<()> {
    Ok(s.start_queue(&id).await?)
}

#[tauri::command]
pub async fn stop_queue(s: State<'_, Scheduler>, id: String) -> CmdResult<()> {
    Ok(s.stop_queue(&id).await?)
}

/// Move downloads to a queue.
#[tauri::command]
pub fn set_download_queue(
    state: State<'_, AppState>,
    s: State<'_, Scheduler>,
    ids: Vec<String>,
    queue_id: String,
) -> CmdResult<()> {
    if !s.queues()?.iter().any(|q| q.id == queue_id) {
        return Err(CommandError::new("not_found", "queue not found"));
    }
    for id in ids {
        state.manager.set_queue(Uuid::parse_str(&id)?, &queue_id)?;
    }
    s.wake();
    Ok(())
}

#[tauri::command]
pub fn get_power_hold(s: State<'_, Scheduler>) -> Option<PowerHold> {
    s.power_hold()
}
