use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifier of the built-in default queue.
pub const MAIN_QUEUE_ID: &str = "main";

/// Action performed after a queue finishes all its downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PostAction {
    #[default]
    None,
    /// Quit the application.
    Exit,
    /// Put the computer to sleep.
    Sleep,
    /// Hibernate the computer.
    Hibernate,
    /// Shut the computer down.
    Shutdown,
}

/// Time window during which a queue may start downloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct Schedule {
    pub enabled: bool,
    /// Local start time "HH:MM".
    pub start_time: Option<String>,
    /// Local stop time "HH:MM"; running downloads are paused at this time.
    pub stop_time: Option<String>,
    /// Days of week the schedule is active (0 = Sunday ... 6 = Saturday).
    /// Empty = every day.
    pub days: Vec<u8>,
}

/// A download queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueueInfo {
    pub id: String,
    pub name: String,
    /// Maximum downloads of this queue running at once (1 = sequential).
    pub max_concurrent: u32,
    /// Queue processing is running.
    pub running: bool,
    pub schedule: Schedule,
    pub post_action: PostAction,
    /// Retry failed items automatically when the queue processes them.
    pub retry_failed: bool,
    pub sort_order: i32,
    pub built_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct QueueUpdate {
    pub name: Option<String>,
    pub max_concurrent: Option<u32>,
    pub schedule: Option<Schedule>,
    pub post_action: Option<PostAction>,
    pub retry_failed: Option<bool>,
}

/// Why queue processing is held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PowerHold {
    /// Running on battery below the configured threshold.
    LowBattery,
    /// The network connection is metered.
    Metered,
}

/// Notifications from the scheduler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SchedulerEvent {
    /// Queue definitions or their running state changed.
    QueuesChanged { queues: Vec<QueueInfo> },
    /// A started queue has no more waiting or running downloads.
    QueueFinished {
        queue_id: String,
        name: String,
        post_action: PostAction,
        completed: u32,
        failed: u32,
    },
    /// Queue processing is held back (`Some`) or allowed again (`None`).
    PowerHold { hold: Option<PowerHold> },
}
