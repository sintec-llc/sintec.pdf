//! Sintec.PDF: installed faces for text the PDF's own fonts can't draw.
//!
//! PDFs usually embed only the glyphs their text uses, sometimes without any character map, or
//! no program at all, and often under made-up names (`CIDFont+F1` for Calibri Bold). Edit PDF
//! therefore recognises each font from its embedded program ([`PdfCraftApp::page_fonts`]) and
//! finds its best installed stand-in. Before a text edit is applied,
//! [`PdfCraftApp::prepare_text_edit`]:
//!
//! - draws Bold, Italic and family changes in the matching installed face of the paragraph's
//!   real family (Calibri Bold for a bold Calibri paragraph), not in the standard PDF fonts,
//!   which have no Cyrillic;
//! - draws characters the paragraph's fonts can't encode in the same family installed, or its
//!   metric-compatible stand-in (Carlito for Calibri), in the font's detected weight and slant,
//!   silently; the edit fits that face to the font's cap height and widths and strokes a regular
//!   face standing in for a bold font (see `pdfcraft_edit`'s text rewrite);
//! - shows those characters in the same face while they're typed ([`PdfCraftApp::preview_substitutes`]).
//!
//! When no similar face is installed, the Font substitution dialog suggests the closest ones
//! and the edit waits for the choice, which is remembered per document and font. Edit PDF also
//! suggests changing a font with no similar installed face when it is first shown.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, Layout};
use pdfcraft_engine::text_fonts::{FontFile, Match, Wanted, family_match, rank};
use pdfcraft_engine::{DocId, Edit};

use crate::{PdfCraftApp, theme, widgets};

/// What the user chose for a font, per document and font family and style.
#[derive(Clone, Debug, PartialEq)]
pub enum FontChoice {
    /// Load this installed face; with `whole`, set the whole paragraph in it.
    Use { path: PathBuf, whole: bool },
    /// Keep the document's font (characters it can't show fall back to Sintec.PDF's own faces).
    Keep,
}

/// The Font substitution dialog.
#[derive(Clone, Debug)]
pub struct FontOffer {
    pub doc: DocId,
    pub page: usize,
    /// The memory's key (family and style) and the font's name as recognised ("Calibri").
    pub key: String,
    pub base_font: String,
    pub wanted: Wanted,
    /// Why it's asked: not installed, or the document lacks glyphs; whether the document
    /// carries any of the font.
    pub installed: bool,
    pub embedded: bool,
    pub choice: Option<PathBuf>,
    pub whole: bool,
    /// The edit waiting for the choice.
    pub pending: Option<Edit>,
    /// Opened from Format text for the paragraph being edited: the choice becomes its font.
    /// Otherwise (a suggestion from Edit PDF's analysis) it is only remembered.
    pub paragraph: bool,
}

/// Sintec.PDF: one font of a page as Edit PDF sees it: what it really is, and the installed face
/// that stands in for it best.
#[derive(Clone, Debug)]
pub struct FontReport {
    pub resource: String,
    pub facts: pdfcraft_engine::FontFacts,
    pub best: Option<FontFile>,
    pub quality: Match,
}

impl FontReport {
    /// "Calibri Bold": the font as it really is.
    pub fn name(&self) -> String {
        styled_name(self.facts.family(), self.facts.is_bold(), self.facts.is_italic())
    }
}

fn styled_name(family: &str, bold: bool, italic: bool) -> String {
    let mut s = family.to_string();
    if bold && !s.to_ascii_lowercase().contains("bold") {
        s.push_str(" Bold");
    }
    if italic && !s.to_ascii_lowercase().contains("italic") {
        s.push_str(" Italic");
    }
    s
}

/// Is this font one of the app's own faces (`pdfcraft_fonts::CRAFT_FONTS`), by its program's
/// family or its `/BaseFont`? Always false in a build without craft-fonts.
fn own_face(facts: &pdfcraft_engine::FontFacts) -> bool {
    // A name with no ASCII letters or digits has an empty key, which says nothing.
    [facts.family(), facts.base_font.as_str()]
        .into_iter()
        .filter(|name| !pdfcraft_fonts::embedding::face_key(name).is_empty())
        .any(|name| !pdfcraft_fonts::embedding::matching_faces(name).is_empty())
}

