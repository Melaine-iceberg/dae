//! Linux: the freedesktop.org icon theme, resolved the way GTK resolves it.
//!
//! There is no API to call here. Unlike the Windows shell (`IShellItemImageFactory`
//! hands back exactly what Explorer draws) or macOS (`NSWorkspace` likewise),
//! the desktop's file icons are a *search* defined by a spec: pick the theme the
//! user chose, follow its inheritance chain, walk the XDG data roots, and within
//! each theme try the directories its own `index.theme` declares, ranked by how
//! well each one's size fits the one asked for. The result is a filename, and
//! the file is usually an SVG that somebody else has to render.
//!
//! Two properties of that search shaped this file:
//!
//!   * **Nothing here links against GTK.** The `gtk` crate is in the dependency
//!     tree already, but `GtkIconTheme` needs a realized display and has to be
//!     driven from the GTK main thread. This extraction runs on the render pool
//!     specifically so a scrolling listing never blocks the UI — see
//!     [`crate::file_system::preview`] — so a resolver that had to hop to the
//!     main thread would undo the one thing that design is for. Reading the same
//!     files GTK reads costs a parser and buys a thread-safe, display-free,
//!     testable lookup.
//!   * **`Directories=` is the theme's own inventory.** Guessing at fixed
//!     subdirectory names is what the one-off lookup in `shell_commands::linux.rs`
//!     did, and it misses exactly the themes that matter: one that ships only
//!     `scalable/` icons, or one whose places icons live under `32x32/` while its
//!     mimetypes icons live under `48x48/`. Parsing the index is the difference
//!     between "works on the machine it was written on" and works on a user's.
//!
//! SVG answers are rasterized here, at the size the row asked for, and travel
//! as PNG. Handing the vector to the webview does not work: WebKitGTK
//! rasterizes an SVG in an `<img>` at the CSS box rather than at the device
//! pixels and then upscales that bitmap, so a 16 CSS-px row on a 1.5-scale
//! display drew a 16-px render blown up to 24 — a smear where the same vector
//! rendered at 24 by `rsvg-convert` is crisp. gdk-pixbuf is the same librsvg
//! the desktop's own icon loaders drive, and `gtk` is already a dependency, so
//! the crisp render costs no new crate.

use super::FileIcon;
use crate::xdg::{config_roots, data_roots, home_dir};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How far the `Inherits=` walk goes. Real chains are two or three deep; this
/// is the cycle guard, not a working limit.
const MAX_INHERITANCE_DEPTH: usize = 24;

/// Where a theme with no setting points looks. What GNOME ships, and what
/// `hicolor` falls through to anyway.
const DEFAULT_ICON_THEME: &str = "Adwaita";

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// The icon the desktop theme would show for this path.
pub(super) fn extract(path: &str, size: u32, is_dir: bool) -> Option<FileIcon> {
    let names = icon_names_for(Path::new(path), is_dir);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();

    // Contexts in the order a *file* resolves them. A directory's icon lives in
    // `places`, a type's in `mimetypes`, and both themes exist independently —
    // so a theme that has one and not the other is ordinary, not a miss.
    let contexts: &[&str] = if is_dir {
        &["places", "mimetypes"]
    } else {
        &["mimetypes", "places"]
    };

    lookup(&names, contexts, size)
}

/// The icon the theme draws for a whole file type, named by its extension.
///
/// The path-shaped half of [`extract`] is what this drops: the mimetype names
/// come from `mime_guess`, which reads an extension off a string and never the
/// file behind it. That is what lets one answer serve every `.rs` in a column of
/// 800, and it is why no request for a type icon touches the disk.
pub(super) fn extract_type(extension: &str, size: u32) -> Option<FileIcon> {
    let names = type_icon_names(extension, false);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();

    lookup(&names, &["mimetypes", "places"], size)
}

