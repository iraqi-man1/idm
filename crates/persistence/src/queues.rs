use rusqlite::{params, OptionalExtension, Row};
use velox_types::{PostAction, QueueInfo, Schedule, MAIN_QUEUE_ID};

use crate::{Database, DbError, DbResult};

fn post_action_str(p: PostAction) -> &'static str {
    match p {
        PostAction::None => "none",
        PostAction::Exit => "exit",
        PostAction::Sleep => "sleep",
        PostAction::Hibernate => "hibernate",
        PostAction::Shutdown => "shutdown",
    }
}

fn parse_post_action(s: &str) -> PostAction {
    match s {
        "exit" => PostAction::Exit,
        "sleep" => PostAction::Sleep,
        "hibernate" => PostAction::Hibernate,
        "shutdown" => PostAction::Shutdown,
        _ => PostAction::None,
    }
}

fn row_to_queue(r: &Row<'_>) -> rusqlite::Result<QueueInfo> {
    let schedule_json: String = r.get("schedule_json")?;
    let post: String = r.get("post_action")?;
    Ok(QueueInfo {
        id: r.get("id")?,
        name: r.get("name")?,
        max_concurrent: r.get::<_, i64>("max_concurrent")?.clamp(1, 64) as u32,
        running: r.get("running")?,
        schedule: serde_json::from_str::<Schedule>(&schedule_json).unwrap_or_default(),
        post_action: parse_post_action(&post),
        retry_failed: r.get("retry_failed")?,
        sort_order: r.get("sort_order")?,
        built_in: r.get("built_in")?,
    })
}

impl Database {
    pub fn list_queues(&self) -> DbResult<Vec<QueueInfo>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached("SELECT * FROM queues ORDER BY sort_order, name")?;
            let rows = stmt.query_map([], row_to_queue)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    pub fn get_queue(&self, id: &str) -> DbResult<Option<QueueInfo>> {
        self.with(|c| {
            Ok(c.prepare_cached("SELECT * FROM queues WHERE id = ?1")?
                .query_row([id], row_to_queue)
                .optional()?)
        })
    }

    pub fn save_queue(&self, q: &QueueInfo) -> DbResult<()> {
        let schedule = serde_json::to_string(&q.schedule)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO queues (id, name, max_concurrent, running, schedule_json, post_action,
                                     retry_failed, sort_order, built_in)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name,
                    max_concurrent=excluded.max_concurrent, running=excluded.running,
                    schedule_json=excluded.schedule_json, post_action=excluded.post_action,
                    retry_failed=excluded.retry_failed, sort_order=excluded.sort_order",
                params![
                    q.id,
                    q.name,
                    q.max_concurrent as i64,
                    q.running,
                    schedule,
                    post_action_str(q.post_action),
                    q.retry_failed,
                    q.sort_order,
                    q.built_in
                ],
            )?;
            Ok(())
        })
    }

    /// Delete a user queue; its downloads move to the main queue.
    pub fn delete_queue(&self, id: &str) -> DbResult<()> {
        if id == MAIN_QUEUE_ID {
            return Err(DbError::Corrupt("the main queue cannot be deleted".into()));
        }
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("UPDATE downloads SET queue_id = ?1 WHERE queue_id = ?2", params![MAIN_QUEUE_ID, id])?;
            tx.execute("DELETE FROM queues WHERE id = ?1 AND built_in = 0", [id])?;
            tx.commit()?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_queue_exists_and_crud_works() {
        let db = Database::open_in_memory().unwrap();
        let qs = db.list_queues().unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].id, MAIN_QUEUE_ID);
        assert!(qs[0].built_in);

        let q = QueueInfo {
            id: "night".into(),
            name: "Night".into(),
            max_concurrent: 1,
            running: false,
            schedule: Schedule {
                enabled: true,
                start_time: Some("02:00".into()),
                stop_time: Some("07:00".into()),
                days: vec![1, 2, 3],
            },
            post_action: PostAction::Shutdown,
            retry_failed: true,
            sort_order: 1,
            built_in: false,
        };
        db.save_queue(&q).unwrap();
        assert_eq!(db.get_queue("night").unwrap().unwrap(), q);
        assert!(db.delete_queue(MAIN_QUEUE_ID).is_err());
        db.delete_queue("night").unwrap();
        assert!(db.get_queue("night").unwrap().is_none());
    }
}
