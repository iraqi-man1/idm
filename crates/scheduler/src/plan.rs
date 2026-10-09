//! Which waiting downloads to start now (pure, unit tested).

use std::collections::{HashMap, HashSet};

use velox_types::{DownloadId, DownloadInfo, DownloadStatus, QueueInfo, MAIN_QUEUE_ID};

/// The queue a download belongs to (`main` when unset).
pub fn queue_of(d: &DownloadInfo) -> &str {
    d.queue_id.as_deref().unwrap_or(MAIN_QUEUE_ID)
}

pub struct Inputs<'a> {
    pub queues: &'a [QueueInfo],
    pub downloads: &'a [DownloadInfo],
    pub running: &'a HashSet<DownloadId>,
    /// Maximum downloads running at once across all queues.
    pub global_limit: usize,
    pub now_ms: i64,
    /// Power policy holds automatic starts back.
    pub hold: bool,
}

/// Downloads to start, in order.
///
/// * A download with a due `scheduled_at` starts on its own (it was
///   scheduled explicitly), within the global limit.
/// * A started queue starts its waiting downloads (highest priority first,
///   then oldest) while both its own and the global limit allow.
/// * Downloads scheduled for later are never started by queue processing.
pub fn plan(i: &Inputs<'_>) -> Vec<DownloadId> {
    if i.hold {
        return Vec::new();
    }
    let mut total = i.running.len();
    let mut per_queue: HashMap<&str, usize> = HashMap::new();
    for d in i.downloads.iter().filter(|d| i.running.contains(&d.id)) {
        *per_queue.entry(queue_of(d)).or_default() += 1;
    }
    let waiting =
        |d: &&DownloadInfo| d.status == DownloadStatus::Queued && !i.running.contains(&d.id);
    let mut out = Vec::new();

    let mut due: Vec<&DownloadInfo> = i
        .downloads
        .iter()
        .filter(waiting)
        .filter(|d| d.scheduled_at.is_some_and(|t| t <= i.now_ms))
        .collect();
    due.sort_by_key(|d| (d.scheduled_at, d.created_at));
    for d in due {
        if total >= i.global_limit {
            return out;
        }
        out.push(d.id);
        total += 1;
        *per_queue.entry(queue_of(d)).or_default() += 1;
    }

    let mut queues: Vec<&QueueInfo> = i.queues.iter().filter(|q| q.running).collect();
    queues.sort_by_key(|q| (q.sort_order, q.name.clone()));
    for q in queues {
        let mut items: Vec<&DownloadInfo> = i
            .downloads
            .iter()
            .filter(waiting)
            .filter(|d| d.scheduled_at.is_none() && queue_of(d) == q.id)
            .collect();
        items.sort_by_key(|d| (std::cmp::Reverse(d.priority), d.created_at));
        for d in items {
            let in_queue = per_queue.get(q.id.as_str()).copied().unwrap_or(0);
            if total >= i.global_limit {
                return out;
            }
            if in_queue >= q.max_concurrent as usize {
                break;
            }
            out.push(d.id);
            total += 1;
            *per_queue.entry(q.id.as_str()).or_default() += 1;
        }
    }
    out
}

