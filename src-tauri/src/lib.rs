mod file_system;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let specta = specta_builder();

    #[cfg(debug_assertions)]
    specta
        .export(
            specta_typescript::Typescript::default(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts"),
        )
        .expect("Failed to export TypeScript bindings");

    let app = tauri::Builder::default()
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            file_system::connections::init(app.handle())?;
            Ok(())
        })
        .manage(file_system::DirectoryWatcher::default())
        .manage(file_system::FileSearchState::default())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
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

    app.run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Creates the shared command registry for Tauri and TypeScript binding export.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        // Millisecond timestamps, byte sizes, and counters all stay below 2^53,
        // so exporting u64 as `number` loses no precision.
        .dangerously_cast_bigints_to_number()
        .commands(tauri_specta::collect_commands![
            file_system::commands::get_home_directory,
            file_system::commands::read_directory,
            file_system::commands::watch_directory,
            file_system::commands::search_directory,
            file_system::commands::cancel_search,
            file_system::commands::rename_entry,
            file_system::commands::create_entry,
            file_system::commands::copy_entries,
            file_system::commands::move_entries,
            file_system::commands::delete_entries,
            file_system::commands::duplicate_entries,
            file_system::commands::open_terminal,
            file_system::archive::compress_entries,
            file_system::archive::extract_archive,
            file_system::preview::get_thumbnail,
            file_system::preview::read_text_preview,
            file_system::sidebar::get_system_places,
            file_system::sidebar::list_disks,
            file_system::sidebar::list_wsl_distros,
            file_system::sidebar::load_favorites,
            file_system::sidebar::save_favorites,
            file_system::recents::list_recents,
            file_system::recents::record_recent,
            file_system::recents::remove_recent,
            file_system::recents::clear_recents,
            file_system::spaces::list_spaces,
            file_system::spaces::create_space,
            file_system::spaces::rename_space,
            file_system::spaces::delete_space,
            file_system::spaces::add_space_item,
            file_system::spaces::remove_space_item,
            file_system::connections::list_connections,
            file_system::connections::save_connection,
            file_system::connections::delete_connection,
            file_system::smb::test_connection
        ])
        .events(tauri_specta::collect_events![
            file_system::DirectoryChanged,
            file_system::progress::FileOperationProgress
        ])
}
