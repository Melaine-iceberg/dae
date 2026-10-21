//! Small shell-level answers the renderer needs before it can paint.
//!
//! These are the commands still owned by the Electron shell rather than
//! `dae-core`: system places are an OS lookup, and the workspace lists are
//! empty until the shell's stores are ported. They are served here so the real
//! renderer has a coherent first paint instead of four failed queries.

use serde::Serialize;

#[derive(Serialize)]
pub struct SystemPlace {
    kind: &'static str,
    path: String,
}

pub fn system_places() -> Vec<SystemPlace> {
    let candidates = [
        ("home", dirs::home_dir()),
        ("desktop", dirs::desktop_dir()),
        ("documents", dirs::document_dir()),
        ("downloads", dirs::download_dir()),
        ("pictures", dirs::picture_dir()),
        ("music", dirs::audio_dir()),
        ("videos", dirs::video_dir()),
    ];

    candidates
        .into_iter()
        .filter_map(|(kind, resolved)| {
            resolved.map(|path| SystemPlace {
                kind,
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect()
}
