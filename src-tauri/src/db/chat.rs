use serde::{Deserialize, Serialize};

/// A persisted chat message row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRecord {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: String,
}

use crate::db::Database;

impl Database {
    /// Insert a chat message and return its row ID.
    pub fn insert_chat_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn_ref();
        conn.execute(
            "INSERT INTO chat_messages (session_id, role, content, metadata) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, role, content, metadata],
        )
        .map_err(|e| format!("insert_chat_message: {}", e))?;
        let id = conn.last_insert_rowid();
        log::debug!("Inserted chat_message id={} session={}", id, session_id);
        Ok(id)
    }

    /// List all chat messages for a session, ordered by created_at ascending.
    pub fn list_chat_messages(&self, session_id: &str) -> Result<Vec<ChatMessageRecord>, String> {
        let conn = self.conn_ref();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, metadata, created_at \
                 FROM chat_messages WHERE session_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| format!("list_chat_messages prepare: {}", e))?;
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| {
                Ok(ChatMessageRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| format!("list_chat_messages query: {}", e))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| format!("list_chat_messages row: {}", e))?);
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
    fn insert_and_list_messages() {
        let db = test_db();
        let id1 = db.insert_chat_message("s1", "user", "hello", None).unwrap();
        let id2 = db.insert_chat_message("s1", "assistant", "hi there", Some("{\"model\":\"gpt\"}")).unwrap();
        assert!(id2 > id1);

        let msgs = db.list_chat_messages("s1").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert!(msgs[0].metadata.is_none());
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].metadata, Some("{\"model\":\"gpt\"}".to_string()));
    }

    #[test]
    fn list_empty_session() {
        let db = test_db();
        let msgs = db.list_chat_messages("nonexistent").unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn messages_isolated_per_session() {
        let db = test_db();
        db.insert_chat_message("s1", "user", "a", None).unwrap();
        db.insert_chat_message("s2", "user", "b", None).unwrap();
        db.insert_chat_message("s1", "user", "c", None).unwrap();

        let s1 = db.list_chat_messages("s1").unwrap();
        let s2 = db.list_chat_messages("s2").unwrap();
        assert_eq!(s1.len(), 2);
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].content, "b");
    }
}
