use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;
use crate::utils::app_notification::app_notification;

mod utils;

struct CurFilepath(PathBuf);

impl CurFilepath {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let specta = specta_builder();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(debug_assertions)]
    let app = app
        .plugin(tauri_plugin_devtools::init())
        .plugin(tauri_plugin_dev_invoke::init());

    app.invoke_handler(specta.invoke_handler())
        .setup(|app| {
            let home = match app.path().home_dir() {
                Ok(path) => path,
                Err(err) => {
                    app_notification(
                        app,
                        format!("Failed to retrieve the current system user directory: {err}"),
                    );
                    std::env::current_dir()?
                }
            };
            app.manage(Mutex::new(CurFilepath::new(home)));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Creates the shared command registry for Tauri and TypeScript binding export.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![])
}