/// Themed icon names (a `.desktop`'s `Icon=`, which is where the context menu
/// and the "Open With" picker get their glyphs) resolved at `size` pixels.
///
/// Fronted onto the same search as the file icons so there is one theme reader
/// in the crate rather than two that disagree about what a theme contains. The
/// context list is wider here because an application may name an icon from any
/// of them, and narrower in ambition because a row that finds nothing just draws
/// without one.
///
/// `name` carries a colon-separated *list*, which is how glib reports one
/// launcher's icon: `dev.zed.Zed.desktop` answers to `dev.zed.Zed` *and* to the
/// `zed` derived from the desktop id, and older themes ship only the shorter one.
/// The order is the theme spec's, so it is preserved; `:` cannot appear inside an
/// icon name, which is what lets the whole list ride in one query parameter.
pub(crate) fn resolve_named_icon(name: &str, size: u32) -> Option<FileIcon> {
    let names = icon_candidates(name);
    if names.is_empty() {
        return None;
    }

    // The spec's context for an application icon is `Applications`, while the
    // *directory* holding them is conventionally named `apps` — so asking for
    // `apps` here matches nothing in any shipped theme. The rest are contexts a
    // launcher's `Icon=` really does name, because packs place their glyphs
    // inconsistently.
    const APP_CONTEXTS: &[&str] = &["applications", "actions", "status", "mimetypes", "places"];
    lookup(&names, APP_CONTEXTS, size)
}

/// The names in one colon-joined icon field, in the order to try them.
fn icon_candidates(name: &str) -> Vec<&str> {
    // `-` is the desktop entry's own spelling for "no icon".
    name.split(':')
        .filter(|candidate| !candidate.is_empty() && *candidate != "-")
        .collect()
}

/// [`resolve_named_icon`]'s bytes wrapped as a `data:` URL.
///
/// Rows that are built as a whole list and handed over in one message — a
/// context menu — take this rather than a `fileicon://` URL, because the protocol
/// would cost the webview a request per row for bytes the backend already holds.
/// A list too long for that is the "Open With" picker, which goes through the
/// protocol instead: a theme's application icons run to hundreds of kilobytes
/// each, so a full list of them does not belong in one `invoke` reply.
pub(crate) fn named_icon_data_url(name: &str, size: u32) -> Option<String> {
    use base64::Engine as _;

    let icon = resolve_named_icon(name, size)?;
    Some(format!(
        "data:{};base64,{}",
        icon.mime,
        base64::engine::general_purpose::STANDARD.encode(&icon.bytes)
    ))
}

/// The icon names to try for one path, best first.
///
/// The type half of the answer comes from [`type_icon_names`]; what a path adds
/// is the directory case, and the executable bit of a name with no extension to
/// key on.
fn icon_names_for(path: &Path, is_dir: bool) -> Vec<String> {
    // A symlinked folder is listed as its own kind but drawn with the plain
    // folder icon, so it belongs on the directory side of this branch.
    if is_dir {
        return ["folder", "inode-directory", "text-x-generic", "unknown"]
            .map(String::from)
            .to_vec();
    }

    let extension = extension_of(path);
    // An extension-less name is where a shell script and a compiled binary live
    // on Unix, and telling them apart takes a permissions read — the one piece of
    // information a type lookup can have from a path but not from an extension.
    let is_executable = extension.is_empty() && is_executable(path);

    type_icon_names(&extension, is_executable)
}

