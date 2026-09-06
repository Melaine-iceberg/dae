//! Startup self-update. Driven entirely from Rust, so no capability permission
//! and no frontend surface are involved.

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Checks the release feed in the background and, if a newer signed bundle is
/// published, installs it and restarts so the swap takes effect.
pub fn spawn_startup_check(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match check_and_install(&app).await {
            Ok(true) => app.restart(),
            Ok(false) => {}
            // A missing endpoint config (a local release build without the
            // `tauri.release.conf.json` overlay) lands here too, which is why
            // this stays a log line rather than a user-facing failure.
            Err(error) => eprintln!("Update check failed: {error}"),
        }
    });
}

async fn check_and_install(
    app: &AppHandle,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(false);
    };
    update.download_and_install(|_, _| {}, || {}).await?;
    Ok(true)
}
