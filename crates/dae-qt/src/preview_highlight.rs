//! Syntax highlighting for the code preview, powered by tree-sitter.
//!
//! Each language is parsed by its own tree-sitter grammar and coloured through
//! the grammar's `highlights.scm` query, so identifiers, types, properties and
//! punctuation are distinguished the way an editor distinguishes them — not
//! the handful of broad scopes a tmTheme gives. The result is emitted as the
//! same coloured-`<span>` HTML the pane's `QTextDocument` already renders, on
//! the same 12 px / 18 px grid, so nothing in QML changes.
//!
//! Most grammars arrive as crates built on `tree-sitter-language` (they share
//! the one runtime this crate links). Eight — Dockerfile, Gitignore, Groovy,
//! JSON5, Kotlin, LaTeX, Less and SCSS — are vendored as C under
//! `assets/grammars`: their published bindings either pull a second, older
//! runtime, omit the external scanner the parser needs (LaTeX), or are absent
//! (Gitignore) or node-incompatible with the upstream query (Groovy);
//! `build.rs` compiles them against this one. A few more crates ship no usable
//! highlight query, so just their `highlights.scm` is vendored alongside
//! (GraphQL, Proto).

use std::collections::HashMap;
use std::sync::LazyLock;

use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};
use tree_sitter_language::LanguageFn;

use crate::preview_markup::{expand_tabs, push_escaped};

// The shell's canvas and ink, kept identical to the pane's surrounding card.
const DARK_BG: &str = "#101216";
const DARK_FG: &str = "#f7f7f9";
const LIGHT_BG: &str = "#fdfdfd";
const LIGHT_FG: &str = "#1c1e23";

/// The capture names tree-sitter queries use, each with a dark and a light
/// colour. Hues are spread across the wheel (red keywords, purple functions,
/// green types, blue strings/numbers, orange constructors) so adjacent tokens
/// read apart; every colour clears 4.5:1 on its canvas, enforced by a test.
/// Order matters: `configure` is handed these names and a `Highlight(i)` then
/// indexes straight back into this table.
const PALETTE: &[(&str, &str, &str)] = &[
    ("keyword", "#ff7b72", "#cf222e"),
    ("function", "#d2a8ff", "#8250df"),
    ("function.builtin", "#d2a8ff", "#8250df"),
    ("type", "#7ee787", "#116329"),
    ("type.builtin", "#7ee787", "#116329"),
    ("constructor", "#ffa657", "#953800"),
    ("constructor.builtin", "#ffa657", "#953800"),
    ("string", "#a5d6ff", "#0a3069"),
    ("string.escape", "#ffa657", "#953800"),
    ("string.regexp", "#a5d6ff", "#0a3069"),
    ("string.special", "#a5d6ff", "#0a3069"),
    ("string.special.symbol", "#a5d6ff", "#0a3069"),
    ("number", "#79c0ff", "#0550ae"),
    ("boolean", "#79c0ff", "#0550ae"),
    ("constant", "#79c0ff", "#0550ae"),
    ("constant.builtin", "#79c0ff", "#0550ae"),
    ("comment", "#8b949e", "#656d76"),
    ("comment.documentation", "#8b949e", "#656d76"),
    ("variable", DARK_FG, LIGHT_FG),
    ("variable.builtin", "#ffa657", "#953800"),
    ("variable.parameter", DARK_FG, LIGHT_FG),
    ("variable.member", "#79c0ff", "#0550ae"),
    ("property", "#79c0ff", "#0550ae"),
    ("property.builtin", "#79c0ff", "#0550ae"),
    ("attribute", "#d2a8ff", "#8250df"),
    ("tag", "#7ee787", "#116329"),
    ("module", "#ffa657", "#953800"),
    ("label", "#79c0ff", "#0550ae"),
    ("escape", "#ffa657", "#953800"),
    ("operator", DARK_FG, LIGHT_FG),
    ("punctuation", "#8b949e", "#656d76"),
    ("punctuation.bracket", DARK_FG, LIGHT_FG),
    ("punctuation.delimiter", "#8b949e", "#656d76"),
    ("punctuation.special", "#ff7b72", "#cf222e"),
    ("embedded", DARK_FG, LIGHT_FG),
    // LaTeX (nvim-treesitter query) speaks in `markup.*` captures; the dot
    // prefix matching in `configure` folds `.heading.1`–`.6` and `.link.url`
    // onto these entries.
    ("markup.heading", "#ffa657", "#953800"),
    ("markup.link", "#a5d6ff", "#0a3069"),
    ("markup.math", "#7ee787", "#116329"),
    ("markup.strong", DARK_FG, LIGHT_FG),
    ("markup.italic", DARK_FG, LIGHT_FG),
    ("error", "#f85149", "#cf222e"),
];