/// The memory's key for a font in a style.
fn key_of(wanted: &Wanted) -> String {
    format!(
        "{}{}{}",
        pdfcraft_engine::text_fonts::family_key(&wanted.base_font),
        if wanted.bold { "-b" } else { "" },
        if wanted.italic { "-i" } else { "" }
    )
}

/// Choices, the analysis and the open dialog.
#[derive(Default)]
pub struct FontSubstitutes {
    pub choices: HashMap<(DocId, String), FontChoice>,
    pub offer: Option<FontOffer>,
    /// The edit being applied came from the dialog: don't check it again.
    pub(crate) bypass: bool,
    /// Edit PDF's font analysis per document and page, with the document version it is for.
    pub reports: HashMap<(DocId, usize), (u64, Vec<FontReport>)>,
    /// Fonts already suggested for changing (per document and key), so it's asked once.
    pub suggested: std::collections::HashSet<(DocId, String)>,
    /// The installed faces to use instead of scanning the system's font folders (embedders and
    /// tests that need a known set).
    pub installed_override: Option<Vec<FontFile>>,
    /// Installed faces read for the inline editor's preview, by path.
    preview_bytes: HashMap<PathBuf, Arc<Vec<u8>>>,
}

/// Sintec.PDF: the face that draws what a source font lacks, and why: what the user chose, or the
/// installed face closest to the font's detected family, weight and slant.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stand {
    pub face: FontFile,
    pub quality: Match,
    /// Chosen by the user (Font substitution dialog), not found.
    pub chosen: bool,
    /// The font is bold and the face isn't: the edit strokes it (synthetic bold).
    pub synthetic_bold: bool,
}

/// What to do with a text edit.
#[expect(clippy::large_enum_variant, reason = "short-lived: returned once per text edit and unpacked at once")]
pub(crate) enum Prepared {
    Apply(Edit),
    /// The dialog is open and holds the edit.
    Wait,
}

/// The installed faces, once the background scan has finished (`None` while it runs).
fn installed(ctx: Option<&egui::Context>) -> Option<Vec<FontFile>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        ctx.and_then(crate::files::installed_font_files)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        Some(Vec::new())
    }
}

impl PdfCraftApp {
    /// The installed faces, once the background scan has finished (see [`installed`]).
    pub(crate) fn installed_faces(&self) -> Option<Vec<FontFile>> {
        self.fonts.installed_override.clone().or_else(|| installed(self.ctx.as_ref()))
    }

    /// The font facts and the font wanted for paragraph `block` of page `page` with `text`.
    fn wanted_for(&self, id: DocId, page: usize, block: usize, text: &str) -> Option<(pdfcraft_engine::FontFacts, Wanted)> {
        let doc = self.session.get(id)?;
        let b = doc.text_blocks(page).into_iter().nth(block)?;
        let lines = doc.text_lines(page);
        let line = b.lines.first().and_then(|i| lines.get(*i))?;
        let facts = doc.font_facts(page, &line.font)?;
        let wanted = Wanted::from_facts(&facts, Some(facts.is_bold() || b.bold), Some(facts.is_italic() || b.italic), text);
        Some((facts, wanted))
    }

    /// Edit PDF's analysis of page `page`'s fonts: each font its text uses, what it really is and
    /// its best installed stand-in. Cached per document version; empty until the installed
    /// fonts are known.
    pub(crate) fn page_fonts(&mut self, id: DocId, page: usize) -> Vec<FontReport> {
        let Some(doc) = self.session.get(id) else { return Vec::new() };
        let generation = doc.edit_generation();
        if let Some((g, reports)) = self.fonts.reports.get(&(id, page))
            && *g == generation
        {
            return reports.clone();
        }
        let Some(fonts) = self.installed_faces() else { return Vec::new() };
        let mut seen = Vec::new();
        let mut reports = Vec::new();
        for line in doc.text_lines(page) {
            if seen.contains(&line.font) || line.font.starts_with("PCFull") || seen.len() >= 64 {
                continue;
            }
            seen.push(line.font.clone());
            let Some(facts) = doc.font_facts(page, &line.font) else { continue };
            let wanted = Wanted::from_facts(&facts, Some(facts.is_bold() || line.bold), Some(facts.is_italic() || line.italic), "");
            let best = rank(&wanted, &fonts).first().map(|f| (*f).clone());
            let quality = best.as_ref().map_or(Match::Other, |f| family_match(&wanted, f));
            reports.push(FontReport { resource: line.font.clone(), facts, best, quality });
        }
        self.fonts.reports.insert((id, page), (generation, reports.clone()));
        reports
    }

