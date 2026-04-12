pub mod chat;
pub mod migrations;
pub mod state;
pub mod tasks;

use std::path::Path;

use rusqlite::Connection;
use rusqlite_migration::Migrations;

/// Wrapper around a rusqlite connection providing CRUD operations for
/// chat messages, tasks, task events, and agent state.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open a file-backed database at the given path.
    pub fn new(path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(path).map_err(|e| format!("Failed to open database at {:?}: {}", path, e))?;
        log::info!("Database opened at {:?}", path);
        Ok(Self { conn })
    }

    /// Open an in-memory database (for unit tests).
    pub fn new_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("Failed to open in-memory DB: {}", e))?;
        Ok(Self { conn })
    }

    /// Run all pending migrations. Safe to call multiple times.
    pub fn run_migrations(&mut self) -> Result<(), String> {
        let migrations: Migrations = migrations::migrations();
        migrations
            .to_latest(&mut self.conn())
            .map_err(|e| format!("Migration failed: {}", e))?;
        log::info!("Database migrations applied successfully");
        Ok(())
    }

    /// Mutable reference to the underlying connection (needed by migrations).
    fn conn(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Immutable reference for queries.
    pub fn conn_ref(&self) -> &Connection {
        &self.conn
    }
}