// The vendored grammars expose only their C entry point; a Rust binding that
// would wrap them either lives against an older runtime or does not exist, so
// the symbol is declared here and handed to the current one. ABI 13–15, within
// the runtime's range.
unsafe extern "C" {
    fn tree_sitter_dockerfile() -> *const ();
    fn tree_sitter_gitignore() -> *const ();
    fn tree_sitter_groovy() -> *const ();
    fn tree_sitter_json5() -> *const ();
    fn tree_sitter_kotlin() -> *const ();
    fn tree_sitter_latex() -> *const ();
    fn tree_sitter_less() -> *const ();
    fn tree_sitter_scss() -> *const ();
}

/// Every canonical token the pane can highlight. `configuration` is built for
/// each; a token whose query fails to compile is simply absent and renders as
/// plain text, but a test insists none are missing.
const TOKENS: &[&str] = &[
    "rs", "js", "jsx", "ts", "tsx", "json", "json5", "css", "scss", "less", "html", "qml", "py",
    "rb", "go", "java", "scala", "c", "cpp", "m", "cs", "fs", "sh", "ps1", "sql",
    "yaml", "toml", "ini", "dockerfile", "lua", "php", "swift", "dart", "graphql", "proto", "tex",
    "nix", "make", "cmake", "groovy", "gitignore", "kotlin",
];

/// Folds a language token (the short form `classify` emits) or a Markdown fence
/// info string (`rust`, `c++`, `objective-c`, …) onto a canonical token.
fn canonical(input: &str) -> Option<&'static str> {
    let lower = input.trim().to_ascii_lowercase();
    Some(match lower.as_str() {
        "rs" | "rust" => "rs",
        "js" | "javascript" | "mjs" | "cjs" => "js",
        "jsx" => "jsx",
        "ts" | "typescript" | "mts" | "cts" => "ts",
        "tsx" => "tsx",
        "json" => "json",
        "jsonc" | "json5" => "json5",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "styl" | "stylus" => "css",
        "html" | "htm" | "xml" | "svg" | "xhtml" | "vue" | "svelte" | "astro" => "html",
        "qml" | "qmljs" => "qml",
        "py" | "python" | "pyi" => "py",
        "rb" | "ruby" | "gemfile" | "rakefile" | "procfile" => "rb",
        "go" | "golang" => "go",
        "java" => "java",
        "scala" => "scala",
        "c" | "h" => "c",
        "cpp" | "c++" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "m" | "mm" | "objc" | "objective-c" => "m",
        "cs" | "c#" | "csharp" => "cs",
        "fs" | "f#" | "fsharp" => "fs",
        "sh" | "bash" | "zsh" | "fish" | "shell" => "sh",
        "ps1" | "psm1" | "powershell" | "pwsh" => "ps1",
        "sql" => "sql",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "conf" | "cfg" | "properties" | "editorconfig" | "env" | "dotenv" => "ini",
        "dockerfile" => "dockerfile",
        "lua" => "lua",
        "php" => "php",
        "swift" => "swift",
        "dart" => "dart",
        "graphql" | "gql" => "graphql",
        "proto" | "protobuf" => "proto",
        "tex" | "latex" | "bib" | "sty" => "tex",
        "nix" => "nix",
        "make" | "makefile" | "mk" | "gnumakefile" => "make",
        "cmake" => "cmake",
        "groovy" | "gradle" | "gvy" | "gy" | "gsh" => "groovy",
        "gitignore" | "dockerignore" | "npmignore" => "gitignore",
        "kotlin" | "kt" | "kts" => "kotlin",
        _ => return None,
    })
}

