use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[tauri::command]
fn ping() -> String {
    "pong".into()
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("pokeclaw")
        .invoke_handler(tauri::generate_handler![ping])
        .build()
}
