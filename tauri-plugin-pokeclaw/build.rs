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
    "tap",
    "swipe",
    "long_press",
    "tap_node",
    "input_text",
    "scroll_to_find",
    "find_and_tap",
    "get_notifications",
    "open_app",
    "system_key",
    "send_chat_message",
    "take_screenshot",
    "clipboard",
    "get_installed_apps",
    "make_call",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