/// The grammar and highlight query for a canonical token. Injections and
/// locals are left empty: the pane highlights a single language at a time, the
/// same scope the previous engine had.
fn language_and_query(token: &str) -> Option<(Language, &'static str)> {
    Some(match token {
        "rs" => (Language::new(tree_sitter_rust::LANGUAGE), tree_sitter_rust::HIGHLIGHTS_QUERY),
        "js" => (Language::new(tree_sitter_javascript::LANGUAGE), tree_sitter_javascript::HIGHLIGHT_QUERY),
        "jsx" => (Language::new(tree_sitter_javascript::LANGUAGE), tree_sitter_javascript::HIGHLIGHT_QUERY),
        "ts" => (Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT), tree_sitter_typescript::HIGHLIGHTS_QUERY),
        "tsx" => (Language::new(tree_sitter_typescript::LANGUAGE_TSX), tree_sitter_typescript::HIGHLIGHTS_QUERY),
        "json" => (Language::new(tree_sitter_json::LANGUAGE), tree_sitter_json::HIGHLIGHTS_QUERY),
        "json5" => (vendored(tree_sitter_json5), include_str!("../assets/grammars/json5/highlights.scm")),
        "css" => (Language::new(tree_sitter_css::LANGUAGE), tree_sitter_css::HIGHLIGHTS_QUERY),
        "scss" => (vendored(tree_sitter_scss), include_str!("../assets/grammars/scss/highlights.scm")),
        "less" => (vendored(tree_sitter_less), include_str!("../assets/grammars/less/highlights.scm")),
        "html" => (Language::new(tree_sitter_html::LANGUAGE), tree_sitter_html::HIGHLIGHTS_QUERY),
        "qml" => (Language::new(tree_sitter_qmljs::LANGUAGE), tree_sitter_qmljs::HIGHLIGHTS_QUERY),
        "py" => (Language::new(tree_sitter_python::LANGUAGE), tree_sitter_python::HIGHLIGHTS_QUERY),
        "rb" => (Language::new(tree_sitter_ruby::LANGUAGE), tree_sitter_ruby::HIGHLIGHTS_QUERY),
        "go" => (Language::new(tree_sitter_go::LANGUAGE), tree_sitter_go::HIGHLIGHTS_QUERY),
        "java" => (Language::new(tree_sitter_java::LANGUAGE), tree_sitter_java::HIGHLIGHTS_QUERY),
        "scala" => (Language::new(tree_sitter_scala::LANGUAGE), tree_sitter_scala::HIGHLIGHTS_QUERY),
        "c" => (Language::new(tree_sitter_c::LANGUAGE), tree_sitter_c::HIGHLIGHT_QUERY),
        "cpp" => (Language::new(tree_sitter_cpp::LANGUAGE), tree_sitter_cpp::HIGHLIGHT_QUERY),
        "m" => (Language::new(tree_sitter_objc::LANGUAGE), tree_sitter_objc::HIGHLIGHTS_QUERY),
        "cs" => (Language::new(tree_sitter_c_sharp::LANGUAGE), tree_sitter_c_sharp::HIGHLIGHTS_QUERY),
        "fs" => (Language::new(tree_sitter_fsharp::LANGUAGE_FSHARP), tree_sitter_fsharp::HIGHLIGHTS_QUERY),
        "sh" => (Language::new(tree_sitter_bash::LANGUAGE), tree_sitter_bash::HIGHLIGHT_QUERY),
        "ps1" => (Language::new(tree_sitter_powershell::LANGUAGE), tree_sitter_powershell::HIGHLIGHTS_QUERY),
        "sql" => (Language::new(tree_sitter_sequel::LANGUAGE), tree_sitter_sequel::HIGHLIGHTS_QUERY),
        "yaml" => (Language::new(tree_sitter_yaml::LANGUAGE), tree_sitter_yaml::HIGHLIGHTS_QUERY),
        "toml" => (Language::new(tree_sitter_toml_ng::LANGUAGE), tree_sitter_toml_ng::HIGHLIGHTS_QUERY),
        "ini" => (Language::new(tree_sitter_ini::LANGUAGE), tree_sitter_ini::HIGHLIGHTS_QUERY),
        "dockerfile" => (vendored(tree_sitter_dockerfile), include_str!("../assets/grammars/dockerfile/highlights.scm")),
        "lua" => (Language::new(tree_sitter_lua::LANGUAGE), tree_sitter_lua::HIGHLIGHTS_QUERY),
        "php" => (Language::new(tree_sitter_php::LANGUAGE_PHP), tree_sitter_php::HIGHLIGHTS_QUERY),
        "swift" => (Language::new(tree_sitter_swift::LANGUAGE), tree_sitter_swift::HIGHLIGHTS_QUERY),
        "dart" => (Language::new(tree_sitter_dart::LANGUAGE), tree_sitter_dart::HIGHLIGHTS_QUERY),
        "graphql" => (Language::new(tree_sitter_graphql::LANGUAGE), include_str!("../assets/grammars/graphql/highlights.scm")),
        "proto" => (Language::new(tree_sitter_proto::LANGUAGE), include_str!("../assets/grammars/proto/highlights.scm")),
        "tex" => (vendored(tree_sitter_latex), include_str!("../assets/grammars/latex/highlights.scm")),
        "nix" => (Language::new(tree_sitter_nix::LANGUAGE), tree_sitter_nix::HIGHLIGHTS_QUERY),
        "make" => (Language::new(tree_sitter_make::LANGUAGE), tree_sitter_make::HIGHLIGHTS_QUERY),
        "cmake" => (Language::new(tree_sitter_cmake::LANGUAGE), tree_sitter_cmake::HIGHLIGHTS_QUERY),
        "groovy" => (vendored(tree_sitter_groovy), include_str!("../assets/grammars/groovy/highlights.scm")),
        "gitignore" => (vendored(tree_sitter_gitignore), include_str!("../assets/grammars/gitignore/highlights.scm")),
        "kotlin" => (vendored(tree_sitter_kotlin), include_str!("../assets/grammars/kotlin/highlights.scm")),
        _ => return None,
    })
}