/// The theme names for one file type, most specific first.
///
/// This order is the whole policy, and both routes here ask the same question —
/// one has the extension, the other reads it off a path — because `lookup` takes
/// the first name that resolves anywhere, and a generic name ahead of a specific
/// one draws every PDF with a sheet.
fn type_icon_names(extension: &str, is_executable: bool) -> Vec<String> {
    let mime = mime_guess::from_ext(extension).first();
    let mut names: Vec<String> = Vec::new();

    // `application/pdf` -> `application-pdf`, the escaped form the spec defines
    // for a mimetype icon's filename.
    if let Some(mime) = &mime {
        names.push(mime.essence_str().replace('/', "-"));
    }

    // Archives are the one family whose theme icons are named after the concept
    // rather than the MIME type: `package-x-generic` is what both Adwaita and
    // Breeze draw for `application/zip`.
    if is_archive_extension(extension) {
        names.push("package-x-generic".to_owned());
    }

    // An extension-less executable is what a shell script and a compiled binary
    // look like in a file manager, and drawing either as a generic sheet is
    // visibly wrong next to the one Explorer and Finder both give it.
    if is_executable {
        names.push("application-x-executable".to_owned());
    }

    // The `<top-level>-x-generic` every theme carries, which is also where the
    // spec's parent-mimetype chain lands for a type with no dedicated icon.
    if let Some(generic) = mime.map(|mime| generic_name_for(mime.type_().as_str())) {
        names.push(generic.to_owned());
    }

    names.push("unknown".to_owned());
    names
}

/// The lowercase extension a name is filed under, empty when it has none.
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The generic icon name for a MIME top-level type.
fn generic_name_for(top_level: &str) -> &'static str {
    match top_level {
        "image" => "image-x-generic",
        "audio" => "audio-x-generic",
        "video" => "video-x-generic",
        "font" => "font-x-generic",
        "text" => "text-x-generic",
        _ => "application-x-generic",
    }
}

/// Extensions whose icon in a real theme is `package-x-generic`.
fn is_archive_extension(extension: &str) -> bool {
    matches!(
        extension,
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "tgz" | "deb" | "rpm" | "jar"
    )
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // Only reachable when this file is compiled on a non-Unix host to be
        // type-checked, where the answer is never used.
        let _ = path;
        false
    }
}

// ---------------------------------------------------------------------------
// The search
// ---------------------------------------------------------------------------

/// Looks the first resolvable name up across the theme chain and the data roots.
fn lookup(names: &[&str], contexts: &[&str], size: u32) -> Option<FileIcon> {
    let chain = theme_chain();
    let roots = data_roots();

    // Name outermost, deliberately. GTK trades name preference against size
    // distance in a single score, but the names handed here are a *preference*
    // order (`application-pdf` before `application-x-generic`) and getting that
    // backwards would draw every PDF with a generic sheet.
    for name in names {
        // An absolute `Icon=` value is a path, not a themed name, and no theme
        // search can answer it. Checked once per name rather than per theme.
        if name.starts_with('/') {
            if let Some(icon) = read_icon_file(Path::new(name), size) {
                return Some(icon);
            }
            continue;
        }

        for theme in chain {
            for root in &roots {
                let base = root.join("icons").join(&theme.name);
                for directory in theme.directories_for(contexts, size) {
                    for extension in ["svg", "png"] {
                        let candidate = base
                            .join(directory)
                            .join(format!("{name}.{extension}"));
                        if let Some(icon) = read_icon_file(&candidate, size) {
                            return Some(icon);
                        }
                    }
                }
            }
        }

        // Last resort for this name, and the one place the theme structure is
        // bypassed: packages that never installed a themed icon drop a file into
        // `/usr/share/pixmaps`.
        for extension in ["svg", "png"] {
            let candidate = Path::new("/usr/share/pixmaps").join(format!("{name}.{extension}"));
            if let Some(icon) = read_icon_file(&candidate, size) {
                return Some(icon);
            }
        }
    }

    None
}

/// One theme: its name, and the icon directories it declares.
struct Theme {
    name: String,
    directories: Vec<Directory>,
}

/// One `Directories=` entry, e.g. `48x48/places`.
struct Directory {
    /// The fragment exactly as the theme spells it — it is the path.
    relative: Box<str>,
    context: Box<str>,
    /// Edge length in px; `None` for `scalable` and `literal`.
    size: Option<u32>,
}