    /// Edit PDF is showing page `page`: suggest changing, once, a font of it that has no
    /// similar installed face.
    pub(crate) fn suggest_fonts(&mut self, id: DocId, page: usize) {
        if self.fonts.offer.is_some() {
            return;
        }
        for report in self.page_fonts(id, page) {
            let wanted = Wanted::from_facts(&report.facts, None, None, "");
            let key = key_of(&wanted);
            // A face the app ships itself (the craft-fonts Arabic, Hebrew or Japanese faces it
            // writes added and replacement text in) needs no installed stand-in.
            if report.quality != Match::Other
                || own_face(&report.facts)
                || self.fonts.choices.contains_key(&(id, key.clone()))
                || !self.fonts.suggested.insert((id, key))
            {
                continue;
            }
            let choice = report.best.as_ref().map(|f| f.path.clone());
            self.open_offer(id, page, &report.facts, wanted, choice, true, None, false);
            return;
        }
    }

    /// Sintec.PDF: the face that draws the characters font `facts` lacks in text `text` of
    /// document `id`: the face chosen for it, else the closest installed one. `None` while the
    /// installed faces are being found, when the user chose to keep the document's font, or when
    /// no face can show the text.
    pub(crate) fn stand_in(&self, id: DocId, facts: &pdfcraft_engine::FontFacts, text: &str) -> Option<Stand> {
        let wanted = Wanted::from_facts(facts, None, None, text);
        let fonts = self.installed_faces()?;
        let synthetic = |f: &FontFile| wanted.bold && f.style.weight < 600;
        match self.fonts.choices.get(&(id, key_of(&wanted))) {
            Some(FontChoice::Keep) => return None,
            Some(FontChoice::Use { path, .. }) => {
                if let Some(face) = fonts.iter().find(|f| &f.path == path) {
                    let quality = family_match(&wanted, face);
                    return Some(Stand { synthetic_bold: synthetic(face), face: face.clone(), quality, chosen: true });
                }
            }
            None => {}
        }
        let face = rank(&wanted, &fonts).first().map(|f| (*f).clone())?;
        let quality = family_match(&wanted, &face);
        Some(Stand { synthetic_bold: synthetic(&face), face, quality, chosen: false })
    }

    /// Sintec.PDF: while a paragraph is open in the inline editor, put each source font's stand-in
    /// ([`PdfCraftApp::stand_in`]) behind its embedded subset in the editor's fonts, so characters
    /// the subset lacks show in that face as they're typed (not in the interface's own face). Once
    /// per document version; waits for the installed faces.
    ///
    /// Sintec.PDF (task 033): every font of the paragraph gets its stand-in, including one the
    /// document doesn't embed (which has no program for the editor: its text is shown in the
    /// stand-in, not in the interface's face). The stand-in is sized by the same
    /// [`pdfcraft_engine::substitute_fit`] the edit applies, and the characters the edit will draw
    /// in it ([`pdfcraft_engine::Document::substitute_chars`]) advance as far as they will.
    pub(crate) fn preview_substitutes(&mut self, ctx: &egui::Context) {
        for i in 0..self.views.len() {
            let Some((id, page, text, resources)) = self.views.get(i).and_then(|v| {
                let ed = v.line_editor.as_ref()?;
                let generation = self.session.get(v.id)?.edit_generation();
                (ed.substitutes_for != Some(generation)).then(|| (v.id, ed.page, ed.text.clone(), ed.preview_resources()))
            }) else {
                continue;
            };
            if self.installed_faces().is_none() {
                break;
            }
            let mut substitutes = std::collections::BTreeMap::new();
            let mut budget: usize = 64 << 20;
            for resource in resources.iter().take(16) {
                let Some(facts) = self.session.get(id).and_then(|d| d.font_facts(page, resource)) else { continue };
                let Some(stand) = self.stand_in(id, &facts, &text) else { continue };
                let italic = stand.face.style.italic;
                let path = stand.face.path;
                let face_path = path.clone();
                let bytes = match self.fonts.preview_bytes.get(&path) {
                    Some(b) => b.clone(),
                    None => {
                        let Ok(meta) = std::fs::metadata(&path) else { continue };
                        if meta.len() > 16 << 20 {
                            continue;
                        }
                        let Ok(b) = std::fs::read(&path) else { continue };
                        let b = Arc::new(b);
                        if self.fonts.preview_bytes.len() < 32 {
                            self.fonts.preview_bytes.insert(path.clone(), b.clone());
                        }
                        b
                    }
                };
                if bytes.len() > budget {
                    continue;
                }
                budget -= bytes.len();
                let fit = pdfcraft_engine::substitute_fit(&facts, &bytes);
                substitutes.insert(resource.clone(), crate::edit_text_ui::SubstituteFace { path: face_path, bytes, fit, italic });
            }
            let Some(doc) = self.session.get(id) else { continue };
            let generation = doc.edit_generation();
            let (fonts, previews) = crate::edit_text_ui::install_source_fonts_with(ctx, doc, page, &substitutes);
            if let Some(ed) = self.views.get_mut(i).and_then(|v| v.line_editor.as_mut()) {
                ed.fonts = fonts;
                ed.substitutes = previews;
                ed.substitutes_for = Some(generation);
            }
        }
        self.classify_typed(ctx);
    }

