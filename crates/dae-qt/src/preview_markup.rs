//! Rich rendering for the preview pane.
//!
//! Markdown is translated into the HTML subset `QTextDocument` understands and
//! code is tokenised with syntect into coloured spans. Both are pure functions
//! over the cached source, so the theme can be re-applied without touching the
//! file again.
//!
//! Anything that would need a second resource — raw HTML, remote images,
//! non-web link schemes — is deliberately dropped or rendered as text. The
//! pane is an inspector, not a browser.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;

use percent_encoding::percent_decode_str;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use url::Url;

/// What the text preview of a file is: plain, a Markdown document, or source
/// code with a language token syntect can look up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SourceKind {
    #[default]
    Plain,
    Markdown,
    Code(String),
}

impl SourceKind {
    pub fn is_markdown(&self) -> bool {
        matches!(self, Self::Markdown)
    }
}

/// Classifies a display name. Markdown wins over the code table because the
/// extension is also a syntect language (and the document is what the user
/// wants to read, not its markup).
pub fn classify(name: &str) -> SourceKind {
    let lower = name.to_ascii_lowercase();
    let extension = lower
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or_default();
    if matches!(extension, "md" | "markdown") {
        return SourceKind::Markdown;
    }
    language_token(name).map_or(SourceKind::Plain, SourceKind::Code)
}

/// Renders `source` for the given kind. Plain text has no rich representation;
/// the panel keeps it in a selectable plain `TextEdit`.
pub fn render(source: &str, source_path: &str, kind: &SourceKind, dark: bool) -> String {
    match kind {
        SourceKind::Plain => String::new(),
        SourceKind::Markdown => render_markdown(source, source_path, dark),
        SourceKind::Code(language) => render_code(source, language, dark),
    }
}

/// A `file://` URL for a local path. `None` when the path cannot be expressed
/// as one (a remote VFS path, for example).
pub fn file_url(path: &str) -> Option<String> {
    Url::from_file_path(Path::new(path))
        .ok()
        .map(|url| url.to_string())
}

// ---------------------------------------------------------------------------
// Code
// ---------------------------------------------------------------------------

/// File extensions and well-known names offered a code preview, mapped to the
/// token syntect looks up. Dialects without a dedicated grammar borrow the
/// closest supported one, matching the old webview preview.
fn language_token(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        "dockerfile" => Some("dockerfile"),
        "makefile" | "gnumakefile" => Some("makefile"),
        "cmakelists.txt" | "cmake" => Some("cmake"),
        "cargo.lock" => Some("toml"),
        "gemfile" | "rakefile" | "procfile" => Some("ruby"),
        _ => None,
    };
    if named.is_some() {
        return named.map(str::to_owned);
    }

    let extension = if let Some(dot_name) = lower.strip_prefix('.') {
        dot_name
    } else {
        lower.rsplit_once('.').map(|(_, extension)| extension)?
    };
    let token = match extension {
        "ts" | "mts" | "cts" | "tsx" => "ts",
        "js" | "mjs" | "cjs" | "jsx" => "js",
        "json" | "jsonc" | "json5" => "json",
        "css" | "scss" | "less" | "styl" => "css",
        "html" | "htm" | "xml" | "svg" | "xhtml" | "vue" | "svelte" | "astro" => "html",
        "py" | "pyi" => "py",
        "rb" => "rb",
        "rs" => "rs",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kt",
        "scala" => "scala",
        "groovy" | "gradle" => "groovy",
        "c" => "c",
        "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "m" | "mm" => "m",
        "cs" => "cs",
        "fs" => "fs",
        "vb" => "vb",
        "sh" | "bash" | "zsh" | "fish" => "sh",
        "ps1" | "psm1" => "ps1",
        "sql" => "sql",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "conf" | "cfg" | "properties" | "editorconfig" | "env" => "ini",
        "dockerfile" => "dockerfile",
        "lua" => "lua",
        "php" => "php",
        "gitignore" | "gitattributes" | "dockerignore" => "gitignore",
        "swift" => "swift",
        "dart" => "dart",
        "graphql" | "gql" => "graphql",
        "proto" => "proto",
        "tex" | "bib" | "sty" => "tex",
        "nix" => "nix",
        "mk" => "makefile",
        _ => return None,
    };
    Some(token.to_owned())
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    SET.get_or_init(ThemeSet::load_defaults)
}

