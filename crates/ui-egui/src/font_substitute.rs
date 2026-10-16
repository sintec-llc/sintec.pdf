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
//! - draws characters the paragraph's fonts can't encode in the same family installed, silently.
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
    fn installed_faces(&self) -> Option<Vec<FontFile>> {
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
            if report.quality != Match::Other || self.fonts.choices.contains_key(&(id, key.clone())) || !self.fonts.suggested.insert((id, key)) {
                continue;
            }
            let choice = report.best.as_ref().map(|f| f.path.clone());
            self.open_offer(id, page, &report.facts, wanted, choice, true, None, false);
            return;
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
        // The paragraph's fonts that can't encode the new text.
        let short: Vec<String> = match self.session.get(id) {
            Some(doc) => {
                let lines = doc.text_lines(page);
                let mut v: Vec<String> = Vec::new();
                if let Some(b) = doc.text_blocks(page).into_iter().nth(block) {
                    for l in b.lines.iter().filter_map(|i| lines.get(*i)) {
                        if !v.contains(&l.font) && doc.first_unencodable(page, &l.font, &text).is_some() {
                            v.push(l.font.clone());
                        }
                    }
                }
                v
            }
            None => Vec::new(),
        };
        let _ = wanted;
        for font in short {
            let Some(facts) = self.session.get(id).and_then(|d| d.font_facts(page, &font)) else { continue };
            let wanted = Wanted::from_facts(&facts, None, None, &text);
            let key = key_of(&wanted);
            match self.fonts.choices.get(&(id, key.clone())).cloned() {
                Some(FontChoice::Keep) => continue,
                Some(FontChoice::Use { path, whole }) => {
                    if let Some(resource) = self.load_substitute(id, page, &path)
                        && whole
                    {
                        style.resource = Some(resource);
                        break;
                    }
                    continue;
                }
                None => {}
            }
            let Some((best, quality)) = self.styled_face(&wanted) else { continue };
            if matches!(quality, Match::Exact | Match::Fingerprint) {
                // The same family is installed: it draws the missing characters, as it is.
                self.fonts.choices.insert((id, key), FontChoice::Use { path: best.path.clone(), whole: false });
                let _ = self.load_substitute(id, page, &best.path);
                continue;
            }
            self.open_offer(id, page, &facts, wanted, Some(best.path), quality == Match::Other, Some(rebuild(style)), false);
            return Prepared::Wait;
        }
        Prepared::Apply(rebuild(style))
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
        Match::Other => f.display(),
    }
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
                    ed.fonts = crate::edit_text_ui::install_source_fonts(ctx, doc, offer.page);
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
