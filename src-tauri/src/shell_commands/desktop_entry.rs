//! XDG desktop-entry parsing for the Linux backend — the parts that are pure
//! string work, kept out of `linux.rs` so they compile and are tested on any
//! host.
//!
//! This split is not cosmetic. The machine this module was written on cannot
//! build the Linux backend at all (its dependencies need a Linux sysroot), so
//! anything left inside `linux.rs` is code nobody ran. Parsing a `.desktop`
//! file, matching a MIME type and expanding an `Exec` line are exactly the parts
//! that can be silently wrong — a mis-split argument list still spawns *a*
//! process — so they live here, where `cargo test` reaches them.
//!
//! The format is the freedesktop Desktop Entry Specification, plus KDE's
//! service-menu conventions layered on top (`Type=Service` with `Actions=`, or
//! the pre-5.85 `X-KDE-ServiceTypes=KonqPopupMenu/Plugin`).

use super::SelectionKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Separates the declaring file from the action id inside a command id. Unit
/// separator: a legal byte in a filename but one no packager types, so the two
/// halves cannot be confused by a path that itself contains the separator.
const ID_SEPARATOR: char = '\u{1f}';

/// KDE's convention for "any file". There is no MIME type that means this, so
/// it is special-cased rather than globbed — and it must not match a directory.
pub(super) const ANY_FILE: &str = "application/octet-stream";

/// A `.desktop` service menu, reduced to what a context menu needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServiceMenu {
    /// The declaring file, absolute. Half of every id this menu produces.
    pub(super) path: PathBuf,
    /// The app's own name for itself, used as the flyout label when it
    /// contributes more than one action.
    pub(super) name: String,
    /// MIME types the menu applies to, lowercased.
    pub(super) mime_types: Vec<String>,
    /// One entry per `[Desktop Action …]`, or a single synthetic entry for a
    /// file that declares no `Actions` and is therefore one command itself.
    pub(super) actions: Vec<MenuAction>,
}

/// One runnable row of a service menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuAction {
    /// The `Actions` id, or empty for a menu that is a single command.
    pub(super) id: String,
    pub(super) name: String,
    /// The raw `Exec` line, with its field codes still in place.
    pub(super) exec: String,
    pub(super) icon: Option<String>,
}

/// Parses one `.desktop` file into a service menu, or `None` when it is not one.
///
/// `language` is the user's locale in the desktop-entry sense (`zh_CN`, `de`),
/// used to prefer a localized `Name[...]` over the bare one — which is what the
/// file manager reading these would do.
pub(super) fn parse_service_menu(
    path: &Path,
    text: &str,
    language: Option<&str>,
) -> Option<ServiceMenu> {
    let groups = parse_groups(text);

    let entry = groups.get("Desktop Entry")?;
    if !is_service_menu(entry) {
        return None;
    }
    // A menu the packager disabled outright. `NoDisplay` is deliberately *not*
    // treated the same way: it is about launchers, several KDE packages set it
    // on a service menu by accident, and a menu should appear here exactly when
    // KDE would show it.
    if entry.get("Hidden").map(String::as_str) == Some("true") {
        return None;
    }

    let name = localized(entry, "Name", language)?.to_string();

    let mime_types: Vec<String> = entry
        .get("MimeType")
        .map(|value| split_list(value).map(str::to_lowercase).collect())
        .unwrap_or_default();

    // `Actions=` names `[Desktop Action …]` groups. A file without it is a
    // single command described by `[Desktop Entry]` itself — the older style,
    // still shipped by some packages.
    let action_ids: Vec<&str> = entry
        .get("Actions")
        .map(|value| split_list(value).collect())
        .unwrap_or_default();

    let mut actions = Vec::new();
    if action_ids.is_empty() {
        if let Some(exec) = entry.get("Exec").filter(|value| !value.trim().is_empty()) {
            actions.push(MenuAction {
                id: String::new(),
                name: name.clone(),
                exec: exec.clone(),
                icon: entry.get("Icon").cloned(),
            });
        }
    } else {
        for id in action_ids {
            // The group name is matched verbatim, as KDE does. A missing or
            // broken group drops that one action rather than the whole menu.
            let Some(group) = groups.get(&format!("Desktop Action {id}")) else {
                continue;
            };
            let Some(exec) = group.get("Exec").filter(|value| !value.trim().is_empty()) else {
                continue;
            };
            let Some(label) = localized(group, "Name", language) else {
                continue;
            };
            actions.push(MenuAction {
                id: id.to_string(),
                name: label.to_string(),
                exec: exec.clone(),
                icon: group.get("Icon").cloned(),
            });
        }
    }

    if actions.is_empty() {
        return None;
    }

    Some(ServiceMenu {
        path: path.to_path_buf(),
        name,
        mime_types,
        actions,
    })
}