    /// Sintec.PDF (task 033): which of the characters typed into an open paragraph the edit will
    /// draw in each font's stand-in, asked of the engine once per character and font.
    fn classify_typed(&mut self, ctx: &egui::Context) {
        for i in 0..self.views.len() {
            let Some(id) = self.views.get(i).map(|v| v.id) else { continue };
            let Some(doc) = self.session.get(id) else { continue };
            let Some(ed) = self.views.get_mut(i).and_then(|v| v.line_editor.as_mut()) else { continue };
            let page = ed.page;
            let (mut asked, mut drawn_new) = (false, false);
            for (resource, sub) in &mut ed.substitutes {
                let mut new: Vec<char> = ed.text.chars().filter(|c| *c != '\n' && !sub.chars.contains_key(c)).collect();
                new.sort_unstable();
                new.dedup();
                // A bounded memory: past it, characters show through the font stack unmeasured.
                new.truncate(crate::edit_text_ui::MAX_CLASSIFIED.saturating_sub(sub.chars.len()));
                if new.is_empty() {
                    continue;
                }
                let sample: String = new.iter().collect();
                let drawn = doc.substitute_chars(page, resource, &sample);
                for c in new {
                    sub.chars.insert(c, drawn.contains(&c));
                }
                asked = true;
                drawn_new |= !drawn.is_empty();
            }
            if asked {
                ctx.request_repaint();
            }
            if !drawn_new {
                continue;
            }
            // The edit ranks the installed faces for the text as it is then: when the text now
            // needs letters the shown stand-in was not chosen for, find the stand-ins again.
            let text = ed.text.clone();
            let shown: Vec<(String, PathBuf)> = ed.substitutes.iter().map(|(r, s)| (r.clone(), s.path.clone())).collect();
            let stale = shown.iter().any(|(resource, path)| {
                self.session
                    .get(id)
                    .and_then(|d| d.font_facts(page, resource))
                    .and_then(|facts| self.stand_in(id, &facts, &text))
                    .is_some_and(|stand| stand.face.path != *path)
            });
            if stale && let Some(ed) = self.views.get_mut(i).and_then(|v| v.line_editor.as_mut()) {
                ed.substitutes_for = None;
            }
        }
    }

    /// The installed face closest to `wanted`, and how well it matches.
    fn styled_face(&self, wanted: &Wanted) -> Option<(FontFile, Match)> {
        let fonts = self.installed_faces()?;
        let best = rank(wanted, &fonts).first().map(|f| (*f).clone())?;
        let quality = family_match(wanted, &best);
        Some((best, quality))
    }