/// Wraps a vendored grammar's C entry point in the current runtime's `Language`.
///
/// # Safety
/// The caller passes a symbol generated by the tree-sitter CLI whose ABI the
/// linked runtime accepts (13–15); the eight vendored parsers are 13–15.
fn vendored(f: unsafe extern "C" fn() -> *const ()) -> Language {
    Language::new(unsafe { LanguageFn::from_raw(f) })
}

static CONFIGS: LazyLock<HashMap<&'static str, HighlightConfiguration>> = LazyLock::new(|| {
    let names: Vec<&str> = PALETTE.iter().map(|entry| entry.0).collect();
    let mut map = HashMap::new();
    for token in TOKENS {
        let Some((language, query)) = language_and_query(token) else {
            continue;
        };
        if let Ok(mut config) = HighlightConfiguration::new(language, *token, query, "", "") {
            config.configure(&names);
            map.insert(*token, config);
        }
    }
    map
});

/// Whether a language token or fence name has a working grammar.
pub fn supports(language: &str) -> bool {
    canonical(language).is_some_and(|token| CONFIGS.contains_key(token))
}

/// Renders `source` as a complete `<pre>` block: coloured spans when the
/// language has a grammar, escaped plain text otherwise. Tabs are expanded to
/// the pane's four-column grid first, so spans and glyphs share coordinates.
pub fn render_code(source: &str, language: &str, dark: bool) -> String {
    let expanded = expand_tabs(source);
    let Some(config) = canonical(language).and_then(|token| CONFIGS.get(token)) else {
        return plain_pre(&expanded, dark);
    };

    let mut highlighter = Highlighter::new();
    let events = match highlighter.highlight(config, expanded.as_bytes(), None, None, |_| None) {
        Ok(iter) => iter.collect::<Vec<_>>(),
        Err(_) => return plain_pre(&expanded, dark),
    };
    // A mid-stream error means the tail would be unstyled; fall back to plain
    // for the whole block rather than ship a half-coloured file.
    if events.iter().any(|event| event.is_err()) {
        return plain_pre(&expanded, dark);
    }

    let mut out = String::with_capacity(source.len() * 2);
    out.push_str(&pre_open(dark));
    let mut stack: Vec<usize> = Vec::new();
    for event in events {
        match event.expect("checked above") {
            HighlightEvent::Source { start, end } => {
                let text = &expanded[start..end];
                if text.is_empty() {
                    continue;
                }
                let (color, italic, bold) = match stack.last() {
                    Some(&index) => paint(PALETTE[index].0, dark),
                    None => (foreground(dark), false, false),
                };
                push_span(&mut out, color, italic, bold, text, dark);
            }
            HighlightEvent::HighlightStart(highlight) => stack.push(highlight.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
        }
    }
    out.push_str("</pre>");
    out
}

fn foreground(dark: bool) -> &'static str {
    if dark { DARK_FG } else { LIGHT_FG }
}