fn theme(dark: bool) -> &'static Theme {
    let set = theme_set();
    let name = if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    };
    set.themes
        .get(name)
        .or_else(|| set.themes.get("base16-ocean.dark"))
        .expect("syntect ships its default themes")
}

fn resolve_syntax<'a>(set: &'a SyntaxSet, token: &str) -> &'a SyntaxReference {
    let token = token.to_ascii_lowercase();
    set.find_syntax_by_extension(&token)
        .or_else(|| set.find_syntax_by_token(&token))
        .or_else(|| {
            let alias = match token.as_str() {
                "ts" => "typescript",
                "js" => "javascript",
                "py" => "python",
                "rb" => "ruby",
                "sh" => "shell",
                "ps1" => "powershell",
                "kt" => "kotlin",
                "cs" => "c#",
                "fs" => "f#",
                "vb" => "vb.net",
                "m" => "objective-c",
                "cpp" | "c" => "c++",
                "makefile" => "make",
                "gitignore" => "git",
                _ => return None,
            };
            set.find_syntax_by_token(alias)
                .or_else(|| set.find_syntax_by_extension(alias))
        })
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

fn render_code(source: &str, language: &str, dark: bool) -> String {
    let set = syntax_set();
    let theme = theme(dark);
    let syntax = resolve_syntax(set, language);
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = String::with_capacity(source.len() * 2);
    out.push_str(&pre_open(theme, dark));
    for line in LinesWithEndings::from(source) {
        match highlighter.highlight_line(line, set) {
            Ok(ranges) => {
                for (style, text) in ranges {
                    push_span(&mut out, style, text);
                }
            }
            Err(_) => push_escaped(&mut out, line),
        }
    }
    out.push_str("</pre>");
    out
}

fn pre_open(theme: &Theme, dark: bool) -> String {
    let background = theme
        .settings
        .background
        .map(color_hex)
        .unwrap_or_else(|| if dark { "#20242b" } else { "#f6f8fa" }.to_owned());
    let foreground = theme
        .settings
        .foreground
        .map(color_hex)
        .unwrap_or_else(|| if dark { "#e6e9ef" } else { "#1c1f24" }.to_owned());
    format!(
        "<pre style=\"margin:0; padding:8px; background-color:{background}; color:{foreground}; \
         font-family:'Cascadia Mono'; font-size:11px; line-height:150%; \
         white-space:pre-wrap;\">"
    )
}

fn push_span(out: &mut String, style: syntect::highlighting::Style, text: &str) {
    if text.is_empty() {
        return;
    }
    let mut css = format!("color:{};", color_hex(style.foreground));
    if style.font_style.contains(FontStyle::BOLD) {
        css.push_str("font-weight:600;");
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        css.push_str("font-style:italic;");
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        css.push_str("text-decoration:underline;");
    }
    out.push_str("<span style=\"");
    out.push_str(&css);
    out.push_str("\">");
    push_escaped(out, text);
    out.push_str("</span>");
}

fn color_hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

struct Palette {
    text: &'static str,
    muted: &'static str,
    hairline: &'static str,
    link: &'static str,
    inline_code_bg: &'static str,
    inline_code_fg: &'static str,
    table_border: &'static str,
    table_head_bg: &'static str,
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                text: "#e6e9ef",
                muted: "#8f98aa",
                hairline: "#3a414c",
                link: "#6ea8ff",
                inline_code_bg: "#262b33",
                inline_code_fg: "#d6deeb",
                table_border: "#4a5260",
                table_head_bg: "#262b33",
            }
        } else {
            Self {
                text: "#1c1f24",
                muted: "#657080",
                hairline: "#dfe4ea",
                link: "#2f6fdf",
                inline_code_bg: "#f1f3f5",
                inline_code_fg: "#24292f",
                table_border: "#c9d1db",
                table_head_bg: "#eef1f4",
            }
        }
    }
}

struct Renderer<'a> {
    out: String,
    source_path: &'a str,
    local: bool,
    dark: bool,
    palette: Palette,
    list_depth: usize,
    code: Option<(String, String)>,
    image: Option<(String, String)>,
    link_open: Vec<bool>,
    table_alignments: Vec<Alignment>,
    table_cell: usize,
    in_table_head: bool,
    in_metadata: bool,
}

