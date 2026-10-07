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
//!
//! Scenario 4 is gated separately by `DAE_POC_REALTAB=1` and moves a webview
//! that runs the real app bundle inside `window_material` windows, since the
//! three scenarios above only ever move a static page. See `run_real_tab`.

use std::time::Duration;

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl, Window};

/// Window the webview starts in.
const SRC_LABEL: &str = "poc-src";
/// Window the webview is reparented into.
const DST_LABEL: &str = "poc-dst";
/// Label of the webview itself; deliberately different from both windows so a
/// label mix-up cannot be mistaken for correct behaviour.
const WEBVIEW_LABEL: &str = "poc-webview";
/// Scenario 2: a window built with `WebviewWindowBuilder`, i.e. the shape
/// `tab_windows.rs` uses, whose webview is created by the builder rather than
/// by `add_child`.
const WW_SRC: &str = "poc-ww-src";
/// Scenario 2 target, also a `WebviewWindow`.
const WW_DST: &str = "poc-ww-dst";
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
        if let Err(error) = run_webview_windows(&app).await {
            note(&serde_json::json!({ "event": "ww-failed", "error": error }));
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

/// Scenario 2: the shape the app actually uses. Both windows are built the way
/// `tab_windows.rs` builds its tear-off windows — `WebviewWindowBuilder`, not
/// `Window` + `add_child` — and the handles come from the manager, because
/// `WebviewWindow` exposes no `&Window` accessor and `reparent` wants one.
///
/// With `unstable` on, `reparent` skips its `is_webview_window()` guard
/// entirely, so this asks whether the cheap migration is viable: keep every
/// `WebviewWindow` (and with it `window_material` and the GTK drop target),
/// and move only the webview.
async fn run_webview_windows(app: &tauri::AppHandle) -> Result<(), String> {
    let src = tauri::WebviewWindowBuilder::new(app, WW_SRC, WebviewUrl::App(PAGE.into()))
        .title("WW source")
        .inner_size(WIDTH, HEIGHT)
        .position(60.0, 560.0)
        .decorations(false)
        .build()
        .map_err(|error| format!("build {WW_SRC}: {error}"))?;

    let dst = tauri::WebviewWindowBuilder::new(app, WW_DST, WebviewUrl::App(PAGE.into()))
        .title("WW target")
        .inner_size(WIDTH + 120.0, HEIGHT + 80.0)
        .position(660.0, 560.0)
        .decorations(false)
        .build()
        .map_err(|error| format!("build {WW_DST}: {error}"))?;

    sleep_ms(1500).await;

    // The two handles the migration would actually hold.
    let webview = app
        .get_webview(WW_SRC)
        .ok_or_else(|| format!("no webview registered as {WW_SRC}"))?;
    let target = app
        .get_window(WW_DST)
        .ok_or_else(|| format!("no window registered as {WW_DST}"))?;

    note(&serde_json::json!({
        "event": "ww-built",
        "src_webviews": src.webviews().len(),
        "dst_webviews": dst.webviews().len(),
        "src_has_window": app.get_window(WW_SRC).is_some(),
        "src_has_webview": app.get_webview(WW_SRC).is_some(),
    }));

    eval(&webview, "window.__poc_mark('ww-before')");
    sleep_ms(700).await;

    let result = webview.reparent(&target);
    note(&serde_json::json!({
        "event": "ww-reparent",
        "result": match &result {
            Ok(()) => "ok".to_string(),
            Err(error) => error.to_string(),
        },
    }));
    result.map_err(|error| format!("ww reparent: {error}"))?;

    sleep_ms(1200).await;
    note(&serde_json::json!({
        "event": "ww-post",
        "webview_window": webview.window().label(),
        "src_webviews": app.get_window(WW_SRC).map(|w| w.webviews().len()),
        "dst_webviews": app.get_window(WW_DST).map(|w| w.webviews().len()),
        "all_webviews": app
            .webviews()
            .into_iter()
            .map(|(label, w)| format!("{label}@{}", w.window().label()))
            .collect::<Vec<_>>(),
    }));

    // The page must still be the same one, on the same JS context.
    eval(&webview, "window.__poc_mark('ww-after')");
    sleep_ms(1200).await;

    // Round trip: drag it back. A tear-off that cannot come home is not usable.
    let src_window = app
        .get_window(WW_SRC)
        .ok_or_else(|| format!("no window registered as {WW_SRC}"))?;
    let back = webview.reparent(&src_window);
    note(&serde_json::json!({
        "event": "ww-reparent-back",
        "result": match &back {
            Ok(()) => "ok".to_string(),
            Err(error) => error.to_string(),
        },
    }));
    back.map_err(|error| format!("ww reparent back: {error}"))?;

    sleep_ms(1000).await;
    eval(&webview, "window.__poc_mark('ww-back')");
    sleep_ms(1000).await;
    note(&serde_json::json!({
        "event": "ww-back-post",
        "webview_window": webview.window().label(),
        "src_webviews": app.get_window(WW_SRC).map(|w| w.webviews().len()),
        "dst_webviews": app.get_window(WW_DST).map(|w| w.webviews().len()),
    }));

    // Now the emptied shell: the tab that was dragged out has come home, so the
    // window it left behind holds no webview. Closing it must not take the live
    // webview down with it — the failure mode that would make this unusable.
    let closed = match app.get_window(WW_DST) {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Err("already gone".to_string()),
    };
    note(&serde_json::json!({
        "event": "ww-close-emptied",
        "result": match &closed {
            Ok(()) => "ok".to_string(),
            Err(error) => error.clone(),
        },
    }));

    sleep_ms(1500).await;
    note(&serde_json::json!({
        "event": "ww-after-close",
        "dst_still_registered": app.get_window(WW_DST).is_some(),
        "webview_window": webview.window().label(),
        "src_webviews": app.get_window(WW_SRC).map(|w| w.webviews().len()),
        "all_webviews": app
            .webviews()
            .into_iter()
            .map(|(label, w)| format!("{label}@{}", w.window().label()))
            .collect::<Vec<_>>(),
    }));
    // Still reachable after the sibling window is gone?
    eval(&webview, "window.__poc_mark('ww-after-close')");
    sleep_ms(1200).await;

    Ok(())
}

/// Scenario 4: the bet the migration actually makes. The three scenarios above
/// move a static probe page, but a real tab is a webview running the app bundle
/// (React tree, jotai store, mounted listeners) inside a `window_material`
/// window. Here both windows are built the way `tear_off_tab` builds one, and
/// the state probe is injected with `eval` instead of living in the page, so a
/// reload would reset `boot` and restart `ticks` just the same.
///
/// `DAE_POC_REALTAB=1` gates it separately, so the static-page results stay
/// reproducible on their own.
const RT_SRC: &str = "poc-rt-src";
/// The window that ends up hosting both webviews, i.e. the shape the target
/// architecture needs: one window, a shell webview plus a moved-in content webview.
const RT_DST: &str = "poc-rt-dst";

pub fn real_tab_enabled() -> bool {
    std::env::var("DAE_POC_REALTAB").is_ok_and(|value| value == "1" || value == "true")
}

pub fn spawn_real_tab(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        std::thread::sleep(Duration::from_secs(2));
        note(&serde_json::json!({ "event": "rt-start" }));
        if let Err(error) = run_real_tab(&app).await {
            note(&serde_json::json!({ "event": "rt-failed", "error": error }));
        }
        note(&serde_json::json!({ "event": "rt-done" }));
    });
}

