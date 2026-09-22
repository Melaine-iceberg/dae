//! macOS service declarations, read out of an application's `Info.plist` — the
//! parts that are pure data work, kept out of `macos.rs` so they compile and are
//! tested on any host.
//!
//! A service is declared under `NSServices`: an array of dictionaries, each
//! naming a menu title, the UTIs it accepts, and the selector that implements
//! it. That declaration is what Finder's own "Services" submenu is built from,
//! so reading it is what makes dae's section show the same commands.
//!
//! # Why the file is converted first
//!
//! `Info.plist` is usually a *binary* plist, and dae deliberately carries no
//! plist crate for it. Instead the macOS backend reads the file's bytes, looks
//! for the `NSServices` key — plain ASCII in both encodings — and only then asks
//! the system's own `/usr/bin/plutil` to render that one file as JSON. Almost
//! every application on a Mac has an `Info.plist` and almost none declare a
//! service, so that first check is what keeps a full scan to one directory read
//! per bundle instead of several hundred process spawns.
//!
//! JSON rather than XML because it is what `serde_json` — already a dependency
//! — parses, and because the declaration is nothing but dictionaries, arrays,
//! strings and booleans. The one cost is that `plutil` refuses to render a plist
//! containing a `<data>` or `<date>` as JSON, which drops that one bundle's
//! services rather than the whole scan; an `Info.plist` rarely carries either,
//! and a bundle whose manifest uses one is not something to guess about.
//!
//! # What else lives here
//!
//! Beyond the parser, the rest of the macOS backend that touches no framework: a
//! bundle's display name, the command-id encoding, and the rule for when a UTI is
//! safe to match on. Those decide what a row is called and which declaration it
//! runs, so they are the same kind of code that would otherwise ship a silent
//! mistake.

use serde_json::Value;
use std::path::Path;

/// One `NSServices` entry, reduced to what a context menu needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServiceDeclaration {
    /// Menu title, localized by [`parse_services`] from the declaration's own
    /// `NSMenuItem` dictionary.
    pub(super) title: String,
    /// The `NSMessage` selector, kept for diagnosis: it is what identifies the
    /// service within its bundle.
    pub(super) message: Option<String>,
    /// `NSSendFileTypes` — the UTIs the service accepts. Empty means the
    /// declaration is not file-based (a selection-based service, for instance),
    /// and it is dropped rather than offered against a file selection.
    pub(super) send_file_types: Vec<String>,
}

/// Reads the `NSServices` declarations out of a plist that `plutil` rendered as
/// JSON.
///
/// Returns an empty list for a plist without the key, which is the common case
/// and not an error. Input that cannot be parsed yields nothing rather than
/// failing: one damaged bundle must not hide every other application's services.
pub(super) fn parse_services(json: &str, language: Option<&str>) -> Vec<ServiceDeclaration> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(services) = root.get("NSServices").and_then(Value::as_array) else {
        return Vec::new();
    };

    services
        .iter()
        .filter_map(|service| declaration_of(service, language))
        .collect()
}

