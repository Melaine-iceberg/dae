use tauri::App;

pub fn app_notification(app: &mut App, body: impl Into<String>) {
    use tauri_plugin_notification::NotificationExt;
    let handle = app.handle().clone();

    handle.notification()
        .builder()
        .title("dae")
        .body(body)
        .show()
        .unwrap();
}