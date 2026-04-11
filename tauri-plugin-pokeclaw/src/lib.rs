use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn ping() -> String {
    "pong from rust".into()
}

#[tauri::command]
fn chat(message: String) -> String {
    log::info!("chat command received: {}", message);
    format!("Echo: {}", message)
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    let builder = Builder::new("pokeclaw");
    
    #[cfg(target_os = "android")]
    let builder = builder
        .setup(|_app, api| {
            api.register_android_plugin("io.agents.pokeclaw", "PokeclawPlugin")?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![chat]);

    #[cfg(not(target_os = "android"))]
    let builder = builder.invoke_handler(tauri::generate_handler![ping, chat]);

    builder.build()
}