/// The `Type=` values a service menu can legitimately carry. `Service` is the
/// current spelling; `KonqPopupMenu/Plugin` belongs to the pre-5.85
/// `kservices5` layout and is still what older packages ship.
fn is_service_menu(entry: &HashMap<String, String>) -> bool {
    entry.get("Type").map(String::as_str) == Some("Service")
        || entry
            .get("X-KDE-ServiceTypes")
            .is_some_and(|value| split_list(value).any(|item| item == "KonqPopupMenu/Plugin"))
}

/// Splits a desktop entry's `;`-separated list, dropping the empty trailing
/// element the format requires and any padding a packager added.
fn split_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

/// Picks the best available localization of `key`: the exact language, then the
/// language without its territory, then the unlocalized value.
fn localized<'a>(
    group: &'a HashMap<String, String>,
    key: &str,
    language: Option<&str>,
) -> Option<&'a str> {
    if let Some(language) = language {
        if let Some(value) = group.get(&format!("{key}[{language}]")) {
            return Some(value);
        }
        if let Some((base, _)) = language.split_once('_')
            && let Some(value) = group.get(&format!("{key}[{base}]"))
        {
            return Some(value);
        }
    }
    group.get(key).map(String::as_str)
}

/// Parses an INI-style desktop entry into `group name → key → value`.
///
/// `#` and a leading `;` start a comment. A `;` anywhere else is a list
/// separator inside a value, which is why the two are checked separately.
/// Values are otherwise kept verbatim, including the backslash escapes an
/// `Exec` line relies on — expansion happens later, in [`expand_exec`].
fn parse_groups(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut groups: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            current = Some(name.to_string());
            groups.entry(name.to_string()).or_default();
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(group) = current.as_ref() else {
            continue;
        };
        groups
            .entry(group.clone())
            .or_default()
            .insert(key.trim().to_string(), value.trim().to_string());
    }

    groups
}

