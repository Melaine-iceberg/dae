mod file_system;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let specta = specta_builder();

    let app = tauri::Builder::default()
        .manage(file_system::DirectoryWatcher::default())
        .manage(file_system::FileSearchState::default())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_snap_layout::init()
                .button_id("window-maximize")
                .build(),
        );

    #[cfg(debug_assertions)]
    let app = app
        .plugin(tauri_plugin_devtools::init())
        .plugin(tauri_plugin_dev_invoke::init());

    app.invoke_handler(specta.invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Creates the shared command registry for Tauri and TypeScript binding export.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![
        file_system::directory::get_home_directory,
        file_system::directory::read_directory,
        file_system::directory::watch_directory,
        file_system::search::search_directory,
        file_system::search::cancel_search,
        file_system::operations::rename_entry,
        file_system::operations::create_entry,
        file_system::operations::copy_entries,
        file_system::operations::move_entries,
        file_system::operations::delete_entries
    ])
}
