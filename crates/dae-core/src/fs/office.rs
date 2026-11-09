//! Office document previews: the text a Word, Excel or PowerPoint file carries.
//!
//! The pane is an inspector, not a layout engine. An OOXML document is a ZIP
//! of XML parts, so this module opens the container, reads the parts that hold
//! text and returns display-ready sections. Rendering happens in the shell,
//! the same split `preview` and `media_meta` already use.
//!
//! What it deliberately does not do: paginate, evaluate formulas, resolve
//! styles beyond heading detection, or touch legacy binary `.doc`/`.xls`/`.ppt`
//! (OLE compound files, not ZIPs). Those, and the OpenDocument formats, are
//! reported by name so the pane can say "not supported" instead of showing
//! binary noise.
//!
//! Every read is bounded twice: the container by [`MAX_CONTAINER_BYTES`, the
//! parts by [`MAX_PART_BYTES`], and the output by [`MAX_LINES`]. A ZIP bomb or
//! a 400-page report therefore costs a fixed amount of memory and one preview.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use super::error::FileSystemError;
use super::types::{EntryKind, display_name_from_path};
use super::vfs;

/// Largest container read as a whole. Office files are mostly compressed media
/// beyond this, and the preview would only show the first pages anyway.
const MAX_CONTAINER_BYTES: u64 = 48 * 1024 * 1024;
/// Uncompressed ceiling for one XML part, which is where a ZIP bomb would show
/// up: the part is read through `Read::take`, not by trusting the header size.
const MAX_PART_BYTES: u64 = 8 * 1024 * 1024;
/// Lines the pane is given across all sections.
const MAX_LINES: usize = 220;
/// Rows kept from one worksheet; a spreadsheet preview is a sample, not a copy.
const MAX_TABLE_ROWS: usize = 40;
/// Columns kept from one worksheet row.
const MAX_TABLE_COLUMNS: usize = 10;
/// Slides kept from one deck.
const MAX_SLIDES: usize = 12;
/// Worksheets kept from one workbook.
const MAX_SHEETS: usize = 6;
/// Characters kept from one line. A paragraph without breaks is untrustworthy
/// to show unclipped in a 300 px column.
const MAX_LINE_CHARS: usize = 400;

/// How a line should read. The shell maps these to its own type scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeLineKind {
    /// Body text: a paragraph, a bullet, a line of a text box.
    Text,
    /// A heading or a slide title.
    Heading,
    /// A table row or a worksheet row: drawn in the mono face.
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeLine {
    pub text: String,
    pub kind: OfficeLineKind,
}

/// One sheet, or one slide, or (for a document) the whole body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeSection {
    /// The sheet name, empty for a document or a slide.
    pub heading: String,
    pub lines: Vec<OfficeLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeKind {
    Word,
    Workbook,
    Slides,
}