/// Colour plus the font styles the pane honours: comments and `markup.italic`
/// italic, keywords, headings and `markup.strong` semibold. Everything else is
/// carried by hue alone.
fn paint(name: &str, dark: bool) -> (&'static str, bool, bool) {
    let color = PALETTE
        .iter()
        .find(|entry| entry.0 == name)
        .map(|entry| if dark { entry.1 } else { entry.2 })
        .unwrap_or_else(|| foreground(dark));
    let italic = name.starts_with("comment") || name == "markup.italic";
    let bold = name == "keyword" || name == "markup.heading" || name == "markup.strong";
    (color, italic, bold)
}

fn push_span(out: &mut String, color: &str, italic: bool, bold: bool, text: &str, dark: bool) {
    if color == foreground(dark) && !italic && !bold {
        push_escaped(out, text);
        return;
    }
    out.push_str("<span style=\"color:");
    out.push_str(color);
    out.push(';');
    if bold {
        out.push_str("font-weight:600;");
    }
    if italic {
        out.push_str("font-style:italic;");
    }
    out.push_str("\">");
    push_escaped(out, text);
    out.push_str("</span>");
}

/// A 12 px face, an 18 px line and `white-space:pre`: the three values the
/// card's width and height are measured against. Padding is deliberately absent
/// — Qt's rich text engine only honours it on table cells; the card's margins
/// do the inset.
fn pre_open(dark: bool) -> String {
    let (background, foreground) = if dark {
        (DARK_BG, DARK_FG)
    } else {
        (LIGHT_BG, LIGHT_FG)
    };
    format!(
        "<pre style=\"margin:0; background-color:{background}; color:{foreground}; \
         font-family:'Mona Sans Mono VF','Mona Sans Mono','Cascadia Mono',Consolas,monospace; font-size:12px; line-height:18px; \
         white-space:pre;\">"
    )
}

