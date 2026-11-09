//! HTML for the preview pane's Office section.
//!
//! `dae_core::fs::office` hands back sections of display-ready lines; this
//! module puts them in the subset `QTextDocument` renders. It is a pure
//! function of the extracted document and the theme, so the pane can re-render
//! after a light/dark switch without opening the container again — the same
//! contract `preview_markup::render` has.

use dae_core::fs::office::{OfficeLineKind, OfficePreview};

use crate::preview_markup::{Palette, push_escaped};

/// Renders every section. Headings only become their own paragraph when the
/// core supplied a label (a worksheet name); a document's headings arrive as
/// `Heading` lines and keep their position in the flow.
pub fn render(preview: &OfficePreview, dark: bool) -> String {
    let palette = Palette::new(dark);
    let mut out = String::with_capacity(4096);
    out.push_str("<div style=\"line-height:150%;\">");

    for (index, section) in preview.sections.iter().enumerate() {
        if !section.heading.is_empty() {
            if index > 0 {
                out.push_str("<div style=\"height:6px;\"></div>");
            }
            out.push_str(
                "<p style=\"font-size:10px; font-weight:600; letter-spacing:0.5px; color:",
            );
            out.push_str(palette.muted);
            out.push_str("; margin:0 0 5px;\">");
            push_escaped(&mut out, &section.heading);
            out.push_str("</p>");
        }

        for line in &section.lines {
            match line.kind {
                OfficeLineKind::Heading => {
                    out.push_str("<p style=\"font-size:13px; font-weight:600; color:");
                    out.push_str(palette.text);
                    out.push_str("; margin:9px 0 4px;\">");
                }
                OfficeLineKind::Text => {
                    out.push_str("<p style=\"font-size:11px; color:");
                    out.push_str(palette.text);
                    out.push_str("; margin:0 0 6px;\">");
                }
                OfficeLineKind::Table => {
                    // A sheet row is aligned by its ` | ` separators, so the
                    // mono face and preserved spaces are what make it readable.
                    out.push_str(
                        "<p style=\"font-family:'Cascadia Mono'; font-size:10px; color:",
                    );
                    out.push_str(palette.muted);
                    out.push_str("; white-space:pre-wrap; margin:0 0 2px;\">");
                }
            }
            push_escaped(&mut out, &line.text);
            out.push_str("</p>");
        }
    }

    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dae_core::fs::office::{OfficeKind, OfficeLine, OfficeSection};

    fn preview() -> OfficePreview {
        OfficePreview {
            kind: OfficeKind::Word,
            sections: vec![OfficeSection {
                heading: String::new(),
                lines: vec![
                    OfficeLine {
                        text: "Quarterly report".to_owned(),
                        kind: OfficeLineKind::Heading,
                    },
                    OfficeLine {
                        text: "Revenue <script> grew".to_owned(),
                        kind: OfficeLineKind::Text,
                    },
                    OfficeLine {
                        text: "Region | Units".to_owned(),
                        kind: OfficeLineKind::Table,
                    },
                ],
            }],
            truncated: false,
        }
    }

    #[test]
    fn headings_body_and_tables_keep_their_own_treatment() {
        let html = render(&preview(), true);
        assert!(html.contains("font-size:13px"));
        assert!(html.contains("Quarterly report"));
        assert!(html.contains("font-family:'Cascadia Mono'"));
        assert!(html.contains("Region | Units"));
    }

    #[test]
    fn document_text_is_escaped_not_rendered_as_markup() {
        let html = render(&preview(), true);
        assert!(!html.contains("<script>"));
        assert!(html.contains("Revenue &lt;script&gt; grew"));
    }

    #[test]
    fn a_sheet_name_is_its_own_label() {
        let mut preview = preview();
        preview.sections[0].heading = "Sales".to_owned();
        let html = render(&preview, false);
        assert!(html.contains("Sales"));
        assert!(html.contains("letter-spacing:0.5px"));
    }
}
