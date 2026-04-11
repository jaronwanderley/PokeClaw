const COMMANDS: &[&str] = &[
    "ping",
    "chat",
    "start_session",
    "stop_session",
    "send_message",
    "get_session_status",
    "list_models",
    "download_model",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