fn plain_pre(source: &str, dark: bool) -> String {
    let mut out = pre_open(dark);
    push_escaped(&mut out, source);
    out.push_str("</pre>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_has_a_working_grammar() {
        for token in TOKENS {
            assert!(CONFIGS.contains_key(token), "{token} has no highlight configuration");
        }
    }

    #[test]
    fn fence_names_and_aliases_resolve() {
        for name in ["rust", "c++", "objective-c", "python", "powershell", "jsonc", "scss", "qml", "gradle", "dockerignore", "kotlin", "kt"] {
            assert!(supports(name), "{name} should resolve");
        }
        assert!(supports("groovy"));
        assert!(supports("gitignore"));
        assert!(!supports("vb"));
        assert!(!supports("nonsense"));
    }

    #[test]
    fn code_is_escaped_and_coloured() {
        let html = render_code("fn main() { let a = \"<tag>\"; }\n", "rs", true);
        assert!(html.contains("<span style=\"color:"));
        assert!(html.contains("&lt;tag&gt;"));
        assert!(!html.contains("\"<tag>\""));
    }

    #[test]
    fn groovy_and_gitignore_are_coloured() {
        let html = render_code("class Foo { def bar() { return 1 } } // note\n", "groovy", true);
        // `class` is a keyword (red) and the comment keeps its grey — a
        // trailing @spell left on the comment capture would erase it.
        assert!(html.contains("#ff7b72"), "groovy keyword not coloured: {html}");
        assert!(html.contains("#8b949e"), "groovy comment not coloured: {html}");
        let html = render_code("# comment\n*.log\n!important.txt\n", "gitignore", true);
        assert!(html.contains("#8b949e"), "gitignore comment not coloured: {html}");
        assert!(html.contains("#ff7b72"), "gitignore negation not coloured: {html}");
    }

    #[test]
    fn style_and_dockerfile_comments_are_coloured() {
        for language in ["less", "scss"] {
            let html = render_code("// note\n@a: 1;\n", language, true);
            assert!(html.contains("#8b949e"), "{language} comment not coloured: {html}");
        }
        let html = render_code("# note\nFROM alpine\n", "dockerfile", true);
        assert!(html.contains("#8b949e"), "dockerfile comment not coloured: {html}");
        assert!(html.contains("#ff7b72"), "dockerfile keyword not coloured: {html}");
    }

    #[test]
    fn distinct_tokens_get_distinct_colours() {
        let html = render_code("fn main() { let x = 1; } // note\n", "rs", true);
        // keyword, function, number and comment must not collapse to one colour.
        let colors: Vec<&str> = html
            .match_indices("color:")
            .map(|(i, _)| &html[i + 6..i + 13])
            .collect();
        let unique: std::collections::HashSet<&&str> = colors.iter().collect();
        assert!(unique.len() >= 3, "expected several colours, got {colors:?}");
    }

    #[test]
    fn latex_commands_and_headings_are_coloured() {
        let html = render_code("\\section{Title} \\textbf{bold}\n", "tex", true);
        // \section is @module and its title @markup.heading.3 (folded onto
        // `markup.heading`, bold orange); \textbf keeps the global @function
        // purple because its dedicated pattern only captures the argument.
        assert!(html.contains("color:#ffa657;font-weight:600;"), "heading not bold-orange: {html}");
        assert!(html.contains("#d2a8ff"), "command not coloured: {html}");
    }

    #[test]
    fn unsupported_language_is_plain_but_escaped() {
        let html = render_code("a < b & c\n", "vb", true);
        assert!(html.contains("a &lt; b &amp; c"));
        assert!(!html.contains("<span"));
    }

    #[test]
    fn kotlin_is_coloured() {
        let html = render_code("// note\nfun main() { val x: Int = 1; println(\"hi\") }\n", "kotlin", true);
        assert!(html.contains("#ff7b72"), "kotlin keyword not coloured: {html}");
        assert!(html.contains("#8b949e"), "kotlin comment not coloured: {html}");
        assert!(html.contains("#7ee787"), "kotlin builtin type not coloured: {html}");
        assert!(html.contains("#a5d6ff"), "kotlin string not coloured: {html}");
        assert!(html.contains("#d2a8ff"), "kotlin function not coloured: {html}");
    }

    #[test]
    fn code_is_rendered_on_the_twelve_pixel_grid() {
        let html = render_code("fn main() {}\n", "rs", true);
        assert!(html.contains("font-size:12px"));
        assert!(html.contains("line-height:18px"));
        assert!(html.contains("white-space:pre;"));
    }

    #[test]
    fn tabs_expand_before_highlighting() {
        let html = render_code("\tfn main() {}\n", "rs", true);
        // The expanded indent sits outside the keyword's span: four plain
        // spaces, then the coloured `fn`.
        assert!(html.contains("    <span"));
        assert!(!html.contains('\t'));
    }

    /// A palette edit that drops a colour below AA body contrast fails here
    /// instead of shipping as a washed-out pane.
    #[test]
    fn the_palette_keeps_text_readable() {
        for dark in [true, false] {
            let background = if dark { DARK_BG } else { LIGHT_BG };
            let bg = parse_hex(background);
            for (name, dark_color, light_color) in PALETTE {
                let fg = parse_hex(if dark { dark_color } else { light_color });
                assert!(
                    contrast_ratio(fg, bg) >= 4.5,
                    "{name} in the {} theme is below 4.5:1",
                    if dark { "dark" } else { "light" }
                );
            }
        }
    }

    fn parse_hex(hex: &str) -> (u8, u8, u8) {
        let hex = hex.trim_start_matches('#');
        (
            u8::from_str_radix(&hex[0..2], 16).unwrap(),
            u8::from_str_radix(&hex[2..4], 16).unwrap(),
            u8::from_str_radix(&hex[4..6], 16).unwrap(),
        )
    }

    fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
        let (high, low) = if luminance(a) >= luminance(b) { (a, b) } else { (b, a) };
        (luminance(high) + 0.05) / (luminance(low) + 0.05)
    }

    fn luminance(color: (u8, u8, u8)) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
    }
}