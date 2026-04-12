pub mod agent;
pub mod commands;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Mutex;

/// Shared state for agent commands. Managed via tauri::State.
pub struct AgentState {
    /// In-memory OpenAI API key, set via `set_openai_api_key` command.
    pub openai_api_key: Mutex<Option<String>>,
    /// Shared cancel flag. Set to true by `cancel_task`, read by the agent loop.
    pub running_task_cancel: Arc<AtomicBool>,
    /// Quick check whether a task is currently active.
    pub task_running: Arc<AtomicBool>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            openai_api_key: Mutex::new(None),
            running_task_cancel: Arc::new(AtomicBool::new(false)),
            task_running: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_pokeclaw::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AgentState::default())
        .setup(|app| {
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
            commands::test_agent_round,
            commands::set_openai_api_key,
            commands::start_task,
            commands::cancel_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
