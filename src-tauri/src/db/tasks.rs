use serde::{Deserialize, Serialize};

/// A persisted task record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: i64,
    pub task_text: String,
    pub status: String,
    pub model_name: String,
    pub answer: Option<String>,
    pub error: Option<String>,
    pub iteration_count: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub tool_count: i64,
    pub created_at: String,
    pub completed_at: Option<String>,
}

/// A persisted task event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEventRecord {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub event_data: Option<String>,
    pub created_at: String,
}

use crate::db::Database;

impl Database {
    /// Insert a new task and return its row ID.
    pub fn insert_task(&self, task_text: &str, model_name: &str) -> Result<i64, String> {
        let conn = self.conn_ref();
        conn.execute(
            "INSERT INTO tasks (task_text, model_name) VALUES (?1, ?2)",
            rusqlite::params![task_text, model_name],
        )
        .map_err(|e| format!("insert_task: {}", e))?;
        let id = conn.last_insert_rowid();
        log::debug!("Inserted task id={} text='{}'", id, task_text);
        Ok(id)
    }

    /// Update a task's status and completion fields.
    pub fn update_task_status(
        &self,
        id: i64,
        status: &str,
        answer: Option<&str>,
        error: Option<&str>,
        iteration_count: i64,
        total_tokens: i64,
        total_cost_usd: f64,
        tool_count: i64,
        completed_at: Option<&str>,
    ) -> Result<(), String> {
        self.conn_ref()
            .execute(
                "UPDATE tasks SET status=?1, answer=?2, error=?3, iteration_count=?4, \
                 total_tokens=?5, total_cost_usd=?6, tool_count=?7, completed_at=?8 \
                 WHERE id=?9",
                rusqlite::params![
                    status,
                    answer,
                    error,
                    iteration_count,
                    total_tokens,
                    total_cost_usd,
                    tool_count,
                    completed_at,
                    id,
                ],
            )
            .map_err(|e| format!("update_task_status id={}: {}", id, e))?;
        log::debug!("Updated task id={} status={}", id, status);
        Ok(())
    }

    /// List recent tasks, ordered by created_at descending.
    pub fn list_tasks(&self, limit: i64) -> Result<Vec<TaskRecord>, String> {
        let conn = self.conn_ref();
        let mut stmt = conn
            .prepare(
                "SELECT id, task_text, status, model_name, answer, error, \
                 iteration_count, total_tokens, total_cost_usd, tool_count, \
                 created_at, completed_at \
                 FROM tasks ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| format!("list_tasks prepare: {}", e))?;
        let rows = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(TaskRecord {
                    id: row.get(0)?,
                    task_text: row.get(1)?,
                    status: row.get(2)?,
                    model_name: row.get(3)?,
                    answer: row.get(4)?,
                    error: row.get(5)?,
                    iteration_count: row.get(6)?,
                    total_tokens: row.get(7)?,
                    total_cost_usd: row.get(8)?,
                    tool_count: row.get(9)?,
                    created_at: row.get(10)?,
                    completed_at: row.get(11)?,
                })
            })
            .map_err(|e| format!("list_tasks query: {}", e))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| format!("list_tasks row: {}", e))?);
        }
        Ok(results)
    }

    /// Insert a task event and return its row ID.
    pub fn insert_task_event(
        &self,
        task_id: i64,
        event_type: &str,
        event_data: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn_ref();
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, event_data) VALUES (?1, ?2, ?3)",
            rusqlite::params![task_id, event_type, event_data],
        )
        .map_err(|e| format!("insert_task_event: {}", e))?;
        let id = conn.last_insert_rowid();
        log::debug!("Inserted task_event id={} task_id={} type={}", id, task_id, event_type);
        Ok(id)
    }

    /// List all events for a given task, ordered by created_at ascending.
    pub fn list_task_events(&self, task_id: i64) -> Result<Vec<TaskEventRecord>, String> {
        let conn = self.conn_ref();
        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, event_type, event_data, created_at \
                 FROM task_events WHERE task_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| format!("list_task_events prepare: {}", e))?;
        let rows = stmt
            .query_map(rusqlite::params![task_id], |row| {
                Ok(TaskEventRecord {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    event_type: row.get(2)?,
                    event_data: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| format!("list_task_events query: {}", e))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| format!("list_task_events row: {}", e))?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        let mut db = Database::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        db
    }

    #[test]
    fn insert_and_list_tasks() {
        let db = test_db();
        let id = db.insert_task("open whatsapp", "gpt-4o").unwrap();
        assert!(id > 0);

        let tasks = db.list_tasks(10).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_text, "open whatsapp");
        assert_eq!(tasks[0].status, "running");
        assert_eq!(tasks[0].model_name, "gpt-4o");
    }

    #[test]
    fn update_task_status_transition() {
        let db = test_db();
        let id = db.insert_task("send message", "gpt-4o").unwrap();

        db.update_task_status(
            id,
            "completed",
            Some("done"),
            None,
            5,
            1200,
            0.03,
            3,
            Some("2026-04-12T10:00:00"),
        )
        .unwrap();

        let tasks = db.list_tasks(10).unwrap();
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.status, "completed");
        assert_eq!(t.answer, Some("done".to_string()));
        assert!(t.error.is_none());
        assert_eq!(t.iteration_count, 5);
        assert_eq!(t.total_tokens, 1200);
        assert!((t.total_cost_usd - 0.03).abs() < f64::EPSILON);
        assert_eq!(t.tool_count, 3);
        assert_eq!(t.completed_at, Some("2026-04-12T10:00:00".to_string()));
    }

    #[test]
    fn update_task_error() {
        let db = test_db();
        let id = db.insert_task("failing task", "model").unwrap();
        db.update_task_status(id, "failed", None, Some("timeout"), 1, 100, 0.01, 0, Some("2026-04-12T10:01:00"))
            .unwrap();

        let tasks = db.list_tasks(10).unwrap();
        assert_eq!(tasks[0].status, "failed");
        assert_eq!(tasks[0].error, Some("timeout".to_string()));
    }

    #[test]
    fn list_tasks_limit() {
        let db = test_db();
        for i in 0..5 {
            db.insert_task(&format!("task {}", i), "m").unwrap();
        }
        let limited = db.list_tasks(3).unwrap();
        assert_eq!(limited.len(), 3);
        // All 5 tasks exist; limit caps the result set
        let all = db.list_tasks(100).unwrap();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn insert_and_list_task_events() {
        let db = test_db();
        let task_id = db.insert_task("task", "m").unwrap();

        let e1 = db.insert_task_event(task_id, "tool_call", Some("{\"name\":\"tap\"}")).unwrap();
        let e2 = db.insert_task_event(task_id, "tool_result", Some("{\"ok\":true}")).unwrap();
        assert!(e2 > e1);

        let events = db.list_task_events(task_id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "tool_call");
        assert_eq!(events[1].event_type, "tool_result");
    }

    #[test]
    fn task_events_isolated_per_task() {
        let db = test_db();
        let t1 = db.insert_task("t1", "m").unwrap();
        let t2 = db.insert_task("t2", "m").unwrap();
        db.insert_task_event(t1, "start", None).unwrap();
        db.insert_task_event(t2, "start", None).unwrap();

        assert_eq!(db.list_task_events(t1).unwrap().len(), 1);
        assert_eq!(db.list_task_events(t2).unwrap().len(), 1);
    }
}