pub fn render_markdown(source: &str, source_path: &str, dark: bool) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_DEFINITION_LIST);

    let mut renderer = Renderer {
        out: String::with_capacity(source.len() * 2),
        source_path,
        local: !source_path.contains("://"),
        dark,
        palette: Palette::new(dark),
        list_depth: 0,
        code: None,
        image: None,
        link_open: Vec::new(),
        table_alignments: Vec::new(),
        table_cell: 0,
        in_table_head: false,
        in_metadata: false,
    };
    renderer.out.push_str("<div style=\"line-height:150%;\">");
    for event in Parser::new_ext(source, options) {
        renderer.event(event);
    }
    renderer.out.push_str("</div>");
    renderer.out
}

impl Renderer<'_> {
    fn event(&mut self, event: Event<'_>) {
        if let Some((_, alt)) = self.image.as_mut() {
            match event {
                Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                Event::SoftBreak | Event::HardBreak => alt.push(' '),
                Event::End(TagEnd::Image) => {
                    let (destination, alt) = self.image.take().expect("image is open");
                    self.emit_image(&destination, &alt);
                }
                _ => {}
            }
            return;
        }

        if self.in_metadata {
            if matches!(event, Event::End(TagEnd::MetadataBlock(_))) {
                self.in_metadata = false;
            }
            return;
        }

        if let Some((_, buffer)) = self.code.as_mut() {
            match event {
                Event::Text(text) => buffer.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    let (language, text) = self.code.take().expect("code block is open");
                    self.emit_code_block(&language, &text);
                }
                _ => {}
            }
            return;
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => push_escaped(&mut self.out, &text),
            Event::Code(code) => self.inline_code(&code),
            Event::InlineMath(text) | Event::DisplayMath(text) => self.inline_code(&text),
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(label) => {
                self.out.push_str("<sup>[");
                push_escaped(&mut self.out, &label);
                self.out.push_str("]</sup>");
            }
            Event::SoftBreak => self.out.push('\n'),
            Event::HardBreak => self.out.push_str("<br>"),
            Event::Rule => {
                let _ = write!(
                    self.out,
                    "<hr style=\"border-top:1px solid {}; margin:10px 0;\">",
                    self.palette.hairline
                );
            }
            Event::TaskListMarker(checked) => {
                self.out.push_str(if checked { "☑ " } else { "☐ " });
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.list_depth > 0 {
                    self.out.push_str("<p style=\"margin:0;\">");
                } else {
                    self.out.push_str("<p style=\"margin:0 0 8px;\">");
                }
            }
            Tag::Heading { level, .. } => {
                let (size, top, bottom) = heading_metrics(level);
                let _ = write!(
                    self.out,
                    "<p style=\"font-size:{size}px; font-weight:600; color:{}; \
                     margin:{top}px 0 {bottom}px;\">",
                    self.palette.text
                );
            }
            Tag::BlockQuote(_) => {
                let _ = write!(
                    self.out,
                    "<blockquote style=\"margin:6px 0 6px 8px; padding-left:8px; \
                     border-left:2px solid {}; color:{};\">",
                    self.palette.hairline, self.palette.muted
                );
            }
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Indented => String::new(),
                    CodeBlockKind::Fenced(info) => fence_language(&info),
                };
                self.code = Some((language, String::new()));
            }
            Tag::HtmlBlock => {}
            Tag::List(start) => {
                self.list_depth += 1;
                match start {
                    Some(1) | None => self.out.push_str(&format!(
                        "<{} style=\"margin:0 0 8px; padding-left:18px;\">",
                        if start.is_some() { "ol" } else { "ul" }
                    )),
                    Some(first) => {
                        let _ = write!(
                            self.out,
                            "<ol start=\"{first}\" style=\"margin:0 0 8px; padding-left:18px;\">"
                        );
                    }
                }
            }
            Tag::Item => self.out.push_str("<li>"),
            Tag::FootnoteDefinition(label) => {
                self.out
                    .push_str("<div style=\"margin:0 0 6px; font-size:10.5px; color:");
                self.out.push_str(self.palette.muted);
                self.out.push_str(";\"><sup>");
                push_escaped(&mut self.out, &label);
                self.out.push_str("</sup> ");
            }
            Tag::DefinitionList => self
                .out
                .push_str("<dl style=\"margin:0 0 8px; padding-left:4px;\">"),
            Tag::DefinitionListTitle => self.out.push_str("<dt style=\"font-weight:600;\">"),
            Tag::DefinitionListDefinition => self.out.push_str("<dd style=\"margin-left:16px;\">"),
            Tag::Table(alignments) => {
                self.table_alignments = alignments;
                // Qt's rich text engine only paints a grid when the table
                // carries `border` and a non-zero `cellspacing`; a collapsed
                // table with per-cell borders renders as bare text.
                let _ = write!(
                    self.out,
                    "<table border=\"1\" cellspacing=\"1\" cellpadding=\"4\" \
                     style=\"border-color:{}; margin:8px 0; font-size:11px;\">",
                    self.palette.table_border
                );
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.table_cell = 0;
                self.out.push_str("<thead><tr>");
            }
            Tag::TableRow => {
                self.table_cell = 0;
                self.out.push_str("<tr>");
            }
            Tag::TableCell => {
                let alignment = self
                    .table_alignments
                    .get(self.table_cell)
                    .copied()
                    .unwrap_or(Alignment::None);
                self.table_cell += 1;
                let tag = if self.in_table_head { "th" } else { "td" };
                let _ = write!(
                    self.out,
                    "<{tag} align=\"{}\" style=\"border-width:1px; border-style:solid; \
                     border-color:{}; {}padding:3px 5px;\">",
                    alignment_name(alignment),
                    self.palette.table_border,
                    if self.in_table_head {
                        format!(
                            "background-color:{}; font-weight:600; ",
                            self.palette.table_head_bg
                        )
                    } else {
                        String::new()
                    }
                );
            }
            Tag::Emphasis => self.out.push_str("<i>"),
            Tag::Strong => self.out.push_str("<b>"),
            Tag::Strikethrough => self.out.push_str("<s>"),
            Tag::Superscript => self.out.push_str("<sup>"),
            Tag::Subscript => self.out.push_str("<sub>"),
            Tag::Link { dest_url, .. } => {
                let target = self.link_target(&dest_url);
                self.link_open.push(target.is_some());
                if let Some(target) = target {
                    self.out.push_str("<a href=\"");
                    push_escaped(&mut self.out, &target);
                    let _ = write!(self.out, "\" style=\"color:{};\">", self.palette.link);
                }
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some((dest_url.into_string(), String::new()));
            }
            Tag::MetadataBlock(_) => self.in_metadata = true,
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.out.push_str("</p>"),
            TagEnd::BlockQuote(_) => self.out.push_str("</blockquote>"),
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {}
            TagEnd::List(ordered) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.out.push_str(if ordered { "</ol>" } else { "</ul>" });
            }
            TagEnd::Item => self.out.push_str("</li>"),
            TagEnd::FootnoteDefinition => self.out.push_str("</div>"),
            TagEnd::DefinitionList => self.out.push_str("</dl>"),
            TagEnd::DefinitionListTitle => self.out.push_str("</dt>"),
            TagEnd::DefinitionListDefinition => self.out.push_str("</dd>"),
            TagEnd::Table => {
                self.table_alignments.clear();
                self.out.push_str("</table>");
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.out.push_str("</tr></thead>");
            }
            TagEnd::TableRow => self.out.push_str("</tr>"),
            TagEnd::TableCell => {
                self.out
                    .push_str(if self.in_table_head { "</th>" } else { "</td>" });
            }
            TagEnd::Emphasis => self.out.push_str("</i>"),
            TagEnd::Strong => self.out.push_str("</b>"),
            TagEnd::Strikethrough => self.out.push_str("</s>"),
            TagEnd::Superscript => self.out.push_str("</sup>"),
            TagEnd::Subscript => self.out.push_str("</sub>"),
            TagEnd::Link => {
                if self.link_open.pop().unwrap_or(false) {
                    self.out.push_str("</a>");
                }
            }
            TagEnd::Image => {}
            TagEnd::MetadataBlock(_) => self.in_metadata = false,
        }
    }

    fn inline_code(&mut self, code: &str) {
        let _ = write!(
            self.out,
            "<span style=\"font-family:'Cascadia Mono'; color:{}; \
             background-color:{};\">",
            self.palette.inline_code_fg, self.palette.inline_code_bg
        );
        push_escaped(&mut self.out, code);
        self.out.push_str("</span>");
    }

    fn emit_code_block(&mut self, language: &str, text: &str) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        if language.is_empty() || resolve_syntax(syntax_set(), language).name == "Plain Text" {
            self.out.push_str(&pre_open(theme(self.dark), self.dark));
            push_escaped(&mut self.out, text);
            self.out.push_str("</pre>");
        } else {
            self.out.push_str(&render_code(text, language, self.dark));
        }
    }

    fn emit_image(&mut self, destination: &str, alt: &str) {
        if scheme(destination).is_none()
            && self.local
            && let Some(path) = resolve_local_target(self.source_path, destination)
            && let Some(url) = file_url(&path)
        {
            self.out.push_str("<img src=\"");
            push_escaped(&mut self.out, &url);
            let _ = write!(
                self.out,
                "\" width=\"236\" alt=\"{}\" style=\"margin:4px 0;\">",
                escape_attribute(alt)
            );
            return;
        }
        push_escaped(&mut self.out, alt);
    }

    fn link_target(&self, destination: &str) -> Option<String> {
        if destination.starts_with('#') {
            return None;
        }
        if let Some(scheme) = scheme(destination) {
            let scheme = scheme.to_ascii_lowercase();
            return matches!(scheme.as_str(), "http" | "https" | "mailto")
                .then(|| destination.to_owned());
        }
        self.local
            .then(|| resolve_local_target(self.source_path, destination))
            .flatten()
    }
}