    /// Before a text edit is applied: load the faces it needs, or ask for one (see the module
    /// notes).
    pub(crate) fn prepare_text_edit(&mut self, id: DocId, edit: Edit) -> Prepared {
        if std::mem::take(&mut self.fonts.bypass) {
            return Prepared::Apply(edit);
        }
        let Edit::EditTextBlock { page, block, text, mut style } = edit else { return Prepared::Apply(edit) };
        let rebuild = |style| Edit::EditTextBlock { page, block, text: text.clone(), style };
        let Some((facts, wanted)) = self.wanted_for(id, page, block, &text) else { return Prepared::Apply(rebuild(style)) };
        let source_family = crate::edit_text_ui::family_of(facts.family());
        // What a family/bold/italic request means: the paragraph's own family in that style, or
        // the installed stand-in of the standard family chosen instead.
        let want_for = |fam: pdfcraft_engine::FontFamily, bold: bool, italic: bool| -> Wanted {
            let base = if fam == source_family {
                facts.family().to_string()
            } else {
                match fam {
                    pdfcraft_engine::FontFamily::Helvetica => "Arial",
                    pdfcraft_engine::FontFamily::Times => "Times New Roman",
                    pdfcraft_engine::FontFamily::Courier => "Courier New",
                }
                .to_string()
            };
            Wanted {
                base_font: base,
                bold,
                italic,
                serif: None,
                monospace: fam == pdfcraft_engine::FontFamily::Courier,
                sample: text.clone(),
                widths: Vec::new(),
            }
        };
        if let Some((fam, bold, italic)) = style.family {
            let want = want_for(fam, bold, italic);
            match self.styled_face(&want) {
                Some((face, Match::Exact | Match::Fingerprint | Match::Alias)) => {
                    if let Some(resource) = self.load_substitute(id, page, &face.path) {
                        style.resource = Some(resource);
                        style.family = None;
                        style.bold = None;
                    }
                }
                Some((face, Match::Other)) => {
                    let key = key_of(&want);
                    if let Some(FontChoice::Use { path, .. }) = self.fonts.choices.get(&(id, key)).cloned() {
                        return self.with_substitute(id, page, &path, true, rebuild(style));
                    }
                    self.open_offer(id, page, &facts, want, Some(face.path), true, Some(rebuild(style)), false);
                    return Prepared::Wait;
                }
                None => {}
            }
        }
        let mut ranges = std::mem::take(&mut style.ranges);
        for r in &mut ranges {
            if let Some((fam, bold, italic)) = r.family
                && let Some((face, Match::Exact | Match::Fingerprint | Match::Alias)) = self.styled_face(&want_for(fam, bold, italic))
                && let Some(resource) = self.load_substitute(id, page, &face.path)
            {
                r.resource = Some(resource);
                r.family = None;
            }
        }
        style.ranges = ranges;
        if style.resource.is_some() {
            return Prepared::Apply(rebuild(style));
        }
        let _ = wanted;
        for font in self.short_fonts(id, page, block, &text) {
            let Some(facts) = self.session.get(id).and_then(|d| d.font_facts(page, &font)) else { continue };
            let wanted = Wanted::from_facts(&facts, None, None, &text);
            let key = key_of(&wanted);
            let choice = self.fonts.choices.get(&(id, key.clone())).cloned();
            match &choice {
                Some(FontChoice::Keep) => continue,
                Some(FontChoice::Use { path, whole: true }) => {
                    if let Some(resource) = self.load_substitute(id, page, path) {
                        style.resource = Some(resource);
                        break;
                    }
                    continue;
                }
                Some(FontChoice::Use { whole: false, .. }) | None => {}
            }
            // Sintec.PDF (task 033): the face the open editor previewed this font's missing
            // characters in ([`PdfCraftApp::stand_in`]), so the edit draws them in that face.
            let Some(stand) = self.stand_in(id, &facts, &text) else { continue };
            if stand.chosen || matches!(stand.quality, Match::Exact | Match::Fingerprint | Match::Alias) {
                // The same family is installed, or a metric-compatible stand-in for it (Carlito
                // for Calibri, Tinos for Times New Roman), in the closest weight and slant: it
                // draws the missing characters, fitted to the font (see the edit crate's `Fit`).
                if choice.is_none() {
                    self.fonts.choices.insert((id, key), FontChoice::Use { path: stand.face.path.clone(), whole: false });
                }
                if let Some(resource) = self.load_substitute(id, page, &stand.face.path) {
                    style.substitutes.push((font, resource));
                }
                continue;
            }
            self.open_offer(id, page, &facts, wanted, Some(stand.face.path), stand.quality == Match::Other, Some(rebuild(style)), false);
            return Prepared::Wait;
        }
        Prepared::Apply(rebuild(style))
    }

