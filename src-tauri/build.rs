fn main() {
    tauri_build::build();

    // The Windows event loop runs nested work on the main thread — WebView2
    // script-completion callbacks, OLE's drag loop, and tracing-subscriber's
    // recursive span close — and debug frames are large. A cross-window tab
    // drag used to overflow the default 1 MB stack (STATUS_STACK_OVERFLOW)
    // while draining a chain of ~2300 pending `wry::eval` spans: 1 MB /
    // 2343 frames = 447 bytes per frame.
    //
    // This is a *reservation*, not a commit: the pages are only touched if the
    // stack actually grows, so 32 MB costs nothing in steady state.
    if std::env::var("TARGET")
        .map(|target| target.ends_with("-pc-windows-msvc"))
        .unwrap_or(false)
    {
        println!("cargo:rustc-link-arg-bins=/STACK:33554432");
    }
}