/// Asks the real page to report the state kept on `window.__rt`, plus the
/// geometry and label the page believes it has.
fn probe(webview: &tauri::Webview, stage: &str) {
    let script = format!(
        "(function (stage) {{
          var internals = window.__TAURI_INTERNALS__ || {{}};
          if (!window.__rt) {{
            window.__rt = {{ boot: Math.random().toString(36).slice(2, 10), ticks: 0 }};
            setInterval(function () {{ window.__rt.ticks += 1; }}, 50);
          }}
          var meta = internals.metadata || {{}};
          internals.invoke('poc_report', {{ data: JSON.stringify({{
            event: 'rt-' + stage,
            boot: window.__rt.boot,
            ticks: window.__rt.ticks,
            nodes: document.getElementsByTagName('*').length,
            viewport: [window.innerWidth, window.innerHeight],
            screen: [window.screenX, window.screenY],
            dpr: window.devicePixelRatio,
            scrollY: Math.round(window.scrollY),
            title: document.title,
            hasFocus: document.hasFocus(),
            label: (meta.currentWindow || {{}}).label,
            webviewLabel: (meta.currentWebview || {{}}).label,
            href: location.href,
            hasInternals: !!internals.invoke,
          }}) }});
        }})({stage:?})"
    );
    eval(webview, &script);
}