    /// The fonts of paragraph `block` of page `page` that can't encode `text`.
    fn short_fonts(&self, id: DocId, page: usize, block: usize, text: &str) -> Vec<String> {
        let Some(doc) = self.session.get(id) else { return Vec::new() };
        let lines = doc.text_lines(page);
        let mut v: Vec<String> = Vec::new();
        if let Some(b) = doc.text_blocks(page).into_iter().nth(block) {
            for l in b.lines.iter().filter_map(|i| lines.get(*i)) {
                if !v.contains(&l.font) && doc.first_unencodable(page, &l.font, text).is_some() {
                    v.push(l.font.clone());
                }
            }
        }
        v
    }

    /// Sintec.PDF (task 033): `edit` with each of its paragraph's fonts that lacks characters of
    /// the text mapped to its loaded stand-in ([`PdfCraftApp::stand_in`], which follows the
    /// choices made), as [`PdfCraftApp::prepare_text_edit`] maps them. Never asks.
    fn with_stand_ins(&mut self, id: DocId, edit: Edit) -> Edit {
        let Edit::EditTextBlock { page, block, text, mut style } = edit else { return edit };
        for font in self.short_fonts(id, page, block, &text) {
            if style.substitutes.iter().any(|(f, _)| *f == font) {
                continue;
            }
            let Some(facts) = self.session.get(id).and_then(|d| d.font_facts(page, &font)) else { continue };
            let Some(stand) = self.stand_in(id, &facts, &text) else { continue };
            if let Some(resource) = self.load_substitute(id, page, &stand.face.path) {
                style.substitutes.push((font, resource));
            }
        }
        Edit::EditTextBlock { page, block, text, style }
    }

    /// Open the Font substitution dialog for `facts`.
    #[expect(clippy::too_many_arguments, reason = "the dialog's whole state, filled in one place")]
    fn open_offer(
        &mut self,
        id: DocId,
        page: usize,
        facts: &pdfcraft_engine::FontFacts,
        wanted: Wanted,
        choice: Option<PathBuf>,
        whole: bool,
        pending: Option<Edit>,
        paragraph: bool,
    ) {
        let fonts = self.installed_faces().unwrap_or_default();
        let installed = rank(&wanted, &fonts).first().is_some_and(|f| matches!(family_match(&wanted, f), Match::Exact | Match::Fingerprint));
        self.fonts.offer = Some(FontOffer {
            doc: id,
            page,
            key: key_of(&wanted),
            base_font: styled_name(facts.family(), wanted.bold, wanted.italic),
            wanted,
            installed,
            embedded: facts.embedded,
            choice,
            whole,
            pending,
            paragraph,
        });
    }

    /// An edit failed for want of a glyph: offer a substitute (once per edit).
    pub(crate) fn offer_after_failure(&mut self, id: DocId, edit: &Edit, message: &str) -> bool {
        let Edit::EditTextBlock { page, block, text, .. } = edit else { return false };
        let glyphs = ["fallback font", "cannot show U+", "complete source TrueType", "can't draw"];
        if !glyphs.iter().any(|g| message.contains(g)) || self.fonts.offer.is_some() {
            return false;
        }
        let Some((facts, wanted)) = self.wanted_for(id, *page, *block, text) else { return false };
        let best = self.styled_face(&wanted);
        let whole = best.as_ref().is_none_or(|(_, q)| *q == Match::Other);
        self.open_offer(id, *page, &facts, wanted, best.map(|(f, _)| f.path), whole, Some(edit.clone()), false);
        true
    }

    /// Format text ▸ Replace font…: choose a face for the paragraph being edited.
    pub(crate) fn offer_for_paragraph(&mut self, id: DocId, page: usize, block: usize, text: &str) {
        let Some((facts, wanted)) = self.wanted_for(id, page, block, text) else { return };
        let best = self.styled_face(&wanted).map(|(f, _)| f.path);
        self.open_offer(id, page, &facts, wanted, best, true, None, true);
    }

