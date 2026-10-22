//! XDG desktop-entry parsing shared by the Linux icon and thumbnail backends.
//!
//! Two readers need the same two pieces of the format: the icon-theme resolver
//! reads `index.theme` files, and `.thumbnailer` files are the same INI dialect
//! with an `Exec=` line. Keeping the parser and the `Exec` splitter here means
//! one implementation is kept correct rather than two, and it is pure string
//! work, so it compiles and is tested on every host.

use std::collections::HashMap;

/// Parses an INI-style desktop entry into `group name → key → value`.
///
/// `#` and a leading `;` start a comment. A `;` anywhere else is a list
/// separator inside a value, which is why the two are checked separately.
/// Values are otherwise kept verbatim, including the backslash escapes an
/// `Exec` line relies on.
pub fn parse_groups(text: &str) -> HashMap<String, HashMap<String, String>> {
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

/// Splits an `Exec` line on whitespace, honouring the spec's quoting and
/// escaping: `\` escapes the next character, and `"`/`'` quote a run.
///
/// A quoted empty argument is kept, because `Exec="" --flag` genuinely passes
/// an empty first argument. The line is never handed to a shell: a selected
/// filename containing `;` or `$(…)` stays the single argument its author
/// wrote.
pub fn split_exec(exec: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn reads_the_thumbnailer_group() {
        let groups = parse_groups(
            "[Thumbnailer Entry]\n\
             TryExec=ffmpegthumbnailer\n\
             Exec=ffmpegthumbnailer -i %i -o %o -s %s\n\
             MimeType=video/mp4;\n",
        );

        let entry = groups.get("Thumbnailer Entry").expect("the group");
        assert_eq!(
            entry.get("MimeType").map(String::as_str),
            Some("video/mp4;")
        );
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
}