impl Theme {
    /// This theme's directories for one of `contexts`, best-fitting `size`
    /// first. Empty when the theme has nothing to offer for that context.
    fn directories_for(&self, contexts: &[&str], size: u32) -> Vec<&str> {
        // Sorted into a `Vec` rather than returned as an iterator: the ordering
        // needs a container to live in, and it is a handful of entries.
        let mut matched: Vec<&Directory> = self
            .directories
            .iter()
            .filter(|directory| contexts.contains(&&*directory.context))
            .collect();
        matched.sort_by_key(|directory| directory.rank_for(size));
        matched.into_iter().map(|directory| &*directory.relative).collect()
    }
}

impl Directory {
    /// Sort key, lower first, against the requested size.
    ///
    /// Scalable always wins. An SVG is not *a* size but every size, and
    /// preferring a 48px PNG over it because 48 is nearer a 22px request than
    /// "infinite" is exactly how a themed desktop ends up drawing soft, upscaled
    /// icons on a HiDPI panel.
    ///
    /// Past that, an oversize raster beats an undersize one: downscaling a 128px
    /// icon to 22px stays legible, blowing a 16px one up to 22px does not.
    fn rank_for(&self, requested: u32) -> (u8, u32) {
        match self.size {
            None => (0, 0),
            Some(size) if size >= requested => (1, size - requested),
            Some(size) => (2, requested - size),
        }
    }
}

/// The theme chain: the user's theme, what it inherits transitively, then
/// `hicolor` — which the spec defines as the base every theme falls through to,
/// and which is therefore appended even when no `Inherits=` names it.
///
/// Cached for the life of the process. A theme switch mid-run is a
/// restart-to-apply event on every desktop that can do it, and the alternative
/// is re-reading and re-parsing a few hundred lines of `index.theme` per lookup.
fn theme_chain() -> &'static Vec<Theme> {
    static CHAIN: OnceLock<Vec<Theme>> = OnceLock::new();

    CHAIN.get_or_init(build_theme_chain)
}

/// The chain, read from disk. Split from [`theme_chain`] so a test can exercise
/// the walk without a process-wide cache to trip over.
fn build_theme_chain() -> Vec<Theme> {
    let roots = data_roots();
    let mut chain = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Breadth-first: an inherited theme's directories are a better answer than
    // `hicolor`'s, so the whole inheritance tree has to be consumed before the
    // base is appended.
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(user_icon_theme().unwrap_or_else(|| DEFAULT_ICON_THEME.to_owned()));
    let mut visited = 0usize;

    while let Some(name) = queue.pop_front() {
        visited += 1;
        if visited > MAX_INHERITANCE_DEPTH || !seen.insert(name.clone()) {
            continue;
        }

        match read_theme(&roots, &name) {
            Some(ReadTheme { theme, inherits }) => {
                queue.extend(inherits);
                chain.push(theme);
            }
            // An uninstalled theme is the ordinary case for a settings value
            // written by a different desktop environment. Skip it, and let
            // `hicolor` below carry the lookup.
            None => continue,
        }
    }

    if seen.insert("hicolor".to_owned())
        && let Some(ReadTheme { theme, .. }) = read_theme(&roots, "hicolor")
    {
        chain.push(theme);
    }

    chain
}

/// A parsed theme plus the names it inherits, kept together so the walk never
/// reads the same `index.theme` twice.
struct ReadTheme {
    theme: Theme,
    inherits: Vec<String>,
}

/// Parses one theme's `index.theme`, from whichever data root has it.
fn read_theme(roots: &[PathBuf], name: &str) -> Option<ReadTheme> {
    roots.iter().find_map(|root| {
        let index = root.join("icons").join(name).join("index.theme");
        let text = std::fs::read_to_string(&index).ok()?;
        Some(parse_theme(name, &text))
    })
}