impl OfficeKind {
    /// The extension family the shell reports in `officeFormat`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Word => "docx",
            Self::Workbook => "xlsx",
            Self::Slides => "pptx",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficePreview {
    pub kind: OfficeKind,
    pub sections: Vec<OfficeSection>,
    /// At least one bound was hit: more text exists than the pane shows.
    pub truncated: bool,
}

/// The Word-family OOXML extensions.
const WORD_EXTENSIONS: &[&str] = &["docx", "docm", "dotx", "dotm"];
const WORKBOOK_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xltx", "xltm"];
const SLIDES_EXTENSIONS: &[&str] = &["pptx", "pptm", "potx", "potm", "ppsx", "ppsm"];
/// Binary formats that need a converter before anything can be read.
const LEGACY_EXTENSIONS: &[&str] = &[
    "doc", "dot", "xls", "xlt", "xla", "ppt", "pot", "pps", "wps", "et", "dps",
];
/// ZIP/XML formats this module does not parse (yet), kept apart from the
/// legacy list so the message can be accurate.
const UNSUPPORTED_EXTENSIONS: &[&str] = &["odt", "ods", "odp", "rtf", "pages", "numbers", "key"];

fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The Office family a name belongs to, or `None` when it has no preview.
pub fn office_kind(name: &str) -> Option<OfficeKind> {
    let extension = extension_of(name);
    if WORD_EXTENSIONS.contains(&extension.as_str()) {
        Some(OfficeKind::Word)
    } else if WORKBOOK_EXTENSIONS.contains(&extension.as_str()) {
        Some(OfficeKind::Workbook)
    } else if SLIDES_EXTENSIONS.contains(&extension.as_str()) {
        Some(OfficeKind::Slides)
    } else {
        None
    }
}

pub fn is_office_path(name: &str) -> bool {
    office_kind(name).is_some()
}

/// `.doc`, `.xls` and `.ppt` (and their siblings): readable only after the user
/// saves a modern copy, so the pane says that rather than staying blank.
pub fn is_legacy_office_path(name: &str) -> bool {
    LEGACY_EXTENSIONS.contains(&extension_of(name).as_str())
}

/// A document format dae knows by name but does not parse.
pub fn is_unsupported_office_path(name: &str) -> bool {
    UNSUPPORTED_EXTENSIONS.contains(&extension_of(name).as_str())
}

/// Reads one Office document. Errors carry `fs.office_*` codes: the bridge
/// turns them into a sentence and the QML localiser into the UI language.
pub async fn read_office_preview(
    path: String,
    max_bytes: u32,
) -> Result<OfficePreview, FileSystemError> {
    tokio::task::spawn_blocking(move || read_blocking(path, max_bytes))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn read_blocking(path: String, max_bytes: u32) -> Result<OfficePreview, FileSystemError> {
    let name = display_name_from_path(&path);
    let kind = office_kind(&name)
        .ok_or_else(|| FileSystemError::Unsupported("fs.office_unsupported".to_owned()))?;

    let limit = u64::from(max_bytes.clamp(1, MAX_CONTAINER_BYTES as u32));
    let backend = vfs::resolve(&path)?;
    let (size, is_directory) = if vfs::is_local_path(&path) {
        let metadata = std::fs::metadata(&path).map_err(FileSystemError::from)?;
        (metadata.len(), metadata.is_dir())
    } else {
        let stat = backend.stat(&path)?;
        (stat.size, stat.kind == EntryKind::Directory)
    };
    if is_directory {
        return Err(FileSystemError::InvalidInput(path));
    }
    if size > limit {
        return Err(FileSystemError::InvalidInput(
            "fs.office_too_large".to_owned(),
        ));
    }

    let mut bytes = Vec::with_capacity(size.min(limit) as usize);
    backend
        .open_read(&path)?
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(FileSystemError::from)?;

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| FileSystemError::InvalidInput("fs.office_unreadable".to_owned()))?;

    let mut truncated = false;
    let sections = match kind {
        OfficeKind::Word => {
            let xml = read_part(&mut archive, "word/document.xml")
                .ok_or_else(|| FileSystemError::InvalidInput("fs.office_unreadable".to_owned()))?;
            let (lines, clipped) = parse_document(&xml);
            truncated |= clipped;
            if lines.is_empty() {
                return Err(FileSystemError::InvalidInput(
                    "fs.office_empty".to_owned(),
                ));
            }
            vec![OfficeSection {
                heading: String::new(),
                lines,
            }]
        }
        OfficeKind::Workbook => {
            let (sections, clipped) = read_workbook(&mut archive)?;
            truncated |= clipped;
            sections
        }
        OfficeKind::Slides => {
            let (sections, clipped) = read_deck(&mut archive)?;
            truncated |= clipped;
            sections
        }
    };

    Ok(OfficePreview {
        kind,
        sections,
        truncated,
    })
}

/// Reads one ZIP entry as text, bounded by `MAX_PART_BYTES`. `None` for a
/// missing or damaged part: an incomplete deck still previews its other slides.
fn read_part(archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Option<String> {
    let entry = archive.by_name(name).ok()?;
    let mut buffer = Vec::new();
    entry
        .take(MAX_PART_BYTES)
        .read_to_end(&mut buffer)
        .ok()?;
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

fn attribute(element: &quick_xml::events::BytesStart<'_>, wanted: &str) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.local_name().as_ref() == wanted)
        .and_then(|attribute| attribute.normalized_value(XmlVersion::default()).ok())
        .map(|value| value.into_owned())
        .filter(|value| !value.is_empty())
}

/// The namespaced `r:id` of a relationship reference. Office elements also
/// carry a plain `id` (a slide's creation counter, a sheet's ordinal), so the
/// prefixed attribute is the one that names a part.
fn relationship_id(element: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    element
        .attributes()
        .flatten()
        .filter(|attribute| attribute.key.local_name().as_ref() == "id")
        .max_by_key(|attribute| attribute.key.prefix().is_some())
        .and_then(|attribute| attribute.normalized_value(XmlVersion::default()).ok())
        .map(|value| value.into_owned())
        .filter(|value| !value.is_empty())
}

/// Folds an entity reference back into the text. Word splits `&` into its own
/// event, so a title without this reads "Costs  Profits".
fn push_reference(text: &mut String, reference: &quick_xml::events::BytesRef<'_>) {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        text.push(character);
        return;
    }
    match reference.xml10_content().as_ref() {
        "amp" => text.push('&'),
        "lt" => text.push('<'),
        "gt" => text.push('>'),
        "quot" => text.push('"'),
        "apos" => text.push('\''),
        _ => {}
    }
}

fn clamp_line(text: &str) -> String {
    // Pretty-printed XML (and Word's own indentation between runs) leaves
    // newlines and runs of spaces inside a paragraph. The pane has one line per
    // paragraph, so they collapse to a single space before the length clip.
    let mut collapsed = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = !collapsed.is_empty();
            continue;
        }
        if pending_space {
            collapsed.push(' ');
            pending_space = false;
        }
        collapsed.push(character);
    }
    if collapsed.chars().count() <= MAX_LINE_CHARS {
        return collapsed;
    }
    clip_line(&collapsed)
}