/// A started queue with nothing left to do: no waiting (unscheduled)
/// download and none of its downloads running.
pub fn queue_idle(
    q: &QueueInfo,
    downloads: &[DownloadInfo],
    running: &HashSet<DownloadId>,
) -> bool {
    !downloads.iter().any(|d| {
        queue_of(d) == q.id
            && (running.contains(&d.id)
                || (d.status == DownloadStatus::Queued && d.scheduled_at.is_none()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use velox_types::{PostAction, Schedule};

    fn queue(id: &str, max: u32, running: bool, order: i32) -> QueueInfo {
        QueueInfo {
            id: id.into(),
            name: id.into(),
            max_concurrent: max,
            running,
            schedule: Schedule::default(),
            post_action: PostAction::None,
            retry_failed: false,
            sort_order: order,
            built_in: id == MAIN_QUEUE_ID,
        }
    }

    fn dl(n: u128, q: &str, status: DownloadStatus, priority: i32, created: i64) -> DownloadInfo {
        let mut d: DownloadInfo = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::from_u128(n), "url": "http://x/", "final_url": null, "page_url": null,
            "file_name": "f", "save_dir": "/d", "kind": "http", "status": "queued", "category": "other",
            "total_size": null, "downloaded": 0, "resumable": null, "max_connections": 8,
            "active_connections": 0, "speed": 0, "avg_speed": 0, "eta_secs": null, "error": null,
            "error_kind": null, "mime": null, "referer": null, "queue_id": null, "priority": 0,
            "created_at": 0, "started_at": null, "completed_at": null, "scheduled_at": null,
            "next_retry_at": null, "retry_count": 0, "speed_limit": 0, "checksum": null,
            "checksum_ok": null, "media": null, "elapsed_ms": 0, "has_secrets": false
        }))
        .unwrap();
        d.queue_id = Some(q.into());
        d.status = status;
        d.priority = priority;
        d.created_at = created;
        d
    }

    fn id(n: u128) -> DownloadId {
        uuid::Uuid::from_u128(n)
    }

    #[test]
    fn respects_queue_and_global_limits_and_order() {
        let queues = [queue("main", 2, true, 0), queue("night", 3, true, 1)];
        let downloads = vec![
            dl(1, "main", DownloadStatus::Queued, 0, 30),
            dl(2, "main", DownloadStatus::Queued, 5, 40), // higher priority first
            dl(3, "main", DownloadStatus::Queued, 0, 10),
            dl(4, "night", DownloadStatus::Queued, 0, 1),
            dl(5, "night", DownloadStatus::Paused, 0, 1),
        ];
        let running = HashSet::new();
        let i = Inputs {
            queues: &queues,
            downloads: &downloads,
            running: &running,
            global_limit: 10,
            now_ms: 0,
            hold: false,
        };
        assert_eq!(plan(&i), vec![id(2), id(3), id(4)]);
        let i = Inputs {
            global_limit: 2,
            ..i
        };
        assert_eq!(plan(&i), vec![id(2), id(3)]);
    }

    #[test]
    fn running_downloads_count_against_limits() {
        let queues = [queue("main", 2, true, 0)];
        let downloads = vec![
            dl(1, "main", DownloadStatus::Downloading, 0, 1),
            dl(2, "main", DownloadStatus::Queued, 0, 2),
            dl(3, "main", DownloadStatus::Queued, 0, 3),
        ];
        let running: HashSet<_> = [id(1)].into();
        let i = Inputs {
            queues: &queues,
            downloads: &downloads,
            running: &running,
            global_limit: 10,
            now_ms: 0,
            hold: false,
        };
        assert_eq!(plan(&i), vec![id(2)]);
        // A manual download in another queue uses a global slot.
        let i = Inputs {
            global_limit: 1,
            ..i
        };
        assert!(plan(&i).is_empty());
    }

    #[test]
    fn stopped_queues_and_power_hold_start_nothing() {
        let queues = [queue("main", 2, false, 0)];
        let downloads = vec![dl(1, "main", DownloadStatus::Queued, 0, 1)];
        let running = HashSet::new();
        let i = Inputs {
            queues: &queues,
            downloads: &downloads,
            running: &running,
            global_limit: 10,
            now_ms: 0,
            hold: false,
        };
        assert!(plan(&i).is_empty());
        let queues = [queue("main", 2, true, 0)];
        let i = Inputs {
            queues: &queues,
            hold: true,
            ..i
        };
        assert!(plan(&i).is_empty());
    }

    #[test]
    fn scheduled_downloads_start_when_due_even_in_stopped_queues() {
        let queues = [queue("main", 1, false, 0)];
        let mut a = dl(1, "main", DownloadStatus::Queued, 0, 1);
        a.scheduled_at = Some(1_000);
        let mut b = dl(2, "main", DownloadStatus::Queued, 0, 2);
        b.scheduled_at = Some(5_000);
        let downloads = vec![a, b];
        let running = HashSet::new();
        let i = Inputs {
            queues: &queues,
            downloads: &downloads,
            running: &running,
            global_limit: 10,
            now_ms: 2_000,
            hold: false,
        };
        assert_eq!(plan(&i), vec![id(1)]);
        // Not yet due items are not taken by a running queue either.
        let queues = [queue("main", 5, true, 0)];
        let i = Inputs {
            queues: &queues,
            now_ms: 0,
            ..i
        };
        assert!(plan(&i).is_empty());
    }

    #[test]
    fn idle_detection() {
        let q = queue("main", 2, true, 0);
        let downloads = vec![
            dl(1, "main", DownloadStatus::Completed, 0, 1),
            dl(2, "main", DownloadStatus::Failed, 0, 1),
        ];
        assert!(queue_idle(&q, &downloads, &HashSet::new()));
        assert!(!queue_idle(&q, &downloads, &[id(1)].into()));
        let downloads = vec![dl(3, "main", DownloadStatus::Queued, 0, 1)];
        assert!(!queue_idle(&q, &downloads, &HashSet::new()));
    }
}
