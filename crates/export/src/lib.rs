//! pdfcraft-export — Export a PDF ▸ Word, HTML, RTF, Markdown (L4).
//!
//! The engine reduces each page to [`Page`]: paragraphs (text, box, size, bold/italic) and
//! images (encoded bytes and box), in reading order. The writers turn that into a flowing
//! document: headings are the paragraphs set larger than the body text; a block that
//! continues another on its line (an italic word, a link) is a run of that block's paragraph, and
//! a paragraph that wraps onto the next line runs on there. Word keeps pages with
//! rotated text, or text overlapped by artwork or watermark-sized text, as positioned, editable
//! shapes to avoid reflowing overlays; other pages flow with images inline.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod lattice;
mod pptx;
mod xlsx;
mod zip;

pub use pptx::pptx;
pub use xlsx::xlsx;
pub use zip::Zip;

mod as_docx;
mod as_html;
mod as_markdown;
mod as_rtf;
mod clip;
mod flow;
mod layout;

pub(crate) use as_docx::hex;
#[cfg(test)]
use as_docx::{MAX_MEDIA, docx_table};
pub use as_docx::{docx, docx_with_language};
pub(crate) use as_html::allowed;
#[cfg(test)]
use as_html::base64;
pub(crate) use as_html::esc;
pub use as_html::{html, html_simple, html_simple_with_language, html_with_language};
pub use as_markdown::markdown;
#[cfg(test)]
use as_markdown::{emphasis, paragraph};
pub use as_rtf::rtf;
pub use clip::{ClipBuilder, ClipSpan, ClipStyle, ClipText, MAX_CLIP_HTML_BYTES, MAX_CLIP_SPANS, MAX_CLIP_TEXT_BYTES, clip_html, font_family};
#[cfg(test)]
use flow::list_item;
#[cfg(test)]
use layout::tables;
pub use layout::{Block, Cell, Image, Page, Table};
pub(crate) use layout::{Item, MAX_COLS, Run, flowing_paragraphs, items, page_tables, reading_order};
#[cfg(test)]
use layout::{LOOKUP_STEPS, body_size};

#[cfg(test)]
mod tests;

/// Test helper: reads back zip archives written by [`Zip`].
#[cfg(test)]
pub(crate) mod tests_support {
    use std::io::Read;

    /// The files of a zip archive written by [`crate::Zip`]: (name, contents as text).
    pub fn unzip(bytes: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut at = 0usize;
        let u16_at = |i: usize| bytes.get(i..i + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize);
        let u32_at = |i: usize| bytes.get(i..i + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        while bytes.get(at..at + 4) == Some(b"PK\x03\x04".as_slice()) {
            let method = u16_at(at + 8);
            let size = u32_at(at + 18);
            let name_len = u16_at(at + 26);
            let extra = u16_at(at + 28);
            let name = String::from_utf8_lossy(&bytes[at + 30..at + 30 + name_len]).into_owned();
            let start = at + 30 + name_len + extra;
            let data = &bytes[start..start + size];
            let text = if method == 8 {
                let mut s = String::new();
                let _ = flate2::read::DeflateDecoder::new(data).read_to_string(&mut s);
                s
            } else {
                String::from_utf8_lossy(data).into_owned()
            };
            out.push((name, text));
            at = start + size;
        }
        out
    }
}