/// The user's language for desktop-entry localization, in the `ll_CC` form the
/// format's `Name[ll_CC]` keys use.
pub(super) fn user_language() -> Option<String> {
    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(variable)
            && !value.is_empty()
            && value != "C"
            && value != "POSIX"
        {
            // `zh_CN.UTF-8@variant` → `zh_CN`
            let value = value.split('.').next().unwrap_or(&value);
            let value = value.split('@').next().unwrap_or(value);
            return Some(value.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// MIME matching
// ---------------------------------------------------------------------------

/// Whether a service menu's `MimeType` entry covers the selection.
///
/// Handles the three forms that appear in practice, and — like the Windows
/// module's `item_type_matches` — answers `false` for anything else rather than
/// guessing. A command offered in the wrong context is worse than one that does
/// not appear.
///
/// The known gap is inheritance: a menu declaring `text/plain` does not match a
/// `.py` file, though the shared-mime-info database says `text/x-python`
/// inherits from it. Reading that database is a much larger job than this
/// section is worth today.
pub(super) fn mime_matches(declared: &str, mime: &str, kind: SelectionKind) -> bool {
    match declared {
        // KDE's spelling of "any file". A directory is not a file.
        ANY_FILE => kind == SelectionKind::File,
        "inode/directory" => kind == SelectionKind::Directory,
        _ => match declared.split_once('/') {
            // The category glob, `image/*` and `text/*`.
            Some((category, "*")) => mime.split_once('/').is_some_and(|(got, _)| got == category),
            _ => declared == mime,
        },
    }
}

// ---------------------------------------------------------------------------
// Exec field codes
// ---------------------------------------------------------------------------

/// Expands a desktop entry's `Exec` line into an argument vector.
///
/// The grammar is the desktop-entry spec's, not a shell's: the line is split on
/// whitespace with quoting, then the `%` field codes are substituted. Nothing is
/// evaluated by a shell, and that is the point — an `Exec` from a `.desktop`
/// file is data, and handing it to `/bin/sh` would give a selected filename
/// containing `;` or `$(…)` a meaning its author never wrote.
///
/// Implemented: `%f`/`%F` (files), `%u`/`%U` (URLs), `%c` (the action's name),
/// `%k` (the declaring file), `%%` (a literal percent). `%i` expands to nothing:
/// it asks for an icon argument that no file manager passes on a command line.
/// Unknown codes drop out, as the spec requires. When the line carries no file
/// or URL code at all, the selection is appended — the same fallback KDE
/// applies, and the reason a menu written as `Exec=some-tool --flag` still
/// works on a selection.
pub(super) fn expand_exec(
    exec: &str,
    paths: &[String],
    name: &str,
    desktop_file: &str,
) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut consumed_paths = false;

    for word in split_exec(exec) {
        expand_word(
            &word,
            paths,
            name,
            desktop_file,
            &mut consumed_paths,
            &mut arguments,
        );
    }

    if !consumed_paths {
        arguments.extend(paths.iter().cloned());
    }
    arguments
}

/// `%X` and nothing else — the form a path code must be in to contribute more
/// than one argument.
fn whole_word_field_code(word: &str) -> Option<char> {
    let mut characters = word.chars();
    match (characters.next(), characters.next(), characters.next()) {
        (Some('%'), Some(code), None) => Some(code),
        _ => None,
    }
}

/// Substitutes the field codes inside one argument.
///
/// Standing alone, a selection code (`%F`/`%U`) contributes *several*
/// arguments. Written into the middle of an argument, the path codes fall back
/// to the first path: joining a list into one argument and splitting it again
/// would break a path that contains a space, so there is no correct expansion
/// there, and the mid-argument form is not one any packager writes
/// (`--dir=%f` takes a single path by construction).
fn expand_word(
    word: &str,
    paths: &[String],
    name: &str,
    desktop_file: &str,
    consumed_paths: &mut bool,
    arguments: &mut Vec<String>,
) {
    if let Some(code) = whole_word_field_code(word) {
        match code {
            'f' | 'u' => {
                *consumed_paths = true;
                if let Some(path) = paths.first() {
                    arguments.push(path.clone());
                }
                return;
            }
            'F' | 'U' => {
                *consumed_paths = true;
                arguments.extend(paths.iter().cloned());
                return;
            }
            _ => {}
        }
    }

    let mut result = String::with_capacity(word.len());
    let mut characters = word.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('%') => result.push('%'),
            Some('f') | Some('u') | Some('F') | Some('U') => {
                *consumed_paths = true;
                if let Some(path) = paths.first() {
                    result.push_str(path);
                }
            }
            Some('c') => result.push_str(name),
            Some('k') => result.push_str(desktop_file),
            // `%i` and any unknown code expand to nothing.
            Some(_) | None => {}
        }
    }

    // An argument that was nothing but codes can collapse to nothing, and an
    // empty argv entry is not the same as a missing one.
    if !result.is_empty() {
        arguments.push(result);
    }
}

/// Splits an `Exec` line on whitespace, honouring the spec's quoting and
/// escaping: `\` escapes the next character, and `"`/`'` quote a run.
///
/// A quoted empty argument is kept, because `Exec="" --flag` genuinely passes
/// an empty first argument.
fn split_exec(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut started = false;

    for character in exec.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            started = true;
            continue;
        }
        match character {
            '\\' => escaped = true,
            _ if quote == Some(character) => quote = None,
            '"' | '\'' if quote.is_none() => {
                quote = Some(character);
                started = true;
            }
            _ if character.is_whitespace() && quote.is_none() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(character);
                started = true;
            }
        }
    }

    if started {
        words.push(current);
    }
    words
}

// ---------------------------------------------------------------------------
// Command ids
// ---------------------------------------------------------------------------

