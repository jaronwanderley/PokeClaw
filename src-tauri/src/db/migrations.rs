use rusqlite_migration::{Migrations, M};

/// Defines the initial database schema with 4 tables and 3 indexes.
pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "\
            CREATE TABLE IF NOT EXISTS chat_messages (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id      TEXT    NOT NULL,
                role            TEXT    NOT NULL,
                content         TEXT    NOT NULL,
                metadata        TEXT,
                created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS tasks (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                task_text       TEXT    NOT NULL,
                status          TEXT    NOT NULL DEFAULT 'running',
                model_name      TEXT    NOT NULL DEFAULT '',
                answer          TEXT,
                error           TEXT,
                iteration_count INTEGER NOT NULL DEFAULT 0,
                total_tokens    INTEGER NOT NULL DEFAULT 0,
                total_cost_usd  REAL    NOT NULL DEFAULT 0.0,
                tool_count      INTEGER NOT NULL DEFAULT 0,
                created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
                completed_at    TEXT
            );

            CREATE TABLE IF NOT EXISTS task_events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id         INTEGER NOT NULL REFERENCES tasks(id),
                event_type      TEXT    NOT NULL,
                event_data      TEXT,
                created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS agent_state (
                key             TEXT    PRIMARY KEY,
                value           TEXT    NOT NULL,
                updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_chat_session    ON chat_messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_tasks_status    ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id);
            ",
        ),
    ])
}
