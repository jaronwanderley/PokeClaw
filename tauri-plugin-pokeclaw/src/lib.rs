use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn ping() -> String {
    "pong from rust".into()
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    let builder = Builder::new("pokeclaw");
    
    #[cfg(target_os = "android")]
    let builder = builder.setup(|_app, api| {
        api.register_android_plugin("io.agents.pokeclaw", "PokeclawPlugin")?;
        Ok(())
    });

    #[cfg(not(target_os = "android"))]
    let builder = builder.invoke_handler(tauri::generate_handler![ping]);

    builder.build()
}