/// Reduces one declaration, dropping the ones this menu cannot honestly offer.
fn declaration_of(service: &Value, language: Option<&str>) -> Option<ServiceDeclaration> {
    // `NSRequiredContext` narrows a service to a context dae cannot provide —
    // most often `NSApplicationSpecific`, where a service only makes sense
    // inside the declaring app's own UI.
    if service.get("NSRequiredContext").is_some() {
        return None;
    }

    let title = menu_title(service.get("NSMenuItem")?, language)?;
    if title.trim().is_empty() {
        return None;
    }

    let send_file_types: Vec<String> = service
        .get("NSSendFileTypes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    // A service that accepts no file type is not a file command: Finder does not
    // offer it for a selection either, and neither does this.
    if send_file_types.is_empty() {
        return None;
    }

    Some(ServiceDeclaration {
        title,
        message: service
            .get("NSMessage")
            .and_then(Value::as_str)
            .map(str::to_string),
        send_file_types,
    })
}

/// Picks a service's menu title.
///
/// `NSMenuItem` is a dictionary keyed by language code — `default` holds the
/// untranslated name and any other key holds a localization. The plain-string
/// form is the older spelling and is still accepted.
fn menu_title(item: &Value, language: Option<&str>) -> Option<String> {
    if let Some(title) = item.as_str() {
        return Some(title.to_string());
    }

    // Exact language first, then its base, then `default` — the same preference
    // order a bundle's own localization lookup uses.
    if let Some(language) = language {
        if let Some(title) = item.get(language).and_then(Value::as_str) {
            return Some(title.to_string());
        }
        if let Some((base, _)) = language.split_once('_')
            && let Some(title) = item.get(base).and_then(Value::as_str)
        {
            return Some(title.to_string());
        }
    }

    item.get("default")
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Bundle identity and command ids
// ---------------------------------------------------------------------------

/// The UTI a directory is matched under. A folder has no filename extension, so
/// no UTI can be derived for it and this one is asserted instead.
pub(super) const FOLDER_TYPE: &str = "public.folder";

/// The UTI standing in for "a file whose type is not otherwise known" — a file
/// with no extension, or one whose extension LaunchServices does not recognize.
/// Narrower than `public.item` on purpose: claiming `public.item` for everything
/// would also claim directories, and a service that accepts files but not
/// folders must not be offered for a folder.
pub(super) const UNKNOWN_FILE_TYPE: &str = "public.data";

/// `dyn.` is the prefix LaunchServices gives a type it has no declaration for.
const DYNAMIC_PREFIX: &str = "dyn.";

/// Separates the declaring bundle from a declaration's index inside a command
/// id. Unit separator: a legal byte in a path but one no developer types, so an
/// id cannot be mis-split by a path that contains the separator.
const ID_SEPARATOR: char = '\u{1f}';

/// Whether a UTI identifier is one LaunchServices actually declared, as opposed
/// to a `dyn.` placeholder invented for an unknown extension.
///
/// Only a declared type can be matched on: a placeholder conforms to the
/// broadest types in the system and would make every service look applicable.
pub(super) fn is_dynamic(identifier: &str) -> bool {
    identifier.is_empty() || identifier.starts_with(DYNAMIC_PREFIX)
}

/// Whether `haystack` contains `needle`.
///
/// A plain byte search: the key it looks for is ASCII in both the binary and the
/// XML plist encodings, which is what lets the macOS scan skip a `plutil` call
/// for the vast majority of bundles that declare no services.
pub(super) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// The name shown in the menu for a bundle's commands.
///
/// Taken from the bundle's filename rather than `CFBundleName`: the two agree
/// for the overwhelming majority of applications, the filename is always there,
/// and the field is only ever read when one app contributes several commands.
pub(super) fn bundle_name(bundle: &Path) -> String {
    let name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Application");
    // `Visual Studio Code.app` → `Visual Studio Code`. Only a recognized bundle
    // suffix is stripped, so an ordinary folder keeps its dots.
    name.rsplit_once('.')
        .filter(|(_, extension)| matches!(*extension, "app" | "service" | "workflow"))
        .map_or(name, |(stem, _)| stem)
        .to_string()
}

/// Builds a command id from its bundle and the declaration's position in it.
///
/// Position rather than `NSMessage`, because one selector can legitimately back
/// several menu items and the index is what is unique within a bundle. The value
/// is only ever produced and consumed by this backend, and [`super::macos`]
/// re-resolves the whole declaration when one is invoked.
pub(super) fn id_of(bundle: &Path, index: usize) -> String {
    format!("{}{ID_SEPARATOR}{index}", bundle.display())
}

/// Splits a command id back into its bundle and declaration index.
///
/// Splits on the *last* separator, so a path containing one is still readable;
/// an index that is not a number is rejected rather than guessed at.
pub(super) fn split_id(id: &str) -> Option<(&str, usize)> {
    let (bundle, index) = id.rsplit_once(ID_SEPARATOR)?;
    Some((bundle, index.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_file_based_service() {
        let json = r#"{
            "CFBundleIdentifier": "com.example.app",
            "NSServices": [
                {
                    "NSMenuItem": { "default": "Open in Example" },
                    "NSMessage": "openInExample",
                    "NSSendFileTypes": ["public.item"]
                }
            ]
        }"#;

        assert_eq!(
            parse_services(json, None),
            vec![ServiceDeclaration {
                title: "Open in Example".into(),
                message: Some("openInExample".into()),
                send_file_types: vec!["public.item".into()],
            }]
        );
    }

    #[test]
    fn returns_nothing_for_a_plist_without_services() {
        // The overwhelmingly common case: an app that declares no service. It
        // must be silent, not an error.
        let json = r#"{"CFBundleName": "Something", "LSMinimumSystemVersion": "11.0"}"#;
        assert_eq!(parse_services(json, None), Vec::new());
    }

    #[test]
    fn drops_services_this_menu_cannot_offer() {
        let json = r#"{
            "NSServices": [
                {
                    "NSMenuItem": { "default": "Selection Only" },
                    "NSSendTypes": ["NSStringPboardType"]
                },
                {
                    "NSMenuItem": { "default": "App Specific" },
                    "NSRequiredContext": { "NSApplicationSpecific": true },
                    "NSSendFileTypes": ["public.item"]
                },
                {
                    "NSMenuItem": { "default": "Keep Me" },
                    "NSSendFileTypes": ["public.folder"]
                }
            ]
        }"#;

        // The first accepts no file type, the second needs its own app's
        // context; only the third belongs in a file menu.
        assert_eq!(
            parse_services(json, None)
                .into_iter()
                .map(|service| service.title)
                .collect::<Vec<_>>(),
            vec!["Keep Me"]
        );
    }

    #[test]
    fn prefers_a_localized_menu_title() {
        let json = r#"{
            "NSServices": [
                {
                    "NSMenuItem": {
                        "default": "Extract",
                        "zh": "解压",
                        "zh_CN": "解压到"
                    },
                    "NSSendFileTypes": ["public.archive"]
                }
            ]
        }"#;

        let title = |language: Option<&str>| {
            parse_services(json, language)
                .into_iter()
                .next()
                .map(|service| service.title)
        };

        assert_eq!(title(Some("zh_CN")).as_deref(), Some("解压到"));
        assert_eq!(title(Some("zh_TW")).as_deref(), Some("解压"));
        assert_eq!(title(Some("de")).as_deref(), Some("Extract"));
        assert_eq!(title(None).as_deref(), Some("Extract"));
    }

    #[test]
    fn accepts_the_plain_string_form_of_a_menu_title() {
        // The pre-localization spelling, still found in older bundles.
        let json = r#"{
            "NSServices": [
                {
                    "NSMenuItem": "Old Style",
                    "NSSendFileTypes": ["public.item"]
                }
            ]
        }"#;

        assert_eq!(
            parse_services(json, None).first().map(|s| s.title.as_str()),
            Some("Old Style")
        );
    }

    #[test]
    fn reads_several_declarations_and_their_type_lists() {
        let json = r#"{
            "NSServices": [
                {
                    "NSMenuItem": { "default": "One" },
                    "NSSendFileTypes": ["public.image", "public.folder"]
                },
                {
                    "NSMenuItem": { "default": "Two" },
                    "NSSendFileTypes": ["public.item"]
                }
            ]
        }"#;

        let services = parse_services(json, None);
        assert_eq!(services.len(), 2);
        assert_eq!(
            services[0].send_file_types,
            vec!["public.image", "public.folder"]
        );
        assert_eq!(services[1].title, "Two");
        // A declaration without `NSMessage` is still usable; the field is only
        // diagnosis.
        assert_eq!(services[1].message, None);
    }

    #[test]
    fn skips_a_declaration_with_no_usable_title() {
        let json = r#"{
            "NSServices": [
                { "NSSendFileTypes": ["public.item"] },
                { "NSMenuItem": { "default": "   " }, "NSSendFileTypes": ["public.item"] },
                { "NSMenuItem": { "default": "Fine" }, "NSSendFileTypes": ["public.item"] }
            ]
        }"#;

        assert_eq!(
            parse_services(json, None)
                .into_iter()
                .map(|service| service.title)
                .collect::<Vec<_>>(),
            vec!["Fine"]
        );
    }

    #[test]
    fn ignores_unrelated_keys_around_the_declaration() {
        let json = r#"{
            "CFBundleDevelopmentRegion": "en",
            "LSMinimumSystemVersion": "11.0",
            "NSSupportsAutomaticGraphicsSwitching": true,
            "BuildCount": 42,
            "NSAppTransportSecurity": { "NSAllowsArbitraryLoads": false },
            "NSServices": [
                {
                    "NSMenuItem": { "default": "After Scalars" },
                    "NSSendFileTypes": ["public.item"]
                }
            ]
        }"#;

        assert_eq!(
            parse_services(json, None).first().map(|s| s.title.as_str()),
            Some("After Scalars")
        );
    }

    #[test]
    fn returns_nothing_for_input_that_is_not_the_expected_shape() {
        assert_eq!(parse_services("", None), Vec::new());
        assert_eq!(parse_services("not json at all", None), Vec::new());
        // A plist whose `NSServices` is not an array: nothing to read, no panic.
        assert_eq!(
            parse_services(r#"{"NSServices": "oops"}"#, None),
            Vec::new()
        );
        // A declaration that is not an object.
        assert_eq!(
            parse_services(r#"{"NSServices": ["oops"]}"#, None),
            Vec::new()
        );
    }

    #[test]
    fn names_a_bundle_after_its_directory() {
        assert_eq!(
            bundle_name(Path::new("/Applications/Visual Studio Code.app")),
            "Visual Studio Code"
        );
        assert_eq!(
            bundle_name(Path::new("/System/Applications/Terminal.app")),
            "Terminal"
        );
        // A user's Automator action is a workflow bundle in the same locations.
        assert_eq!(
            bundle_name(Path::new("/Users/x/Library/Services/Unzip Files.workflow")),
            "Unzip Files"
        );
        // A legacy `.service` bundle.
        assert_eq!(
            bundle_name(Path::new("/System/Library/Services/Spotlight.service")),
            "Spotlight"
        );
        // Only a recognized bundle suffix is stripped: an ordinary folder keeps
        // its dots.
        assert_eq!(
            bundle_name(Path::new("/Applications/My.Cool.App")),
            "My.Cool.App"
        );
        assert_eq!(bundle_name(Path::new("/Applications/Plain")), "Plain");
    }

    #[test]
    fn round_trips_a_bundle_id() {
        let bundle = Path::new("/Applications/Visual Studio Code.app");

        assert_eq!(
            split_id(&id_of(bundle, 0)),
            Some(("/Applications/Visual Studio Code.app", 0))
        );
        assert_eq!(
            split_id(&id_of(bundle, 12)),
            Some(("/Applications/Visual Studio Code.app", 12))
        );
        // The last separator wins, so a path containing one is still readable.
        assert_eq!(
            split_id(&id_of(Path::new("/odd\u{1f}dir/X.app"), 3)),
            Some(("/odd\u{1f}dir/X.app", 3))
        );
        // An id that cannot be resolved is rejected rather than guessed at.
        assert_eq!(split_id("no-separator-here"), None);
        assert_eq!(split_id("/Applications/X.app\u{1f}not-a-number"), None);
    }

    #[test]
    fn finds_the_service_key_in_both_plist_encodings() {
        // The XML encoding spells the key in the clear.
        assert!(contains(
            br#"<plist version="1.0"><dict><key>NSServices</key></dict></plist>"#,
            b"NSServices"
        ));

        // A binary plist keeps its keys as plain ASCII too, which is what makes
        // the cheap pre-filter sound. This is the shape of a `bplist00` dict: an
        // ASCII string token, its length byte, then the key.
        let mut binary = b"bplist00".to_vec();
        binary.push(0x59); // a 9-character ASCII string token
        binary.extend_from_slice(b"NSServices");
        assert!(contains(&binary, b"NSServices"));

        // And an app that declares none is cheaply rejected.
        assert!(!contains(
            br#"<plist version="1.0"><dict><key>CFBundleName</key></dict></plist>"#,
            b"NSServices"
        ));
    }

    #[test]
    fn treats_the_launch_services_placeholders_as_unknown() {
        // A declared type is usable.
        assert!(!is_dynamic("public.png"));
        assert!(!is_dynamic("com.apple.rtfd"));
        // `dyn.` is LaunchServices saying it has no declaration, and an empty
        // identifier is the same thing said worse.
        assert!(is_dynamic("dyn.a4t8y2wwxk"));
        assert!(is_dynamic(""));
        // Only the prefix counts, not the substring.
        assert!(!is_dynamic("com.example.dyn.thing"));
    }
}
