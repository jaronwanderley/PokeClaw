pub mod agent;
pub mod commands;
pub mod db;

use tauri::Manager;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Mutex;
/// LLM provider type selection.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LlmProviderType {
    OpenAi,
    Anthropic,
    Local,
}

impl Default for LlmProviderType {
    fn default() -> Self {
        Self::OpenAi
    }
}

/// Shared state for agent commands. Managed via tauri::State.
pub struct AgentState {
    /// In-memory OpenAI API key, set via `set_openai_api_key` command.
    pub openai_api_key: Mutex<Option<String>>,
    /// In-memory Anthropic API key, set via `set_anthropic_api_key` command.
    pub anthropic_api_key: Mutex<Option<String>>,
    /// Current LLM provider type selection.
    pub llm_provider_type: Mutex<LlmProviderType>,
    /// Shared cancel flag. Set to true by `cancel_task`, read by the agent loop.
    pub running_task_cancel: Arc<AtomicBool>,
    /// Quick check whether a task is currently active.
    pub task_running: Arc<AtomicBool>,
    /// Shared database handle for task/chat persistence.
    pub db: Arc<Mutex<db::Database>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_pokeclaw::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Initialize SQLite database
            let db_arc = {
                let app_data_dir = app
                    .path()
                    .app_data_dir()
                    .expect("Failed to resolve app data directory");
                std::fs::create_dir_all(&app_data_dir).expect("Failed to create app data directory");
                let db_path = app_data_dir.join("pokeclaw.db");
                let mut db = db::Database::new(&db_path)
                    .expect("Failed to open database");
                db.run_migrations()
                    .expect("Failed to run database migrations");
                log::info!("Database initialized at {:?}", db_path);
                Arc::new(Mutex::new(db))
            };

            // Create AgentState with the real DB handle
            let agent_state = AgentState {
                openai_api_key: Mutex::new(None),
                anthropic_api_key: Mutex::new(None),
                llm_provider_type: Mutex::new(LlmProviderType::default()),
                running_task_cancel: Arc::new(AtomicBool::new(false)),
                task_running: Arc::new(AtomicBool::new(false)),
                db: db_arc.clone(),
            };

            // Manage both AgentState and the standalone DB Arc (for chat commands)
            app.manage(agent_state);
            app.manage(db_arc);

            if cfg!(debug_assertions) {
                app.handle().plugin(
                  tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::testAgentRound,
            commands::setOpenAiApiKey,
            commands::setAnthropicApiKey,
            commands::setLlmProviderType,
            commands::startTask,
            commands::cancelTask,
            commands::saveChatMessage,
            commands::loadChatHistory,
            commands::loadTaskHistory,
            commands::loadTaskEvents,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
