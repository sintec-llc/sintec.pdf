//! Sintec.PDF: the floating Format text panel. It floats over the right edge of the document
//! area, just left of the right icon rail (and of a docked inspector), and shows only while text
//! is being edited: a paragraph open in the line editor, a selected added text box, or the Add
//! text tool. Each document window (the main one and torn-off ones) draws its own, over its own
//! page view. It sits above the page but below the inline editors, and never takes the keyboard:
//! the paragraph keeps its caret, and Esc still reaches the editor.

use egui::{Align2, CornerRadius, Margin, Rect, Stroke, pos2};

use crate::PdfCraftApp;
use crate::theme::Tokens;

/// The panel's width: the colour swatches' row (about 256 points), the scroll bar, the margins.
const WIDTH: f32 = 300.0;
/// Space between the panel and the document area's edges.
const GAP: f32 = 10.0;
/// Room along the right edge for the page view's vertical scrollbar.
const SCROLLBAR: f32 = 12.0;
/// The frame's inner margin.
const PAD: i8 = 12;
/// The frame's border width.
const BORDER: f32 = 1.0;

/// What the panel formats.
enum Target {
    /// The paragraph open in the line editor.
    Paragraph,
    /// A selected added text box: page, index among the page's added items, its text.
    Added(usize, usize, pdfcraft_engine::AddedText),
    /// The Add text tool with nothing selected: the style new text gets.
    NewText,
}

fn target(app: &PdfCraftApp, index: usize) -> Option<Target> {
    let view = app.views.get(index)?;
    if view.line_editor.is_some() {
        return Some(Target::Paragraph);
    }
    let doc = app.session.get(view.id);
    let selected = view.content.selected.and_then(|(page, i)| {
        let item = doc?.added.iter().filter(|a| a.page == page).nth(i)?;
        Some((page, i, &item.content))
    });
    match selected {
        Some((page, i, pdfcraft_engine::AddedContent::Text(text))) => Some(Target::Added(page, i, text.clone())),
        // A selected image has its own panel among the tools (Edit image).
        Some(_) => None,
        None => (app.quick_tool == crate::QuickTool::AddText).then_some(Target::NewText),
    }
}

/// The layer document `doc`'s panel is drawn on: above the page, below the inline editors.
pub(crate) fn layer(doc: pdfcraft_engine::DocId) -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, egui::Id::new(("text-format-panel", doc.0)))
}

/// Draw the panel over view `index`'s document area, after the page view has been laid out
/// this frame (its `viewport_rect` is current).
pub(crate) fn show(app: &mut PdfCraftApp, index: usize, ctx: &egui::Context) {
    let Some(view) = app.views.get_mut(index) else { return };
    view.text_panel = Rect::NOTHING;
    if app.full_screen || view.organize {
        return;
    }
    let (doc_view, viewport) = (view.id, view.viewport_rect());
    let Some(target) = target(app, index) else { return };
    if !viewport.is_positive() {
        return;
    }
    let pad = f32::from(PAD);
    let width = WIDTH.min(viewport.width() - 2.0 * GAP - SCROLLBAR);
    // A page view too small to hold it: the controls would cover the page and the rail.
    if width < 2.0 * pad + 120.0 {
        return;
    }
    let right = viewport.right() - GAP - SCROLLBAR;
    let max_height = (viewport.height() - 2.0 * GAP - 2.0 * pad).max(40.0);
    let t = Tokens::get(ctx);
    let layer = layer(doc_view);
    let area = egui::Area::new(layer.id);
    // Pinned by its top-right corner, and kept inside the page view, so it never spills over
    // the rail.
    let area = area.order(layer.order).pivot(Align2::RIGHT_TOP).fixed_pos(pos2(right, viewport.top() + GAP));
    let area = area.constrain_to(viewport).show(ctx, |ui| {
        let frame = egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(BORDER, t.border)).corner_radius(CornerRadius::same(8));
        let frame = frame.inner_margin(Margin::same(PAD)).shadow(ui.visuals().popup_shadow);
        frame.show(ui, |ui| {
            // Room for the scroll bar, which sits beside the controls when they don't fit, and
            // for the border.
            let bar = ui.spacing().scroll.allocated_width();
            ui.set_width((width - 2.0 * (pad + BORDER) - bar).max(0.0));
            // An area keeps the size it had last frame (400 points tall on its first, sizing
            // pass) as the room it offers; let the controls take the page view's height.
            ui.set_max_height(max_height);
            let scroll = egui::ScrollArea::vertical().id_salt("text-format-panel-scroll").max_height(max_height).auto_shrink([false, true]);
            scroll.show(ui, |ui| body(app, index, target, ui, &t));
        });
    });
    if let Some(view) = app.views.get_mut(index) {
        view.text_panel = area.response.rect;
    }
}

/// The controls for what is being edited; each applies as it changes, as one undoable step.
fn body(app: &mut PdfCraftApp, index: usize, target: Target, ui: &mut egui::Ui, t: &Tokens) {
    match target {
        Target::Paragraph => paragraph(app, index, ui, t),
        Target::Added(page, i, text) => {
            if let Some(style) = crate::content_ui::format_panel(ui, t, &text) {
                app.text_style = pdfcraft_engine::AddedText { text: String::new(), rect: [0.0; 4], ..style.clone() };
                app.apply_edit(pdfcraft_engine::Edit::UpdateContent { page, index: i, content: pdfcraft_engine::AddedContent::Text(style) });
            }
        }
        Target::NewText => {
            crate::content_ui::hint(ui, t);
            if let Some(style) = crate::content_ui::format_panel(ui, t, &app.text_style) {
                app.text_style = style;
            }
        }
    }
}