/// The length clip on its own. Table rows keep their empty cells: "| |" is how
/// a reader sees that a column has no value, and collapsing the gap would hide
/// the alignment.
fn clip_line(text: &str) -> String {
    let mut clipped: String = text.chars().take(MAX_LINE_CHARS).collect();
    if text.chars().count() > MAX_LINE_CHARS {
        clipped.push('…');
    }
    clipped
}

// ---------------------------------------------------------------------------
// Word
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DocumentBuilder {
    lines: Vec<OfficeLine>,
    truncated: bool,
    paragraph: String,
    heading: bool,
    in_table: bool,
    row: Vec<String>,
    cell: String,
    in_cell: bool,
}

impl DocumentBuilder {
    fn full(&self) -> bool {
        self.lines.len() >= MAX_LINES
    }

    fn push(&mut self, text: &str, kind: OfficeLineKind) {
        if text.trim().is_empty() {
            return;
        }
        if self.lines.len() >= MAX_LINES {
            self.truncated = true;
            return;
        }
        let text = if kind == OfficeLineKind::Table {
            clip_line(text.trim())
        } else {
            clamp_line(text)
        };
        self.lines.push(OfficeLine { text, kind });
    }

    fn end_paragraph(&mut self) {
        let text = std::mem::take(&mut self.paragraph);
        if self.in_cell {
            let text = text.trim();
            if !text.is_empty() {
                if !self.cell.is_empty() {
                    self.cell.push(' ');
                }
                self.cell.push_str(text);
            }
            return;
        }
        let kind = if self.heading {
            OfficeLineKind::Heading
        } else {
            OfficeLineKind::Text
        };
        self.push(&text, kind);
        self.heading = false;
    }

    fn end_row(&mut self) {
        if self.row.is_empty() {
            return;
        }
        let line = self.row.join(" | ");
        self.push(&line, OfficeLineKind::Table);
        self.row.clear();
    }
}

