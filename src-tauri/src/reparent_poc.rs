//! Throwaway feasibility probe — **not** part of the app.
//!
//! Question: can a tab move between windows by re-parenting the *existing*
//! webview (`Webview::reparent`), the way a browser moves a renderer to another
//! host, instead of `tab_windows`' serialize-payload-and-rebuild handoff?
//!
//! Run with `DAE_POC_REPARENT=1` in a debug build. The probe:
//!
//! 1. builds a plain [`Window`] (`poc-src`) plus a *child* webview via
//!    [`Window::add_child`], loading `reparent-poc.html`;
//! 2. asks the page to report its state (`before`);
//! 3. builds a second plain window (`poc-dst`);
//! 4. `reparent`s the live webview into it;
//! 5. asks the page to report again (`after`) and dumps the Rust-side view.
//!
//! The page reports a random `boot` id on every load, so the three possible
//! outcomes are distinguishable from the log alone:
//!
//! * same `boot` + `ticks` kept counting → same JS context, no reload;
//! * new `boot` + a `load` event in between → the webview reloaded;
//! * no `after` report at all → `eval` no longer reaches the page.
//!
//! The page also sends its own `currentWindow.label`, which is the thing the
//! frontend keys windows by (`src/lib/app-window.ts`), so a stale label there
//! is visible as data rather than as a guess.
//!
//! Both [`Window::add_child`] and [`Webview::reparent`] are `unstable`-gated
//! (see `Cargo.toml`).

use std::time::Duration;

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl, Window};

/// Window the webview starts in.
const SRC_LABEL: &str = "poc-src";
/// Window the webview is reparented into.
const DST_LABEL: &str = "poc-dst";
/// Label of the webview itself; deliberately different from both windows so a
/// label mix-up cannot be mistaken for correct behaviour.
const WEBVIEW_LABEL: &str = "poc-webview";
/// Served from `public/`, so the probe page never boots the real app.
const PAGE: &str = "reparent-poc.html";
/// Survives the shell that started the app, so a failed run is still readable.
const REPORT_PATH: &str = "/tmp/dae-reparent-poc.jsonl";

const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 400.0;

/// `DAE_POC_REPARENT=1` opts in; nothing else about the app changes.
pub fn enabled() -> bool {
    std::env::var("DAE_POC_REPARENT").is_ok_and(|value| value == "1" || value == "true")
}

/// Runs the probe off the main thread: `add_child` blocks on a main-thread
/// round trip, so it must not be called from `setup` itself.
pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        std::thread::sleep(Duration::from_secs(2));
        note(&serde_json::json!({ "event": "poc-start" }));
        if let Err(error) = run(&app).await {
            note(&serde_json::json!({ "event": "poc-failed", "error": error }));
        }
        note(&serde_json::json!({ "event": "poc-done" }));
    });
}

async fn run(app: &tauri::AppHandle) -> Result<(), String> {
    let src = Window::builder(app, SRC_LABEL)
        .title("PoC source")
        .inner_size(WIDTH, HEIGHT)
        .position(60.0, 60.0)
        .build()
        .map_err(|error| format!("build {SRC_LABEL}: {error}"))?;

    let webview = src
        .add_child(
            tauri::webview::WebviewBuilder::new(WEBVIEW_LABEL, WebviewUrl::App(PAGE.into()))
                .auto_resize(),
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(WIDTH, HEIGHT),
        )
        .map_err(|error| format!("add_child: {error}"))?;

    note(&serde_json::json!({
        "event": "built",
        "webview": webview.label(),
        "window": webview.window().label(),
        "src_webviews": src.webviews().len(),
    }));

    // Let the page boot and send its own `load` report before the first mark.
    sleep_ms(1500).await;
    eval(&webview, "window.__poc_mark('before')");

    sleep_ms(600).await;

    let dst = Window::builder(app, DST_LABEL)
        .title("PoC target")
        .inner_size(WIDTH + 120.0, HEIGHT + 80.0)
        .position(660.0, 60.0)
        .build()
        .map_err(|error| format!("build {DST_LABEL}: {error}"))?;

    note(&serde_json::json!({ "event": "before-reparent" }));

    let reparent_result = webview.reparent(&dst);
    note(&serde_json::json!({
        "event": "reparent",
        "result": match &reparent_result {
            Ok(()) => "ok".to_string(),
            Err(error) => error.to_string(),
        },
    }));
    reparent_result.map_err(|error| format!("reparent: {error}"))?;

    // Rust-side view of where things now live.
    note(&serde_json::json!({
        "event": "post-reparent",
        "webview_window": webview.window().label(),
        "src_webviews": src.webviews().len(),
        "dst_webviews": dst.webviews().len(),
        "all_webviews": app
            .webviews()
            .into_iter()
            .map(|(label, w)| format!("{label}@{}", w.window().label()))
            .collect::<Vec<_>>(),
        "src_title": src.title().map_err(|error| error.to_string()),
        "dst_title": dst.title().map_err(|error| error.to_string()),
    }));

    // Same handle, after the move: does `eval` still reach the page?
    sleep_ms(1200).await;
    eval(&webview, "window.__poc_mark('after')");

    // And does the *new* parent's title follow the page's `document.title`?
    sleep_ms(1200).await;
    note(&serde_json::json!({
        "event": "titles-later",
        "src_title": src.title().map_err(|error| error.to_string()),
        "dst_title": dst.title().map_err(|error| error.to_string()),
    }));

    Ok(())
}

fn eval(webview: &tauri::Webview, script: &str) {
    if let Err(error) = webview.eval(script) {
        note(&serde_json::json!({ "event": "eval-failed", "script": script, "error": error.to_string() }));
    }
}

async fn sleep_ms(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

/// Appends one JSON line to the terminal and to [`REPORT_PATH`], so the
/// evidence survives whatever shell launched the app.
fn note(value: &serde_json::Value) {
    let line = value.to_string();
    println!("[poc] {line}");
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(REPORT_PATH)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// Reports land here. Kept outside the specta registry on purpose: this probe
/// must not disturb `src/bindings.ts`.
#[tauri::command]
fn poc_report(data: String) {
    println!("[poc] {data}");
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(REPORT_PATH)
    {
        let _ = writeln!(file, "{data}");
    }
}

pub fn handle_invoke(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    match invoke.message.command() {
        "poc_report" => __cmd__poc_report!(poc_report, invoke),
        _ => false,
    }
}
