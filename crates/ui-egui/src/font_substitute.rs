//! Sintec.PDF: a substitute for a font the edited text needs but the PDF doesn't fully carry.
//!
//! PDFs usually embed only the glyphs their text uses (a subset), or no program at all. Text
//! typed into such a paragraph may need glyphs that aren't there. Before the edit is applied,
//! [`PdfCraftApp::prepare_text_edit`] checks that: when the same family is installed it is
//! loaded into the page silently; otherwise the Font substitution dialog offers the installed
//! faces that look most like it (`pdfcraft_engine::text_fonts::rank`), and the edit waits for the
//! choice. A choice is remembered per document and font. An edit that still fails for want of a
//! glyph opens the dialog too.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, Layout};
use pdfcraft_engine::text_fonts::{FontFile, Match, Wanted, family_match, rank};
use pdfcraft_engine::{DocId, Edit};

use crate::{PdfCraftApp, theme, widgets};

/// What the user chose for a font, per document and font family.
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
    /// The font's family key (the memory's key) and how it reads ("TimesNewRomanPS-BoldMT").
    pub key: String,
    pub base_font: String,
    pub wanted: Wanted,
    /// Why it's asked: not installed, or the document lacks glyphs; whether the document
    /// carries any of the font.
    pub installed: bool,
    pub embedded: bool,
    pub choice: Option<PathBuf>,
    pub whole: bool,
    /// The edit waiting for the choice; `None` when opened from Format text, where the chosen
    /// face becomes the paragraph's font.
    pub pending: Option<Edit>,
}

/// Choices and the open dialog.
#[derive(Default)]
pub struct FontSubstitutes {
    pub choices: HashMap<(DocId, String), FontChoice>,
    pub offer: Option<FontOffer>,
    /// The edit being applied came from the dialog: don't check it again.
    pub(crate) bypass: bool,
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
    /// The font facts and the font wanted for paragraph `block` of page `page` with `text`.
    fn wanted_for(&self, id: DocId, page: usize, block: usize, text: &str) -> Option<(pdfcraft_engine::FontFacts, Wanted, String)> {
        let doc = self.session.get(id)?;
        let b = doc.text_blocks(page).into_iter().nth(block)?;
        let lines = doc.text_lines(page);
        let line = b.lines.first().and_then(|i| lines.get(*i))?;
        let facts = doc.font_facts(page, &line.font)?;
        let wanted = Wanted {
            base_font: facts.base_font.clone(),
            bold: facts.bold || b.bold,
            italic: facts.italic || b.italic,
            serif: facts.serif,
            monospace: facts.monospace,
            sample: text.to_string(),
        };
        Some((facts, wanted, b.editor_text))
    }

    /// Before a text edit is applied: load the substitute it needs, or ask for one.
    pub(crate) fn prepare_text_edit(&mut self, id: DocId, edit: Edit) -> Prepared {
        if std::mem::take(&mut self.fonts.bypass) {
            return Prepared::Apply(edit);
        }
        let Edit::EditTextBlock { page, block, text, style } = &edit else { return Prepared::Apply(edit) };
        let (page, block) = (*page, *block);
        if style.resource.is_some() {
            return Prepared::Apply(edit);
        }
        let Some((facts, wanted, source_text)) = self.wanted_for(id, page, block, text) else { return Prepared::Apply(edit) };
        // A full program, a family already completed, or only characters the paragraph already
        // shows: nothing new is needed.
        let new_chars = text.chars().any(|c| !c.is_whitespace() && !source_text.contains(c));
        if (facts.embedded && !facts.subset) || facts.completed || !new_chars {
            return Prepared::Apply(edit);
        }
        let key = pdfcraft_engine::text_fonts::family_key(&facts.base_font);
        match self.fonts.choices.get(&(id, key.clone())).cloned() {
            Some(FontChoice::Keep) => return Prepared::Apply(edit),
            Some(FontChoice::Use { path, whole }) => return self.with_substitute(id, page, &path, whole, edit),
            None => {}
        }
        let Some(fonts) = installed(self.ctx.as_ref()) else { return Prepared::Apply(edit) };
        let ranked = rank(&wanted, &fonts);
        let Some(best) = ranked.first() else { return Prepared::Apply(edit) };
        // The same family is installed: use it for the missing characters, as it is.
        if family_match(&wanted, best) == Match::Exact {
            let path = best.path.clone();
            self.fonts.choices.insert((id, key), FontChoice::Use { path: path.clone(), whole: false });
            return self.with_substitute(id, page, &path, false, edit);
        }
        let whole = family_match(&wanted, best) == Match::Other;
        self.fonts.offer = Some(FontOffer {
            doc: id,
            page,
            key,
            base_font: facts.base_font,
            wanted,
            installed: false,
            embedded: facts.embedded,
            choice: Some(best.path.clone()),
            whole,
            pending: Some(edit),
        });
        Prepared::Wait
    }

    /// An edit failed for want of a glyph: offer a substitute (once per edit).
    pub(crate) fn offer_after_failure(&mut self, id: DocId, edit: &Edit, message: &str) -> bool {
        let Edit::EditTextBlock { page, block, text, .. } = edit else { return false };
        let glyphs = ["fallback font", "cannot show U+", "complete source TrueType", "can't draw"];
        if !glyphs.iter().any(|g| message.contains(g)) || self.fonts.offer.is_some() {
            return false;
        }
        let Some((facts, wanted, _)) = self.wanted_for(id, *page, *block, text) else { return false };
        let fonts = installed(self.ctx.as_ref()).unwrap_or_default();
        let ranked = rank(&wanted, &fonts);
        let installed = ranked.first().is_some_and(|f| family_match(&wanted, f) == Match::Exact);
        let whole = ranked.first().is_none_or(|f| family_match(&wanted, f) == Match::Other);
        self.fonts.offer = Some(FontOffer {
            doc: id,
            page: *page,
            key: pdfcraft_engine::text_fonts::family_key(&facts.base_font),
            base_font: facts.base_font,
            wanted,
            installed,
            embedded: facts.embedded,
            choice: ranked.first().map(|f| f.path.clone()),
            whole,
            pending: Some(edit.clone()),
        });
        true
    }

    /// Format text ▸ Replace font…: choose a face for the paragraph being edited.
    pub(crate) fn offer_for_paragraph(&mut self, id: DocId, page: usize, block: usize, text: &str) {
        let Some((facts, wanted, _)) = self.wanted_for(id, page, block, text) else { return };
        let fonts = installed(self.ctx.as_ref()).unwrap_or_default();
        let ranked = rank(&wanted, &fonts);
        let installed = ranked.first().is_some_and(|f| family_match(&wanted, f) == Match::Exact);
        self.fonts.offer = Some(FontOffer {
            doc: id,
            page,
            key: pdfcraft_engine::text_fonts::family_key(&facts.base_font),
            base_font: facts.base_font,
            wanted,
            installed,
            embedded: facts.embedded,
            choice: ranked.first().map(|f| f.path.clone()),
            whole: true,
            pending: None,
        });
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