/// The `index.theme` reader: `[Icon Theme]` for `Directories` and `Inherits`,
/// then one section per directory for its `Context`.
///
/// Hand-written rather than through the `toml` crate already in the tree,
/// because the two are not the same dialect: icon-theme keys are case-*insensitive*,
/// `toml` rejects a bare `[48x48/apps]` table name, and a comment here may start
/// with `#` at any column. Twenty lines of parsing buys that compatibility, which
/// is the cheaper trade.
fn parse_theme(name: &str, text: &str) -> ReadTheme {
    let mut declared: Vec<String> = Vec::new();
    let mut inherits: Vec<String> = Vec::new();
    // `Context=` per section. The section name *is* the directory fragment, so
    // the two halves of the file join without a lookup.
    let mut contexts: Vec<(String, String)> = Vec::new();
    let mut section = String::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            section = header.to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        if section.eq_ignore_ascii_case("Icon Theme") {
            match key.as_str() {
                "directories" => declared = split_list(value),
                "inherits" => inherits = split_list(value),
                _ => {}
            }
        } else if key == "context" {
            contexts.push((section.clone(), value.to_owned()));
        }
    }

    let theme = Theme {
        name: name.to_owned(),
        directories: declared
            .iter()
            .filter_map(|relative| {
                // The fragment's own second half is the context. A `[48x48/places]`
                // section's explicit `Context=places` agrees with it in every
                // theme shipped today, so the section value is only consulted to
                // catch the theme that spells it differently.
                let (size_token, fallback) = relative.split_once('/')?;
                let context = contexts
                    .iter()
                    .find(|(section, _)| section == relative)
                    .map_or(fallback, |(_, context)| context.as_str());
                Some(Directory {
                    relative: relative.as_str().into(),
                    // Lowercased on the way in, so every comparison downstream
                    // is a plain string match. `index.theme` writes these
                    // `Places` / `MimeTypes` and the lookup names are lowercase.
                    context: context.to_ascii_lowercase().into_boxed_str(),
                    size: parse_directory_size(size_token),
                })
            })
            .collect(),
    };

    ReadTheme { theme, inherits }
}

/// The edge length a directory fragment names, or `None` when it names none
/// (`scalable`, `literal`).
fn parse_directory_size(token: &str) -> Option<u32> {
    let (width, _) = token.split_once('x')?;
    width.parse().ok()
}

/// A comma-separated `index.theme` list, trimmed, empties dropped.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The theme the user chose, or `None` when no desktop setting names one.
fn user_icon_theme() -> Option<String> {
    static CACHED: OnceLock<Option<String>> = OnceLock::new();

    CACHED.get_or_init(read_user_icon_theme).clone()
}

/// Reads the icon theme out of the settings files, most widely honored first.
///
/// `dconf` is where GNOME keeps this value and `zbus` is already a dependency,
/// but `gsettings-desktop-schemas` mirrors it into `~/.config/gtk-3.0/`, and
/// reading the file gets the same answer for XFCE, Cinnamon, MATE, LXQt and KDE
/// without a D-Bus round-trip from a render worker. A desktop that writes
/// neither is a desktop with no icon theme set, which is what `None` means and
/// why the caller defaults to Adwaita.
fn read_user_icon_theme() -> Option<String> {
    // (file, key, section-independent) pairs. `kdeglobals` spells the same
    // setting `Theme` under `[Icons]`; GTK spells it `gtk-icon-theme-name`.
    const SOURCES: &[(&str, &str)] = &[
        ("gtk-3.0/settings.ini", "gtk-icon-theme-name"),
        ("gtk-4.0/settings.ini", "gtk-icon-theme-name"),
        ("kdeglobals", "Theme"),
    ];

    for root in config_roots() {
        for (file, key) in SOURCES {
            if let Some(value) = ini_value(&root.join(file), key) {
                return Some(value);
            }
        }
    }

    // The two places a settings file turns up that is not under
    // `XDG_CONFIG_HOME`: GTK 2's rc file in `$HOME`, and an install that put
    // `kdeglobals` there directly.
    let home = home_dir();
    if let Some(value) = ini_value(&home.join(".gtkrc-2.0"), "gtk-icon-theme-name") {
        return Some(value);
    }
    if let Some(value) = ini_value(&home.join(".kde/share/config/kdeglobals"), "Theme") {
        return Some(value);
    }

    std::env::var("GTK_ICON_THEME").ok().filter(|value| !value.is_empty())
}