fn window_webviews(app: &tauri::AppHandle, label: &str) -> Option<usize> {
    app.get_window(label).map(|window| window.webviews().len())
}

async fn run_real_tab(app: &tauri::AppHandle) -> Result<(), String> {
    let build = |label: &str, title: &str, x: f64, width: f64| {
        crate::window_material::configure(
            tauri::WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
                .title(title)
                .inner_size(width, 620.0)
                .position(x, 120.0)
                .decorations(false),
        )
        .build()
        .map(|window| window.label().to_owned())
        .map_err(|error| format!("build {label}: {error}"))
    };
    build(RT_SRC, "RT source", 80.0, 780.0)?;
    build(RT_DST, "RT target", 920.0, 900.0)?;

    // The bundle has to finish booting React before any of this means anything.
    sleep_ms(6000).await;

    let webview = app
        .get_webview(RT_SRC)
        .ok_or_else(|| format!("no webview registered as {RT_SRC}"))?;
    let target = app
        .get_window(RT_DST)
        .ok_or_else(|| format!("no window registered as {RT_DST}"))?;

    note(&serde_json::json!({
        "event": "rt-built",
        "src_webviews": window_webviews(app, RT_SRC),
        "dst_webviews": window_webviews(app, RT_DST),
    }));

    probe(&webview, "before");
    sleep_ms(1500).await;

    let moved = webview.reparent(&target);
    note(&serde_json::json!({
        "event": "rt-reparent",
        "result": match &moved {
            Ok(()) => "ok".to_string(),
            Err(error) => error.to_string(),
        },
    }));
    moved.map_err(|error| format!("rt reparent: {error}"))?;

    sleep_ms(2000).await;
    note(&serde_json::json!({
        "event": "rt-post",
        "webview_window": webview.window().label(),
        "src_webviews": window_webviews(app, RT_SRC),
        "dst_webviews": window_webviews(app, RT_DST),
        "all_webviews": app
            .webviews()
            .into_iter()
            .map(|(label, w)| format!("{label}@{}", w.window().label()))
            .collect::<Vec<_>>(),
    }));
    probe(&webview, "after");
    sleep_ms(1500).await;

    // Drag it home, then out again, so the shell can be closed while genuinely
    // empty — the case scenario 3 never actually reached.
    let source = app
        .get_window(RT_SRC)
        .ok_or_else(|| format!("no window registered as {RT_SRC}"))?;
    let back = webview.reparent(&source);
    note(&serde_json::json!({
        "event": "rt-reparent-back",
        "result": match &back {
            Ok(()) => "ok".to_string(),
            Err(error) => error.to_string(),
        },
    }));
    back.map_err(|error| format!("rt reparent back: {error}"))?;
    sleep_ms(1500).await;
    probe(&webview, "back");
    sleep_ms(1000).await;

    let target = app
        .get_window(RT_DST)
        .ok_or_else(|| format!("no window registered as {RT_DST}"))?;
    webview
        .reparent(&target)
        .map_err(|error| format!("rt reparent out again: {error}"))?;
    sleep_ms(1500).await;
    probe(&webview, "out-again");

    note(&serde_json::json!({
        "event": "rt-before-close-shell",
        "src_webviews": window_webviews(app, RT_SRC),
    }));
    let closed = match app.get_window(RT_SRC) {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Err("already gone".to_string()),
    };
    note(&serde_json::json!({
        "event": "rt-close-empty-shell",
        "result": match &closed {
            Ok(()) => "ok".to_string(),
            Err(error) => error.clone(),
        },
    }));

    sleep_ms(2500).await;
    note(&serde_json::json!({
        "event": "rt-after-close",
        "src_still_registered": app.get_window(RT_SRC).is_some(),
        "webview_window": webview.window().label(),
        "dst_webviews": window_webviews(app, RT_DST),
        "all_webviews": app
            .webviews()
            .into_iter()
            .map(|(label, w)| format!("{label}@{}", w.window().label()))
            .collect::<Vec<_>>(),
    }));
    probe(&webview, "after-close");
    sleep_ms(1500).await;

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