fn heading_metrics(level: HeadingLevel) -> (u8, u8, u8) {
    match level {
        HeadingLevel::H1 => (17, 12, 6),
        HeadingLevel::H2 => (15, 12, 5),
        HeadingLevel::H3 => (13, 10, 4),
        HeadingLevel::H4 => (12, 10, 4),
        HeadingLevel::H5 => (11, 8, 3),
        HeadingLevel::H6 => (11, 8, 3),
    }
}

fn alignment_name(alignment: Alignment) -> &'static str {
    match alignment {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
        Alignment::None => "left",
    }
}

fn fence_language(info: &str) -> String {
    info.split(|character: char| character.is_whitespace() || character == ',')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches('{')
        .trim_start_matches('.')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

fn scheme(destination: &str) -> Option<&str> {
    // Two letters minimum: `C:\notes.md` is a drive-absolute path, not a
    // `c:` scheme.
    let colon = destination.find(':')?;
    if colon < 2 {
        return None;
    }
    let candidate = &destination[..colon];
    let mut characters = candidate.chars();
    if !characters.next()?.is_ascii_alphabetic() {
        return None;
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-'))
        .then_some(candidate)
}

/// Resolves a document-relative destination against the document that named
/// it, folding `.`/`..`, percent-decoding segments and dropping a fragment.
/// Mirrors `src/features/explorer/markdown-links.ts`.
pub fn resolve_local_target(source_path: &str, destination: &str) -> Option<String> {
    if destination.is_empty() || destination.starts_with('#') {
        return None;
    }
    let cleaned: String = destination
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let without_fragment = cleaned.split('#').next().unwrap_or_default();
    if without_fragment.is_empty() {
        return None;
    }

    let separator = if source_path.contains('\\') {
        '\\'
    } else {
        '/'
    };
    let normalized = without_fragment.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    let drive_rooted = parts.first().is_some_and(|part| is_drive(part));
    let rooted = drive_rooted || without_fragment.starts_with('/');
    // A `C:\…` destination inside a POSIX document (or the reverse) has no
    // answer worth inventing.
    if rooted && drive_rooted != (separator == '\\') {
        return None;
    }

    let mut segments: Vec<String> = if rooted {
        Vec::new()
    } else {
        let mut parent: Vec<&str> = source_path.split(['\\', '/']).collect();
        parent.pop();
        parent.into_iter().map(str::to_owned).collect()
    };

    for raw in parts {
        let part = percent_decode_str(raw).decode_utf8_lossy();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if segments.last().is_some_and(|last| !is_root_segment(last)) {
                segments.pop();
            }
            continue;
        }
        segments.push(part.into_owned());
    }

    if segments.is_empty() {
        return None;
    }
    let joined = segments.join(&separator.to_string());
    if rooted && separator == '/' {
        Some(format!("/{joined}"))
    } else {
        Some(joined)
    }
}

fn is_drive(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_root_segment(segment: &str) -> bool {
    segment.is_empty() || is_drive(segment)
}

fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
}

fn escape_attribute(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    push_escaped(&mut escaped, text);
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_markdown_and_code_by_name() {
        assert_eq!(classify("README.md"), SourceKind::Markdown);
        assert_eq!(classify("notes.MARKDOWN"), SourceKind::Markdown);
        assert_eq!(classify("main.rs"), SourceKind::Code("rs".to_owned()));
        assert_eq!(
            classify("Dockerfile"),
            SourceKind::Code("dockerfile".to_owned())
        );
        assert_eq!(classify("notes.txt"), SourceKind::Plain);
        assert_eq!(classify("archive.zip"), SourceKind::Plain);
    }

    #[test]
    fn resolves_relative_links_against_a_windows_document() {
        assert_eq!(
            resolve_local_target(r"C:\docs\readme.md", "./img/shot.png").as_deref(),
            Some(r"C:\docs\img\shot.png")
        );
        assert_eq!(
            resolve_local_target(r"C:\docs\readme.md", "../guide.md").as_deref(),
            Some(r"C:\guide.md")
        );
        assert_eq!(
            resolve_local_target(r"C:\docs\readme.md", r"sub\page.md").as_deref(),
            Some(r"C:\docs\sub\page.md")
        );
        assert_eq!(
            resolve_local_target(r"C:\docs\readme.md", "C:\\absolute\\x.md").as_deref(),
            Some(r"C:\absolute\x.md")
        );
    }

    #[test]
    fn resolves_relative_links_against_a_posix_document() {
        assert_eq!(
            resolve_local_target("/home/me/docs/readme.md", "img/x.png").as_deref(),
            Some("/home/me/docs/img/x.png")
        );
        assert_eq!(
            resolve_local_target("/home/me/docs/readme.md", "/etc/hosts").as_deref(),
            Some("/etc/hosts")
        );
        assert_eq!(
            resolve_local_target("/home/me/docs/readme.md", "a%20b.md").as_deref(),
            Some("/home/me/docs/a b.md")
        );
    }

    #[test]
    fn fragments_and_mismatched_roots_do_not_resolve() {
        assert_eq!(resolve_local_target(r"C:\docs\readme.md", "#section"), None);
        assert_eq!(resolve_local_target("/home/me/readme.md", r"C:\x.md"), None);
        assert_eq!(
            resolve_local_target(r"C:\docs\readme.md", "/rooted/x.md"),
            None
        );
    }

    #[test]
    fn two_letter_schemes_are_not_drive_paths() {
        assert_eq!(scheme("https://example.com"), Some("https"));
        assert_eq!(scheme("mailto:a@b.c"), Some("mailto"));
        assert_eq!(scheme(r"C:\notes.md"), None);
        assert_eq!(scheme("dae://open?path=x"), Some("dae"));
    }

    /// Image URLs go through `Url::from_file_path`, which only accepts the
    /// host platform's path syntax; the string resolver above is tested on
    /// every platform.
    #[cfg(windows)]
    #[test]
    fn markdown_renders_structure_links_and_escapes_html() {
        let html = render_markdown(
            "# Title\n\n**bold** and <script>alert(1)</script>\n\n[guide](docs/guide.md)\n\n![shot](img/a.png)\n",
            r"C:\notes\readme.md",
            true,
        );
        assert!(html.contains("<p style=\"font-size:17px"));
        assert!(html.contains("<b>bold</b>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("alert(1)"));
        assert!(html.contains(r#"href="C:\notes\docs\guide.md""#));
        assert!(html.contains("<img src=\"file:///C:/notes/img/a.png\""));
    }

    #[test]
    fn markdown_code_fences_are_highlighted() {
        let html = render_markdown(
            "```rust\nfn main() { let x = 1; }\n```\n",
            "C:\\notes\\readme.md",
            true,
        );
        assert!(html.contains("<pre style="));
        assert!(html.contains("<span style=\"color:"));
        assert!(html.contains("main"));
    }

    #[test]
    fn code_is_escaped_and_coloured() {
        let html = render_code("fn main() { let a = \"<tag>\"; }\n", "rs", true);
        assert!(html.contains("<span style=\"color:"));
        assert!(html.contains("&lt;tag&gt;"));
        assert!(!html.contains("\"<tag>\""));
    }

    #[cfg(windows)]
    #[test]
    fn file_urls_encode_spaces_and_drive_letters() {
        let url = file_url(r"C:\notes\a b.md").expect("local path has a URL");
        assert_eq!(url, "file:///C:/notes/a%20b.md");
    }
}