fn parse_document(xml: &str) -> (Vec<OfficeLine>, bool) {
    let mut reader = Reader::from_str(xml);
    let mut builder = DocumentBuilder::default();

    loop {
        if builder.full() {
            builder.truncated = true;
            break;
        }
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                "tbl" => builder.in_table = true,
                "tr" if builder.in_table => builder.row.clear(),
                "tc" if builder.in_table => {
                    builder.in_cell = true;
                    builder.cell.clear();
                }
                "p" => builder.paragraph.clear(),
                // A style id is stable across locales ("Heading1" even in a
                // Chinese Word); the outline level catches the rest.
                "pStyle" => {
                    if let Some(value) = attribute(&element, "val") {
                        let value = value.to_ascii_lowercase();
                        builder.heading = value.starts_with("heading") || value == "title";
                    }
                }
                "outlineLvl" => {
                    if let Some(value) = attribute(&element, "val")
                        && let Ok(level) = value.parse::<u8>()
                        && level < 3
                    {
                        builder.heading = true;
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(element)) => match element.local_name().as_ref() {
                // A tab or a soft break keeps the words apart instead of
                // gluing two runs into one word.
                "tab" | "br" | "cr" => builder.paragraph.push(' '),
                "pStyle" => {
                    if let Some(value) = attribute(&element, "val") {
                        let value = value.to_ascii_lowercase();
                        builder.heading = value.starts_with("heading") || value == "title";
                    }
                }
                "outlineLvl" => {
                    if let Some(value) = attribute(&element, "val")
                        && let Ok(level) = value.parse::<u8>()
                        && level < 3
                    {
                        builder.heading = true;
                    }
                }
                _ => {}
            },
            Ok(Event::Text(value)) => builder.paragraph.push_str(&value),
            Ok(Event::CData(value)) => builder
                .paragraph
                .push_str(&value.xml10_content()),
            Ok(Event::GeneralRef(value)) => push_reference(&mut builder.paragraph, &value),
            Ok(Event::End(element)) => match element.local_name().as_ref() {
                "p" => builder.end_paragraph(),
                "tc" if builder.in_table => {
                    builder.in_cell = false;
                    let cell = std::mem::take(&mut builder.cell);
                    builder.row.push(cell.trim().to_owned());
                }
                "tr" if builder.in_table => builder.end_row(),
                "tbl" => {
                    builder.in_table = false;
                    builder.row.clear();
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            // A damaged part yields what was read before the damage.
            Err(_) => break,
        }
    }

    (builder.lines, builder.truncated)
}

// ---------------------------------------------------------------------------
// Excel
// ---------------------------------------------------------------------------

fn read_workbook(
    archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
) -> Result<(Vec<OfficeSection>, bool), FileSystemError> {
    let shared = read_part(archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let workbook = read_part(archive, "xl/workbook.xml")
        .ok_or_else(|| FileSystemError::InvalidInput("fs.office_unreadable".to_owned()))?;
    let relationships = read_part(archive, "xl/_rels/workbook.xml.rels")
        .map(|xml| parse_relationships(&xml))
        .unwrap_or_default();

    let mut truncated = false;
    let mut sections = Vec::new();
    for (name, relationship) in parse_workbook_sheets(&workbook) {
        if sections.len() >= MAX_SHEETS {
            truncated = true;
            break;
        }
        let Some(target) = relationships.get(&relationship) else {
            continue;
        };
        let part = resolve_part("xl/", target);
        let Some(xml) = read_part(archive, &part) else {
            continue;
        };
        let (lines, clipped) = parse_worksheet(&xml, &shared);
        truncated |= clipped;
        sections.push(OfficeSection {
            heading: name,
            lines,
        });
    }
    if sections.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "fs.office_unreadable".to_owned(),
        ));
    }
    Ok((sections, truncated))
}

