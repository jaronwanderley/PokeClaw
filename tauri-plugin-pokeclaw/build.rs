const COMMANDS: &[&str] = &["ping", "chat"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