    /// Load `path` into page `page` (once) and return its resource name.
    fn load_substitute(&mut self, id: DocId, page: usize, path: &PathBuf) -> Option<String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.notify_fmt("Couldn't read {name}: {e}", &[("name", &path.display().to_string()), ("e", &e.to_string())]);
                return None;
            }
        };
        let find = |app: &PdfCraftApp| {
            app.session
                .get(id)
                .and_then(|d| d.text_editor_fonts(page).into_iter().find(|f| f.bytes == bytes && f.resource.starts_with("PCFull")))
                .map(|f| f.resource)
        };
        if let Some(resource) = find(self) {
            return Some(resource);
        }
        if !self.apply_edit(Edit::RegisterTextFont { page, bytes: Arc::new(bytes.clone()) }) {
            return None;
        }
        find(self)
    }

    fn with_substitute(&mut self, id: DocId, page: usize, path: &PathBuf, whole: bool, edit: Edit) -> Prepared {
        let Some(resource) = self.load_substitute(id, page, path) else { return Prepared::Apply(edit) };
        match edit {
            Edit::EditTextBlock { page, block, text, mut style } if whole => {
                style.resource = Some(resource);
                style.family = None;
                style.bold = None;
                Prepared::Apply(Edit::EditTextBlock { page, block, text, style })
            }
            edit => Prepared::Apply(edit),
        }
    }
}

/// The Font substitution dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(mut offer) = app.fonts.offer.clone() else { return };
    let t = theme::Tokens::get(ctx);
    let fonts = installed(Some(ctx));
    let mut done: Option<Option<FontChoice>> = None;
    let modal = widgets::show_modal(ctx, "font-substitute", |ui| {
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("file-text", 22.0, t.accent));
            ui.label(egui::RichText::new(tl!("Font substitution")).font(theme::semibold(16.0)));
        });
        ui.add_space(8.0);
        let why = if offer.installed {
            tl!("The document carries only some characters of the font {font}, and the text needs others.")
        } else if !offer.embedded {
            tl!("The font {font} isn't installed on this computer and isn't embedded in the document.")
        } else {
            tl!("The font {font} isn't installed on this computer, and the document carries only some of its characters.")
        };
        ui.label(crate::i18n::fmt(why, &[("font", &format!("«{}»", offer.base_font))]));
        if offer.pending.is_none() && !offer.paragraph {
            ui.label(
                egui::RichText::new(tl!("No similar font is installed. Choose the font edited text in it is set in.")).color(t.text_muted).small(),
            );
        }
        ui.add_space(8.0);
        match &fonts {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tl!("Finding installed fonts…"));
                });
            }
            Some(fonts) => {
                let ranked = rank(&offer.wanted, fonts);
                if ranked.is_empty() {
                    ui.label(tl!("No installed font can show this text."));
                } else {
                    let selected = offer.choice.as_ref().and_then(|p| ranked.iter().find(|f| &f.path == p)).copied().or(ranked.first().copied());
                    ui.horizontal(|ui| {
                        let l = ui.label(tl!("Replace with"));
                        egui::ComboBox::from_id_salt("font-substitute-choice")
                            .width(300.0)
                            .selected_text(selected.map(label_of(&offer.wanted)).unwrap_or_default())
                            .show_ui(ui, |ui| {
                                for f in ranked.iter().take(60) {
                                    let picked = offer.choice.as_ref() == Some(&f.path);
                                    if ui.selectable_label(picked, label_of(&offer.wanted)(f)).clicked() {
                                        offer.choice = Some(f.path.clone());
                                    }
                                }
                            })
                            .response
                            .labelled_by(l.id);
                    });
                    if offer.choice.is_none() {
                        offer.choice = selected.map(|f| f.path.clone());
                    }
                }
            }
        }
        ui.add_space(6.0);
        if offer.pending.is_some() {
            ui.add(widgets::checkbox(&mut offer.whole, tl!("Set the whole paragraph in this font")));
            ui.label(egui::RichText::new(tl!("Otherwise only the characters the document lacks are drawn in it.")).color(t.text_muted).small());
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let can = offer.choice.is_some() && fonts.is_some();
            if ui.add_enabled_ui(can, |ui| widgets::pill_button(ui, tl!("Replace"), true)).inner.clicked()
                && let Some(path) = offer.choice.clone()
            {
                done = Some(Some(FontChoice::Use { path, whole: offer.whole }));
            }
            if widgets::pill_button(ui, tl!("Keep the original font"), false).clicked() {
                done = Some(Some(FontChoice::Keep));
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                done = Some(None);
            }
        });
    });
    if modal.should_close() && done.is_none() {
        done = Some(None);
    }
    match done {
        None => app.fonts.offer = Some(offer),
        Some(choice) => {
            app.fonts.offer = None;
            finish(app, offer, choice);
        }
    }
}

