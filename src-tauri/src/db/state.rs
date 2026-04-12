use rusqlite::OptionalExtension;

use crate::db::Database;

impl Database {
    /// Get a value from the agent_state table. Returns None if the key does not exist.
    pub fn get_state(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn_ref();
        let mut stmt = conn
            .prepare("SELECT value FROM agent_state WHERE key = ?1")
            .map_err(|e| format!("get_state prepare: {}", e))?;
        let result = stmt
            .query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| format!("get_state query key={}: {}", key, e))?;
        Ok(result)
    }

    /// Set a key-value pair in agent_state. Overwrites existing values.
    pub fn set_state(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn_ref()
            .execute(
                "INSERT INTO agent_state (key, value, updated_at) VALUES (?1, ?2, datetime('now')) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                rusqlite::params![key, value],
            )
            .map_err(|e| format!("set_state key={}: {}", key, e))?;
        log::debug!("Set agent_state key='{}'", key);
        Ok(())
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
    fn get_missing_key_returns_none() {
        let db = test_db();
        assert!(db.get_state("nonexistent").unwrap().is_none());
    }

    #[test]
    fn set_and_get() {
        let db = test_db();
        db.set_state("session_id", "abc-123").unwrap();
        assert_eq!(db.get_state("session_id").unwrap(), Some("abc-123".to_string()));
    }

    #[test]
    fn set_overwrites_existing() {
        let db = test_db();
        db.set_state("key", "v1").unwrap();
        db.set_state("key", "v2").unwrap();
        assert_eq!(db.get_state("key").unwrap(), Some("v2".to_string()));
    }

    #[test]
    fn multiple_keys_independent() {
        let db = test_db();
        db.set_state("a", "1").unwrap();
        db.set_state("b", "2").unwrap();
        assert_eq!(db.get_state("a").unwrap(), Some("1".to_string()));
        assert_eq!(db.get_state("b").unwrap(), Some("2".to_string()));
    }
}