/// The sheet list in workbook order. `r:id` maps through the workbook's
/// relationships; `sheetId` is a creation counter and says nothing about the
/// part name, which is why the relationship is the only reliable route.
fn parse_workbook_sheets(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut sheets = Vec::new();
    let mut in_sheets = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                match element.local_name().as_ref() {
                    "sheets" => in_sheets = true,
                    "sheet" if in_sheets => {
                        if let (Some(name), Some(id)) = (
                            attribute(&element, "name"),
                            relationship_id(&element).or_else(|| attribute(&element, "id")),
                        ) {
                            sheets.push((name, id));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(element)) => {
                if element.local_name().as_ref() == "sheets" {
                    in_sheets = false;
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    sheets
}

/// `Id` → `Target` for a `.rels` part. Both the workbook and the presentation
/// use this shape, which is why it is shared.
fn parse_relationships(xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml);
    let mut relationships = HashMap::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() != "Relationship" {
                    continue;
                }
                // `<Relationship Id="rId1" Type="…" Target="worksheets/sheet1.xml"/>`
                if let (Some(id), Some(target)) =
                    (attribute(&element, "Id"), attribute(&element, "Target"))
                {
                    relationships.insert(id, target);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    relationships
}

/// A relationship target is relative to the part that owns the `.rels` file,
/// unless it is package-absolute. `..` segments are folded so the ZIP lookup
/// sees the entry name the container actually stores.
fn resolve_part(base: &str, target: &str) -> String {
    let target = target.replace('\\', "/");
    let joined = if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("{base}{target}")
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

fn parse_shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut strings = Vec::new();
    let mut in_item = false;
    let mut in_phonetic = false;
    let mut in_text = false;
    let mut buffer = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                "si" => {
                    in_item = true;
                    buffer.clear();
                }
                // `<rPh>` holds furigana/phonetic text, not the value.
                "rPh" => in_phonetic = true,
                "t" if in_item && !in_phonetic => in_text = true,
                _ => {}
            },
            Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() == "t" && in_item && !in_phonetic {
                    // An explicitly empty string still occupies an index.
                }
            }
            Ok(Event::Text(value)) => {
                if in_text {
                    buffer.push_str(&value);
                }
            }
            Ok(Event::CData(value)) => {
                if in_text {
                    buffer.push_str(&value.xml10_content());
                }
            }
            Ok(Event::GeneralRef(value)) => {
                if in_text {
                    push_reference(&mut buffer, &value);
                }
            }
            Ok(Event::End(element)) => match element.local_name().as_ref() {
                "t" => in_text = false,
                "rPh" => in_phonetic = false,
                "si" => {
                    in_item = false;
                    strings.push(std::mem::take(&mut buffer));
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    strings
}

fn parse_worksheet(xml: &str, shared: &[String]) -> (Vec<OfficeLine>, bool) {
    let mut reader = Reader::from_str(xml);
    let mut lines = Vec::new();
    let mut truncated = false;
    let mut in_row = false;
    let mut row: Vec<(usize, String)> = Vec::new();
    let mut in_cell = false;
    let mut cell_reference = String::new();
    let mut cell_type = String::new();
    let mut in_value = false;
    let mut in_inline = false;
    let mut in_text = false;
    let mut value = String::new();

    loop {
        if lines.len() >= MAX_TABLE_ROWS {
            truncated = true;
            break;
        }
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                "row" => {
                    in_row = true;
                    row.clear();
                }
                "c" if in_row => {
                    in_cell = true;
                    value.clear();
                    cell_type = attribute(&element, "t").unwrap_or_default();
                    cell_reference = attribute(&element, "r").unwrap_or_default();
                }
                "v" if in_cell => in_value = true,
                "is" if in_cell => in_inline = true,
                "t" if in_inline => in_text = true,
                _ => {}
            },
            Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() == "c" && in_row {
                    // `<c r="B2"/>` is a deliberately empty cell.
                }
            }
            Ok(Event::Text(text)) => {
                if in_value || in_text {
                    value.push_str(&text);
                }
            }
            Ok(Event::CData(text)) => {
                if in_value || in_text {
                    value.push_str(&text.xml10_content());
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if in_value || in_text {
                    push_reference(&mut value, &reference);
                }
            }
            Ok(Event::End(element)) => match element.local_name().as_ref() {
                "v" => in_value = false,
                "t" => in_text = false,
                "is" => in_inline = false,
                "c" if in_cell => {
                    in_cell = false;
                    let text = if cell_type == "s" {
                        value
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared.get(index))
                            .map(String::as_str)
                            .unwrap_or_default()
                    } else {
                        value.trim()
                    };
                    if !text.is_empty()
                        && let Some(column) = column_index(&cell_reference)
                        && column < MAX_TABLE_COLUMNS
                    {
                        row.push((column, text.to_owned()));
                    }
                }
                "row" if in_row => {
                    in_row = false;
                    if !row.is_empty() {
                        row.sort_by_key(|(column, _)| *column);
                        let width = row.last().map(|(column, _)| column + 1).unwrap_or(0);
                        let mut cells = vec![String::new(); width];
                        for (column, text) in row.drain(..) {
                            cells[column] = text;
                        }
                        let line = cells.join(" | ");
                        if !line.trim().is_empty() {
                            lines.push(OfficeLine {
                                text: clip_line(line.trim()),
                                kind: OfficeLineKind::Table,
                            });
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    (lines, truncated)
}

/// `"AB12"` → 27. Zero-based; digits, `$` and the sheet prefix are ignored.
fn column_index(reference: &str) -> Option<usize> {
    let letters: String = reference
        .chars()
        .skip_while(|character| !character.is_ascii_alphabetic())
        .take_while(|character| character.is_ascii_alphabetic())
        .collect();
    if letters.is_empty() {
        return None;
    }
    let mut index = 0usize;
    for character in letters.chars() {
        index = index * 26 + (character.to_ascii_uppercase() as u8 - b'A' + 1) as usize;
    }
    Some(index - 1)
}

// ---------------------------------------------------------------------------
// PowerPoint
// ---------------------------------------------------------------------------

fn read_deck(
    archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
) -> Result<(Vec<OfficeSection>, bool), FileSystemError> {
    let presentation = read_part(archive, "ppt/presentation.xml")
        .ok_or_else(|| FileSystemError::InvalidInput("fs.office_unreadable".to_owned()))?;
    let relationships = read_part(archive, "ppt/_rels/presentation.xml.rels")
        .map(|xml| parse_relationships(&xml))
        .unwrap_or_default();

    let mut truncated = false;
    let mut sections = Vec::new();
    for relationship in parse_slide_order(&presentation) {
        if sections.len() >= MAX_SLIDES {
            truncated = true;
            break;
        }
        let Some(target) = relationships.get(&relationship) else {
            continue;
        };
        let part = resolve_part("ppt/", target);
        let Some(xml) = read_part(archive, &part) else {
            continue;
        };
        let (lines, clipped) = parse_slide(&xml);
        truncated |= clipped;
        if lines.is_empty() {
            continue;
        }
        sections.push(OfficeSection {
            heading: String::new(),
            lines,
        });
    }
    if sections.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "fs.office_unreadable".to_owned(),
        ));
    }
    Ok((sections, truncated))
}

/// Slide order is the `sldIdLst`, not the file name: inserting a slide renames
/// the parts, and sorting `slide10.xml` before `slide2.xml` would lie.
fn parse_slide_order(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut order = Vec::new();
    let mut in_list = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                match element.local_name().as_ref() {
                    "sldIdLst" => in_list = true,
                    "sldId" if in_list => {
                        if let Some(id) = relationship_id(&element) {
                            order.push(id);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(element)) => {
                if element.local_name().as_ref() == "sldIdLst" {
                    in_list = false;
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    order
}

fn parse_slide(xml: &str) -> (Vec<OfficeLine>, bool) {
    let mut reader = Reader::from_str(xml);
    let mut lines = Vec::new();
    let mut truncated = false;
    let mut paragraph = String::new();
    let mut in_paragraph = false;

    loop {
        if lines.len() >= MAX_TABLE_ROWS {
            truncated = true;
            break;
        }
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                if element.local_name().as_ref() == "p" {
                    in_paragraph = true;
                    paragraph.clear();
                }
            }
            Ok(Event::Empty(element)) => {
                if matches!(element.local_name().as_ref(), "br" | "tab") && in_paragraph {
                    paragraph.push(' ');
                }
            }
            Ok(Event::Text(text)) => {
                if in_paragraph {
                    paragraph.push_str(&text);
                }
            }
            Ok(Event::CData(text)) => {
                if in_paragraph {
                    paragraph.push_str(&text.xml10_content());
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if in_paragraph {
                    push_reference(&mut paragraph, &reference);
                }
            }
            Ok(Event::End(element)) => {
                if element.local_name().as_ref() == "p" && in_paragraph {
                    in_paragraph = false;
                    let text = std::mem::take(&mut paragraph);
                    if !text.trim().is_empty() {
                        // The first paragraph on a slide is its title in every
                        // layout PowerPoint ships; a placeholder check would
                        // need the slide layout part for no better answer.
                        let kind = if lines.is_empty() {
                            OfficeLineKind::Heading
                        } else {
                            OfficeLineKind::Text
                        };
                        lines.push(OfficeLine {
                            text: clamp_line(&text),
                            kind,
                        });
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    (lines, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn archive_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            for (name, contents) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .expect("start zip entry");
                writer
                    .write_all(contents.as_bytes())
                    .expect("write zip entry");
            }
            writer.finish().expect("finish zip");
        }
        buffer.into_inner()
    }

    fn write_fixture(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "dae-office-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&directory).expect("create fixture directory");
        let path = directory.join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    #[test]
    fn classifies_office_families_and_the_formats_without_a_preview() {
        assert_eq!(office_kind("report.docx"), Some(OfficeKind::Word));
        assert_eq!(office_kind("budget.XLSM"), Some(OfficeKind::Workbook));
        assert_eq!(office_kind("deck.pptx"), Some(OfficeKind::Slides));
        assert_eq!(office_kind("notes.md"), None);
        assert!(is_legacy_office_path("old.doc"));
        assert!(is_legacy_office_path("old.XLS"));
        assert!(is_unsupported_office_path("letter.odt"));
        assert!(!is_unsupported_office_path("report.docx"));
    }

    #[test]
    fn word_paragraphs_keep_headings_tables_and_entities() {
        let xml = r#"<?xml version="1.0"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
                     <w:r><w:t>Quarterly report</w:t></w:r></w:p>
                <w:p><w:r><w:t>Revenue grew </w:t></w:r>
                     <w:r><w:t>12% &amp; costs fell</w:t></w:r></w:p>
                <w:tbl>
                  <w:tr>
                    <w:tc><w:p><w:r><w:t>Region</w:t></w:r></w:p></w:tc>
                    <w:tc><w:p><w:r><w:t>Units</w:t></w:r></w:p></w:tc>
                  </w:tr>
                  <w:tr>
                    <w:tc><w:p><w:r><w:t>North</w:t></w:r></w:p></w:tc>
                    <w:tc><w:p><w:r><w:t>42</w:t></w:r></w:p></w:tc>
                  </w:tr>
                </w:tbl>
              </w:body>
            </w:document>"#;

        let (lines, truncated) = parse_document(xml);
        assert!(!truncated);
        assert_eq!(lines[0].text, "Quarterly report");
        assert_eq!(lines[0].kind, OfficeLineKind::Heading);
        assert_eq!(lines[1].text, "Revenue grew 12% & costs fell");
        assert_eq!(lines[1].kind, OfficeLineKind::Text);
        assert_eq!(lines[2].text, "Region | Units");
        assert_eq!(lines[2].kind, OfficeLineKind::Table);
        assert_eq!(lines[3].text, "North | 42");
    }

    #[test]
    fn worksheets_resolve_shared_strings_and_keep_column_positions() {
        let shared = parse_shared_strings(
            r#"<sst xmlns="x"><si><t>Month</t></si><si><t>Total</t></si></sst>"#,
        );
        assert_eq!(shared, vec!["Month", "Total"]);

        let sheet = r#"<worksheet xmlns="x"><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>Jan</t></is></c><c r="C2"><v>7</v></c></row>
        </sheetData></worksheet>"#;
        let (lines, truncated) = parse_worksheet(sheet, &shared);
        assert!(!truncated);
        assert_eq!(lines[0].text, "Month | Total");
        assert_eq!(lines[1].text, "Jan |  | 7");
    }

    #[test]
    fn slides_follow_the_presentation_order_not_the_file_name() {
        let presentation = r#"<p:presentation xmlns:p="x">
            <p:sldIdLst>
              <p:sldId id="256" r:id="rId3"/>
              <p:sldId id="257" r:id="rId2"/>
            </p:sldIdLst>
        </p:presentation>"#;
        assert_eq!(parse_slide_order(presentation), vec!["rId3", "rId2"]);

        let relationships = parse_relationships(
            r#"<Relationships xmlns="x">
                 <Relationship Id="rId2" Target="slides/slide2.xml"/>
                 <Relationship Id="rId3" Target="slides/slide10.xml"/>
               </Relationships>"#,
        );
        assert_eq!(
            relationships.get("rId3").map(String::as_str),
            Some("slides/slide10.xml")
        );
        assert_eq!(resolve_part("ppt/", "slides/slide10.xml"), "ppt/slides/slide10.xml");
        assert_eq!(resolve_part("xl/", "/xl/worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
    }

    #[test]
    fn a_slide_titles_itself_with_its_first_paragraph() {
        let slide = r#"<p:sld xmlns:a="x"><p:cSld><p:spTree><p:sp>
            <p:txBody>
              <a:p><a:r><a:t>Roadmap</a:t></a:r></a:p>
              <a:p><a:r><a:t>Ship the preview</a:t></a:r></a:p>
              <a:p><a:r><a:t>Measure it</a:t></a:r></a:p>
            </p:txBody>
        </p:sp></p:spTree></p:cSld></p:sld>"#;
        let (lines, _) = parse_slide(slide);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, OfficeLineKind::Heading);
        assert_eq!(lines[0].text, "Roadmap");
        assert_eq!(lines[2].text, "Measure it");
    }

    #[test]
    fn the_column_letters_become_a_zero_based_index() {
        assert_eq!(column_index("A1"), Some(0));
        assert_eq!(column_index("B12"), Some(1));
        assert_eq!(column_index("Z1"), Some(25));
        assert_eq!(column_index("AB3"), Some(27));
        assert_eq!(column_index("3"), None);
    }

    #[tokio::test]
    async fn reads_a_docx_container_end_to_end() {
        let bytes = archive_bytes(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="x"><w:body>
                 <w:p><w:r><w:t>Hello from Word</w:t></w:r></w:p>
                 <w:p><w:r><w:t>Second paragraph</w:t></w:r></w:p>
               </w:body></w:document>"#,
        )]);
        let path = write_fixture("report.docx", &bytes);

        let preview = read_office_preview(path.to_string_lossy().into_owned(), 1024 * 1024)
            .await
            .expect("read docx");
        assert_eq!(preview.kind, OfficeKind::Word);
        assert_eq!(preview.sections.len(), 1);
        assert_eq!(preview.sections[0].lines[0].text, "Hello from Word");
        assert_eq!(preview.sections[0].lines[1].text, "Second paragraph");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_zip_is_reported_as_unreadable() {
        let path = write_fixture("broken.docx", b"this is not a container");

        let error = read_office_preview(path.to_string_lossy().into_owned(), 1024 * 1024)
            .await
            .expect_err("plain text is not a docx");
        assert!(error.to_string().contains("fs.office_unreadable"));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn an_oversized_container_is_refused_before_it_is_read() {
        let bytes = archive_bytes(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body></w:document>"#,
        )]);
        let path = write_fixture("big.docx", &bytes);

        let error = read_office_preview(path.to_string_lossy().into_owned(), 8)
            .await
            .expect_err("the container is larger than the limit");
        assert!(error.to_string().contains("fs.office_too_large"));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