/// "Times New Roman Bold — same font" / "Arial — usual substitute".
fn label_of(want: &Wanted) -> impl Fn(&FontFile) -> String + '_ {
    move |f: &FontFile| match family_match(want, f) {
        Match::Exact => format!("{} — {}", f.display(), tl!("same font")),
        Match::Fingerprint => {
            let d = pdfcraft_engine::text_fonts::fingerprint_distance(want, f).unwrap_or_default();
            crate::i18n::fmt(tl!("{face} — same glyph widths (off by {d} on average)"), &[("face", &f.display()), ("d", &format!("{d:.1}"))])
        }
        Match::Alias => format!("{} — {}", f.display(), tl!("usual substitute")),
        Match::Other => listed_name(f),
    }
}

/// A face's name as listed: bundled ones say they come with the app.
pub(crate) fn listed_name(f: &FontFile) -> String {
    if f.bundled { format!("{} — {}", f.display(), tl!("included with Sintec.PDF")) } else { f.display() }
}

/// Apply the dialog's choice: remember it, load the face, then the edit that waited (or, from
/// Format text, make the face the paragraph's font).
fn finish(app: &mut PdfCraftApp, offer: FontOffer, choice: Option<FontChoice>) {
    let Some(choice) = choice else {
        // Cancelled: an edit that waited is dropped, so the paragraph stays as it was.
        if offer.pending.is_some() {
            app.notify(tl!("The text wasn't changed."));
        }
        return;
    };
    app.fonts.choices.insert((offer.doc, offer.key.clone()), choice.clone());
    match (choice, offer.pending) {
        (FontChoice::Keep, Some(edit)) => {
            app.fonts.bypass = true;
            app.apply_edit(edit);
        }
        (FontChoice::Keep, None) => {}
        (FontChoice::Use { path, whole }, Some(edit)) => {
            if let Prepared::Apply(edit) = app.with_substitute(offer.doc, offer.page, &path, whole, edit) {
                // Only the characters it lacks: in the face just chosen, as the editor shows them.
                let edit = if whole { edit } else { app.with_stand_ins(offer.doc, edit) };
                app.fonts.bypass = true;
                app.apply_edit(edit);
            }
        }
        // A suggestion from the analysis: remembered, used when text in the font is edited.
        (FontChoice::Use { .. }, None) if !offer.paragraph => {}
        (FontChoice::Use { path, .. }, None) => {
            let Some(resource) = app.load_substitute(offer.doc, offer.page, &path) else { return };
            if let Some((i, _)) = app.active_ids()
                && let Some(ed) = app.views[i].line_editor.as_mut()
            {
                ed.font_resource = Some(resource);
                if let (Some(doc), Some(ctx)) = (app.session.get(offer.doc), app.ctx.as_ref()) {
                    ed.reload_fonts(ctx, doc);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analysis_shows_the_width_fingerprint() {
        let widths: Vec<(char, u16)> = "ABCDEFGHIJ".chars().enumerate().map(|(i, c)| (c, 500 + 10 * i as u16)).collect();
        let face = FontFile {
            path: PathBuf::from("Mystery.ttf"),
            family: "Mystery Sans".into(),
            face: "MysterySans".into(),
            style: pdfcraft_fonts::embedding::FaceStyle { weight: 400, italic: false, monospace: false, serif: Some(false), cyrillic: true },
            widths,
            bundled: false,
        };
        let want = Wanted {
            base_font: "CIDFont+F1".into(),
            widths: "ABCDEFGHIJ".chars().enumerate().map(|(i, c)| (c, 503.0 + 10.0 * i as f32)).collect(),
            ..Wanted::default()
        };
        let label = label_of(&want)(&face);
        assert!(label.contains("Mystery Sans") && label.contains("3.0"), "{label}");
    }
}