/// Editing a paragraph of existing text: its formatting, then its font, what of it the document
/// carries, applied as they change (one undoable step each).
fn paragraph(app: &mut PdfCraftApp, i: usize, ui: &mut egui::Ui, t: &Tokens) {
    let Some(mut ed) = app.views.get(i).and_then(|v| v.line_editor.clone()) else { return };
    let Some(doc_id) = app.views.get(i).map(|v| v.id) else { return };
    let mut changed = false;
    // Format text first: its title heads the panel.
    if let Some(look) = crate::content_ui::format_panel(ui, t, &ed.look) {
        ed.look = look;
        changed = true;
    }
    changed |= crate::edit_text_ui::extras_panel(ui, &mut ed.extras);
    ui.add_space(4.0);
    crate::widgets::section_title(ui, tl!("Source font"));
    // Sintec.PDF: which font the paragraph is in, what of it the document carries, and
    // whether this computer has it; Replace font… offers the closest installed faces.
    let source = app.session.get(doc_id).and_then(|d| {
        let b = d.text_blocks(ed.page).into_iter().nth(ed.block)?;
        let line = b.lines.first().and_then(|li| d.text_lines(ed.page).into_iter().nth(*li))?;
        d.font_facts(ed.page, &line.font).map(|f| (f, b.bold, b.italic))
    });
    let mut replace = false;
    if let Some((facts, bold, italic)) = &source {
        let carried = if facts.embedded && !facts.subset {
            tl!("embedded")
        } else if facts.embedded {
            tl!("embedded in part")
        } else {
            tl!("not embedded")
        };
        #[cfg(not(target_arch = "wasm32"))]
        let on_system = crate::files::installed_font_files(ui.ctx()).map(|fonts| {
            let base_font = facts.base_font.clone();
            let want = pdfcraft_engine::text_fonts::Wanted { base_font, bold: *bold, italic: *italic, ..Default::default() };
            fonts.iter().any(|f| !f.bundled && pdfcraft_engine::text_fonts::family_match(&want, f) == pdfcraft_engine::text_fonts::Match::Exact)
        });
        #[cfg(target_arch = "wasm32")]
        let on_system: Option<bool> = {
            let _ = (bold, italic);
            None
        };
        let system = match on_system {
            Some(true) => format!(", {}", tl!("installed")),
            Some(false) => format!(", {}", tl!("not installed")),
            None => String::new(),
        };
        let tip = tl!("The paragraph's font as the PDF names it, how much of it the document carries, and whether this computer has it.");
        ui.label(egui::RichText::new(format!("{} — {carried}{system}", facts.base_font)).small().color(t.text_muted)).on_hover_text(tip);
        replace = ui.button(tl!("Replace font…")).clicked();
    }
    if replace {
        app.offer_for_paragraph(doc_id, ed.page, ed.block, &ed.text.clone());
    }
    let font_choices = app.session.get(doc_id).map(|d| d.text_editor_fonts(ed.page)).unwrap_or_default();
    let chosen = ed.font_resource.as_ref().and_then(|r| font_choices.iter().find(|f| &f.resource == r));
    let shown = chosen.map_or(tl!("Keep source fonts"), |f| f.face.as_str());
    egui::ComboBox::from_id_salt("document-font-resource").selected_text(shown).show_ui(ui, |ui| {
        changed |= ui.selectable_value(&mut ed.font_resource, None, tl!("Keep source fonts")).changed();
        for font in font_choices.iter().filter(|f| !f.resource.starts_with("PCApproved")) {
            changed |= ui.selectable_value(&mut ed.font_resource, Some(font.resource.clone()), &font.face).changed();
        }
    });
    if ui.button(tl!("Load font…")).clicked() {
        app.pick_files(crate::files::FilePurpose::TextFont { page: ed.page }, false);
    }
    #[cfg(not(target_arch = "wasm32"))]
    egui::ComboBox::from_id_salt("installed-document-font").selected_text(tl!("Installed fonts…")).show_ui(ui, |ui| {
        if let Some(fonts) = crate::files::installed_font_files(ui.ctx()) {
            if fonts.is_empty() {
                ui.label(tl!("No embeddable installed fonts found. Load a font file."));
            }
            // The closest to the paragraph's font first.
            let want = source.as_ref().map(|(f, bold, italic)| pdfcraft_engine::text_fonts::Wanted {
                base_font: f.base_font.clone(),
                bold: *bold,
                italic: *italic,
                serif: f.serif,
                monospace: f.monospace,
                sample: ed.text.clone(),
                widths: f.widths.clone(),
            });
            let ordered: Vec<&pdfcraft_engine::text_fonts::FontFile> = match &want {
                Some(w) => pdfcraft_engine::text_fonts::rank(w, &fonts),
                None => fonts.iter().collect(),
            };
            for font in ordered {
                if ui.selectable_label(false, crate::font_substitute::listed_name(font)).clicked() {
                    app.use_paths(crate::files::FilePurpose::TextFont { page: ed.page }, std::slice::from_ref(&font.path));
                }
            }
        } else {
            ui.label(tl!("Finding document fonts…"));
        }
    });
    if changed {
        let edit = pdfcraft_engine::Edit::EditTextBlock { page: ed.page, block: ed.block, text: ed.text.clone(), style: ed.style() };
        if app.apply_edit(edit) {
            ed.applied();
            if let Some(doc) = app.session.get(doc_id)
                && let Some(block) = doc.text_blocks(ed.page).get(ed.block)
            {
                ed.refresh_source(block, doc.text_lines(ed.page));
                ed.fonts = crate::edit_text_ui::install_source_fonts(ui.ctx(), doc, ed.page);
            }
        }
        if let Some(view) = app.views.get_mut(i) {
            view.line_editor = Some(ed);
        }
    }
}
