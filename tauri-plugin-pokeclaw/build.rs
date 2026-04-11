const COMMANDS: &[&str] = &[
    "ping",
    "chat",
    "start_session",
    "stop_session",
    "send_message",
    "get_session_status",
    "list_models",
    "download_model",
    "get_screen_info",
    "find_node_info",
    "get_device_info",
    "check_permissions",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