/// One key from an INI-ish settings file, matched case-insensitively and
/// ignoring which section it landed in.
///
/// Section-blind on purpose: `kdeglobals` puts `Theme` under `[Icons]` and GTK
/// puts `gtk-icon-theme-name` under `[Settings]`, so naming the sections here
/// would mean knowing each desktop's layout. The keys are distinctive enough to
/// identify the file that matters.
fn ini_value(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let wanted = key.to_ascii_lowercase();

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') || line.starts_with('#') {
            continue;
        }
        let Some((found, value)) = line.split_once('=') else {
            continue;
        };
        if found.trim().eq_ignore_ascii_case(&wanted) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }

    None
}

/// The MIME type a file on disk has to be served as, or `None` for an extension
/// no webview renders. `xpm` is deliberately absent: it is still in some themes,
/// and a row with a broken image is worse than a row with the app's own glyph.
fn mime_for_extension(extension: &str) -> Option<&'static str> {
    match extension {
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        _ => None,
    }
}

/// Reads an icon file off disk and labels it. A zero-byte or unreadable file is
/// a miss, not an error — themes are dropped and half-installed often enough
/// that one broken file must not blank a whole directory.
///
/// An SVG is rendered to a PNG of `size` pixels on the way out — see the module
/// docs for why the vector must not reach the webview. A host with no SVG
/// pixbuf loader keeps the old answer rather than losing the icon to it.
fn read_icon_file(path: &Path, size: u32) -> Option<FileIcon> {
    let mime = mime_for_extension(path.extension()?.to_str()?)?;
    if mime == "image/svg+xml"
        && let Some(bytes) = rasterize_svg(path, size)
    {
        return Some(FileIcon {
            mime: "image/png",
            bytes,
        });
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(FileIcon { mime, bytes })
}

/// The vector at `size` pixels, as PNG bytes.
///
/// `from_file_at_size` fits the viewBox into the box preserving aspect ratio,
/// which for the square artwork every theme ships is exactly `size`×`size`.
/// Runs on the render pool, so it has to need no display and no main thread —
/// gdk-pixbuf's loaders need neither, which is the same reason the resolver
/// here reads theme files instead of driving `GtkIconTheme`.
#[cfg(target_os = "linux")]
fn rasterize_svg(path: &Path, size: u32) -> Option<Vec<u8>> {
    use gtk::gdk_pixbuf::Pixbuf;

    let edge = i32::try_from(size).ok()?;
    let pixbuf = Pixbuf::from_file_at_size(path, edge, edge).ok()?;
    pixbuf.save_to_bufferv("png", &[]).ok()
}

/// The Windows host this module is test-compiled on has no gdk-pixbuf behind
/// it, and a theme lookup there is a miss anyway.
#[cfg(not(target_os = "linux"))]
fn rasterize_svg(_path: &Path, _size: u32) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A theme index of the shape real themes have: `[Icon Theme]` carrying the
    /// inventory, one section per directory carrying its context.
    const INDEX: &str = "\
[Icon Theme]
Name=Test
Comment=A test theme
Directories=scalable/mimetypes,48x48/places,16x16/actions,16x16/apps
Inherits=hicolor

[scalable/mimetypes]
Context=MimeTypes
Type=Scalable

[48x48/places]
Context=Places
Size=48
MinSize=32
MaxSize=128
Type=Fixed

[16x16/actions]
Context=Actions
Size=16
Type=Fixed

[16x16/apps]
Context=Applications
Size=16
Type=Fixed
";

    #[test]
    fn reads_the_inventory_and_the_inheritance_from_the_index() {
        let ReadTheme { theme, inherits } = parse_theme("Test", INDEX);

        assert_eq!(theme.name, "Test");
        assert_eq!(inherits, vec!["hicolor".to_owned()]);
        assert_eq!(theme.directories.len(), 4);
    }

    /// `index.theme` keys are case-insensitive, which is the one thing a
    /// generic INI or TOML reader gets wrong here — the contexts above are
    /// spelled `MimeTypes`, not `mimetypes`.
    #[test]
    fn normalizes_the_context_case_so_lookups_match() {
        let ReadTheme { theme, .. } = parse_theme("Test", INDEX);
        let contexts: Vec<&str> = theme.directories.iter().map(|d| &*d.context).collect();

        // Lowercased on the way in, because every lookup name is lowercase and
        // a `Places` that must be compared case-insensitively is a footgun
        // waiting for the theme that spells it differently.
        assert_eq!(contexts, ["mimetypes", "places", "actions", "applications"]);
    }

    #[test]
    fn scalable_is_the_size_of_every_size() {
        let scalable = Directory {
            relative: "scalable/mimetypes".into(),
            context: "mimetypes".into(),
            size: None,
        };
        let small = Directory {
            relative: "16x16/mimetypes".into(),
            context: "mimetypes".into(),
            size: Some(16),
        };
        let large = Directory {
            relative: "256x256/mimetypes".into(),
            context: "mimetypes".into(),
            size: Some(256),
        };

        // The trap this ranking exists to avoid: at a 22px request, 16px is
        // *nearer* 22 than 256 is, and sorting by distance alone would pick the
        // icon that has to be blown up.
        assert!(small.rank_for(22) > large.rank_for(22));
        assert!(large.rank_for(22) > scalable.rank_for(22));
    }

    #[test]
    fn only_offers_directories_for_the_requested_context() {
        let ReadTheme { theme, .. } = parse_theme("Test", INDEX);

        assert_eq!(theme.directories_for(&["places"], 22), vec!["48x48/places"]);
        assert_eq!(
            theme.directories_for(&["mimetypes", "places"], 22),
            vec!["scalable/mimetypes", "48x48/places"]
        );
        assert!(theme.directories_for(&["filesystems"], 22).is_empty());
    }

    /// The context an application icon's directory declares is `Applications`,
    /// while the directory itself is named `apps` — the only context in the spec
    /// whose name and path segment differ, which is why asking for `apps` finds
    /// nothing in every theme shipped.
    #[test]
    fn an_application_icon_lives_under_the_applications_context() {
        let ReadTheme { theme, .. } = parse_theme("Test", INDEX);

        assert_eq!(
            theme.directories_for(&["applications"], 16),
            vec!["16x16/apps"]
        );
        assert!(theme.directories_for(&["apps"], 16).is_empty());
    }

    #[test]
    fn a_directory_without_a_size_is_scalable() {
        assert_eq!(parse_directory_size("48x48"), Some(48));
        assert_eq!(parse_directory_size("22x22"), Some(22));
        assert_eq!(parse_directory_size("scalable"), None);
        assert_eq!(parse_directory_size("literal"), None);
    }

    /// The name order is the policy, so it is what deserves a test. Getting
    /// this backwards draws every PDF with a generic sheet: `lookup` takes the
    /// first name that resolves anywhere.
    #[test]
    fn names_a_type_before_its_generic_and_before_unknown() {
        let names = icon_names_for(Path::new("/x/report.pdf"), false);

        assert_eq!(names[0], "application-pdf");
        assert_eq!(names.last().unwrap(), "unknown");
        assert!(
            names.iter().any(|name| name == "application-x-generic"),
            "the top-level generic has to be in the chain, at {names:?}"
        );
    }

    #[test]
    fn an_archive_is_named_after_the_concept_too() {
        let names = icon_names_for(Path::new("/x/backup.zip"), false);
        assert!(names.contains(&"package-x-generic".to_owned()));
        assert_eq!(names[0], "application-zip");
    }

    #[test]
    fn a_directory_leads_with_the_folder_icon() {
        let names = icon_names_for(Path::new("/home/user"), true);
        assert_eq!(names[0], "folder");
        assert_eq!(names.last().unwrap(), "unknown");
    }

    #[test]
    fn media_types_reach_their_own_generic() {
        let names = icon_names_for(Path::new("/x/song.mp3"), false);
        assert!(
            names.contains(&"audio-x-generic".to_owned()),
            "got {names:?}"
        );
    }

    /// The type route has to ask the same question the path route asks, or the
    /// same `.pdf` draws one icon in a list and another in a grid — and the type
    /// route is the one a whole column of rows shares an answer through, so a
    /// drift between them is a visible inconsistency rather than a rare one.
    #[test]
    fn a_type_resolves_to_the_names_the_same_path_would() {
        for extension in ["pdf", "zip", "mp3", "rs", "txt", "bin"] {
            let path = format!("/x/file.{extension}");
            assert_eq!(
                type_icon_names(extension, false),
                icon_names_for(Path::new(&path), false),
                "{extension} must be asked the same way either route"
            );
        }
    }

    #[test]
    fn a_type_with_no_mime_still_has_its_last_resort() {
        assert_eq!(
            type_icon_names("dae-no-such-extension-zzz", false),
            ["unknown"],
            "an unknown type is an unknown icon, not no icon"
        );
        // The executable bit is the one thing an extension cannot report, which is
        // why it is an argument here rather than something read off a path.
        assert!(type_icon_names("", true).contains(&"application-x-executable".to_owned()));
    }

    /// A type answer is not allowed to read the file it names — there is no file
    /// to read — so the two routes have to land on one bitmap.
    #[test]
    fn the_type_route_and_the_path_route_draw_the_same_icon() {
        let rendered = |icon: Option<FileIcon>| icon.map(|icon| (icon.mime, icon.bytes));

        assert_eq!(
            rendered(extract_type("pdf", 32)),
            rendered(extract("/x/report.pdf", 32, false)),
            "one type, one answer, whichever way it was asked"
        );
    }

    /// A name that cannot be on any machine, so this exercises the miss path
    /// rather than depending on the host's icon set.
    #[test]
    fn an_unresolvable_name_is_none_not_a_panic() {
        assert!(lookup(&["dae-no-such-icon-name-zzz"], &["apps"], 16).is_none());
        assert!(resolve_named_icon("", 16).is_none());
        assert!(resolve_named_icon("-", 16).is_none());
    }

    /// One field carrying the list glib reports for a launcher, split without
    /// losing the order — the search takes the first name that resolves, so a
    /// reordered list draws the app with its monochrome fallback.
    #[test]
    fn splits_the_icon_field_without_reshuffling_it() {
        assert_eq!(
            icon_candidates("dev.zed.Zed:zed:zed-symbolic"),
            ["dev.zed.Zed", "zed", "zed-symbolic"]
        );
        // Empty elements come from an `Icon=` that is a bare `;`, and `-` is the
        // entry saying it has no icon at all; neither is a name to look up.
        assert_eq!(icon_candidates("a::b"), ["a", "b"]);
        assert!(icon_candidates("-").is_empty());
        assert!(icon_candidates("").is_empty());
    }

    #[test]
    fn only_webview_renderables_are_served() {
        assert_eq!(mime_for_extension("svg"), Some("image/svg+xml"));
        assert_eq!(mime_for_extension("png"), Some("image/png"));
        // `xpm` is still shipped by some themes and no webview renders it.
        assert_eq!(mime_for_extension("xpm"), None);
        assert_eq!(mime_for_extension("jpg"), None);
    }

    #[test]
    fn an_unreadable_icon_file_is_a_miss() {
        assert!(read_icon_file(Path::new("/nonexistent/folder.svg"), 32).is_none());
        // A `.xpm` that exists is still not something to hand to a webview.
        assert!(read_icon_file(Path::new("/usr/share/pixmaps"), 32).is_none());
    }

    #[test]
    fn splits_index_lists_without_leaving_blanks() {
        assert_eq!(split_list("a, b ,,c"), vec!["a", "b", "c"]);
        assert!(split_list("").is_empty());
    }
}