/// Builds a command id from its declaring file and action.
pub(super) fn id_of(path: &Path, action: &str) -> String {
    format!("{}{ID_SEPARATOR}{action}", path.display())
}

/// Splits a command id back into its declaring file and action.
///
/// Splits on the *last* separator: an action id cannot contain one, so anything
/// before it is the path, even in the pathological case of a path that does.
pub(super) fn split_id(id: &str) -> Option<(&str, &str)> {
    id.rsplit_once(ID_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(text: &str) -> Option<ServiceMenu> {
        parse_service_menu(
            Path::new("/usr/share/kio/servicemenus/test.desktop"),
            text,
            None,
        )
    }

    #[test]
    fn reads_a_service_menu_with_multiple_actions() {
        let parsed = menu(
            "[Desktop Entry]\n\
             Type=Service\n\
             Name=Ark\n\
             MimeType=application/zip;application/x-tar;\n\
             Actions=extractHere;extractTo;\n\
             \n\
             [Desktop Action extractHere]\n\
             Name=Extract Here\n\
             Icon=archive-extract\n\
             Exec=ark --batch --autodestination %F\n\
             \n\
             [Desktop Action extractTo]\n\
             Name=Extract To...\n\
             Exec=ark --batch --destination %d %F\n",
        )
        .expect("a service menu");

        assert_eq!(parsed.name, "Ark");
        assert_eq!(
            parsed.mime_types,
            vec!["application/zip", "application/x-tar"]
        );
        assert_eq!(parsed.actions.len(), 2);
        assert_eq!(parsed.actions[0].id, "extractHere");
        assert_eq!(parsed.actions[0].name, "Extract Here");
        assert_eq!(parsed.actions[0].icon.as_deref(), Some("archive-extract"));
        assert_eq!(parsed.actions[1].id, "extractTo");
    }

    #[test]
    fn treats_a_menu_without_actions_as_a_single_command() {
        // The pre-`Actions=` style: the desktop entry *is* the command.
        let parsed = menu(
            "[Desktop Entry]\n\
             Type=Service\n\
             Name=Open in Kate\n\
             MimeType=text/plain;\n\
             Exec=kate %U\n\
             Icon=kate\n",
        )
        .expect("a service menu");

        assert_eq!(parsed.actions.len(), 1);
        assert_eq!(parsed.actions[0].id, "");
        assert_eq!(parsed.actions[0].name, "Open in Kate");
        assert_eq!(parsed.actions[0].icon.as_deref(), Some("kate"));
    }

    #[test]
    fn accepts_the_deprecated_plugin_service_type() {
        // `kservices5`-era files declare no `Type=Service` at all.
        let parsed = menu(
            "[Desktop Entry]\n\
             X-KDE-ServiceTypes=KonqPopupMenu/Plugin\n\
             Name=Old\n\
             MimeType=image/*;\n\
             Actions=go;\n\
             [Desktop Action go]\n\
             Name=Go\n\
             Exec=go %f\n",
        )
        .expect("a service menu");

        assert_eq!(parsed.actions.len(), 1);
    }

    #[test]
    fn ignores_files_that_are_not_service_menus() {
        // An ordinary application entry.
        assert_eq!(
            menu(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Firefox\n\
                 Exec=firefox %u\n"
            ),
            None
        );
        // A service menu the packager disabled.
        assert_eq!(
            menu(
                "[Desktop Entry]\n\
                 Type=Service\n\
                 Name=Gone\n\
                 Hidden=true\n\
                 MimeType=image/png;\n\
                 Exec=gone %f\n"
            ),
            None
        );
    }

    #[test]
    fn skips_an_action_group_that_is_missing_its_exec() {
        let parsed = menu(
            "[Desktop Entry]\n\
             Type=Service\n\
             Name=Half\n\
             MimeType=image/png;\n\
             Actions=good;bad;\n\
             [Desktop Action good]\n\
             Name=Good\n\
             Exec=true %f\n\
             [Desktop Action bad]\n\
             Name=Bad\n",
        )
        .expect("a service menu");

        assert_eq!(parsed.actions.len(), 1);
        assert_eq!(parsed.actions[0].id, "good");
    }

    #[test]
    fn returns_nothing_when_every_action_is_unusable() {
        assert_eq!(
            menu(
                "[Desktop Entry]\n\
                 Type=Service\n\
                 Name=Empty\n\
                 MimeType=image/png;\n\
                 Actions=one;\n\
                 [Desktop Action one]\n\
                 Name=One\n"
            ),
            None
        );
        // A menu with a MIME filter but no command at all.
        assert_eq!(
            menu(
                "[Desktop Entry]\n\
                 Type=Service\n\
                 Name=Nothing\n\
                 MimeType=image/png;\n"
            ),
            None
        );
    }

    #[test]
    fn prefers_the_users_language_then_its_base_language() {
        let groups = parse_groups(
            "[Desktop Entry]\n\
             Name=Extract\n\
             Name[zh]=解压\n\
             Name[zh_CN]=解压到\n",
        );

        let entry = groups.get("Desktop Entry").expect("the group");
        assert_eq!(localized(entry, "Name", Some("zh_CN")), Some("解压到"));
        assert_eq!(localized(entry, "Name", Some("zh_TW")), Some("解压"));
        assert_eq!(localized(entry, "Name", Some("de")), Some("Extract"));
        assert_eq!(localized(entry, "Name", None), Some("Extract"));
    }

    #[test]
    fn keeps_comments_and_list_semicolons_apart() {
        let groups = parse_groups(
            "[Desktop Entry]\n\
             Exec=ark --batch %F\n\
             MimeType=a;b;\n\
             # a comment\n\
             ; another comment\n\
             Name=Ark\n",
        );

        let entry = groups.get("Desktop Entry").expect("the group");
        assert_eq!(
            entry.get("Exec").map(String::as_str),
            Some("ark --batch %F")
        );
        assert_eq!(entry.get("Name").map(String::as_str), Some("Ark"));
        assert!(!entry.contains_key("# a comment"));
        assert!(!entry.contains_key("; another comment"));
        assert_eq!(entry.get("MimeType").map(String::as_str), Some("a;b;"));
    }

    #[test]
    fn matches_the_three_forms_a_mime_type_entry_takes() {
        use SelectionKind::{Directory, File};

        assert!(mime_matches(ANY_FILE, "image/png", File));
        assert!(mime_matches(ANY_FILE, ANY_FILE, File));
        assert!(!mime_matches(ANY_FILE, "inode/directory", Directory));

        assert!(mime_matches(
            "inode/directory",
            "inode/directory",
            Directory
        ));
        assert!(!mime_matches("inode/directory", "image/png", File));

        assert!(mime_matches("image/*", "image/png", File));
        assert!(mime_matches("image/*", "image/svg+xml", File));
        assert!(!mime_matches("image/*", "text/plain", File));

        assert!(mime_matches("application/zip", "application/zip", File));
        assert!(!mime_matches("application/zip", "application/x-tar", File));
    }

    #[test]
    fn splits_exec_on_whitespace_and_quotes() {
        assert_eq!(split_exec("ark --batch %F"), vec!["ark", "--batch", "%F"]);
        assert_eq!(
            split_exec(r#"/usr/bin/tool --label "two words" %f"#),
            vec!["/usr/bin/tool", "--label", "two words", "%f"]
        );
        assert_eq!(
            split_exec(r#"tool --path '/a dir/x' %f"#),
            vec!["tool", "--path", "/a dir/x", "%f"]
        );
        // The spec's escaping: a backslash makes the next character literal.
        assert_eq!(split_exec(r"tool a\ b %f"), vec!["tool", "a b", "%f"]);
        // Repeated whitespace is not significant.
        assert_eq!(split_exec("  tool   %f  "), vec!["tool", "%f"]);
        // A quoted empty argument is still an argument.
        assert_eq!(split_exec(r#"tool "" %f"#), vec!["tool", "", "%f"]);
    }

    #[test]
    fn expands_the_field_codes_the_spec_defines() {
        let paths = vec!["/tmp/a.txt".to_string(), "/tmp/c.txt".to_string()];

        assert_eq!(
            expand_exec("tool %f", &paths, "Label", "/d/x.desktop"),
            vec!["tool", "/tmp/a.txt"]
        );
        // `%c` and `%k` carry metadata rather than files, so they substitute
        // without marking the paths consumed — and the selection is therefore
        // still appended, because the line names no file code.
        assert_eq!(
            expand_exec("tool --name %c --file %k", &paths, "Label", "/d/x.desktop"),
            vec![
                "tool",
                "--name",
                "Label",
                "--file",
                "/d/x.desktop",
                "/tmp/a.txt",
                "/tmp/c.txt"
            ]
        );
        // `%%` is a literal percent, not a code.
        assert_eq!(
            expand_exec("tool 100%% %f", &paths, "L", "/d/x.desktop"),
            vec!["tool", "100%", "/tmp/a.txt"]
        );
        // An unknown code drops out without taking its argument with it.
        assert_eq!(
            expand_exec("tool %z %f", &paths, "L", "/d/x.desktop"),
            vec!["tool", "/tmp/a.txt"]
        );
        // `%i` asks for an icon argument no file manager passes, so it vanishes.
        assert_eq!(
            expand_exec("tool %i %f", &paths, "L", "/d/x.desktop"),
            vec!["tool", "/tmp/a.txt"]
        );
    }

    /// The load-bearing one. A selection code standing alone contributes one
    /// argument *per file*, and a path with a space in it survives intact —
    /// which a join-then-split implementation would silently corrupt.
    #[test]
    fn a_whole_word_selection_code_becomes_one_argument_per_file() {
        let paths = vec!["/tmp/a b.txt".to_string(), "/tmp/c d.txt".to_string()];

        assert_eq!(
            expand_exec("tool %F", &paths, "L", "/d/x.desktop"),
            vec!["tool", "/tmp/a b.txt", "/tmp/c d.txt"]
        );
        assert_eq!(
            expand_exec("tool %U --flag", &paths, "L", "/d/x.desktop"),
            vec!["tool", "/tmp/a b.txt", "/tmp/c d.txt", "--flag"]
        );
    }

    #[test]
    fn a_mid_argument_path_code_falls_back_to_the_first_file() {
        let paths = vec!["/tmp/a.txt".to_string(), "/tmp/b.txt".to_string()];

        assert_eq!(
            expand_exec("tool --file=%f", &paths, "L", "/d/x.desktop"),
            vec!["tool", "--file=/tmp/a.txt"]
        );
    }

    #[test]
    fn appends_the_selection_when_the_exec_names_no_file_code() {
        // The KDE fallback: a menu written as a bare command still receives the
        // selection.
        let paths = vec!["/tmp/a.txt".to_string()];

        assert_eq!(
            expand_exec("tool --flag", &paths, "L", "/d/x.desktop"),
            vec!["tool", "--flag", "/tmp/a.txt"]
        );
    }

    #[test]
    fn round_trips_an_id_through_its_separator() {
        let path = Path::new("/usr/share/kio/servicemenus/ark.desktop");
        let id = id_of(path, "extractHere");

        assert_eq!(
            split_id(&id),
            Some(("/usr/share/kio/servicemenus/ark.desktop", "extractHere"))
        );
        // The single-command form has an empty action, and still splits.
        assert_eq!(
            split_id(&id_of(path, "")),
            Some(("/usr/share/kio/servicemenus/ark.desktop", ""))
        );
        // The last separator wins, so a path containing one is still readable.
        assert_eq!(
            split_id(&id_of(Path::new("/odd\u{1f}dir/x.desktop"), "go")),
            Some(("/odd\u{1f}dir/x.desktop", "go"))
        );
        // An id without one cannot be resolved, and is rejected rather than
        // guessed at.
        assert_eq!(split_id("no-separator-here"), None);
    }

    #[test]
    fn reads_the_language_out_of_the_locale_environment() {
        // Not asserted against a value, because it reads the ambient
        // environment: it asserts only that whatever comes back is in the
        // `ll` or `ll_CC` shape the `Name[...]` keys use.
        if let Some(language) = user_language() {
            assert!(
                !language.contains('.'),
                "locale kept its codeset: {language}"
            );
            assert!(
                !language.contains('@'),
                "locale kept its variant: {language}"
            );
            assert!(!language.is_empty());
        }
    }
}
