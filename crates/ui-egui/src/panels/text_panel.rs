//! Sintec.PDF: the floating Format text panel. It floats over the right edge of the active
//! document's page view and shows only while text is being edited: a paragraph open in the line
//! editor, added text being typed, a selected added text box, or the Add text tool. Its controls
//! are Edit a PDF's Format text ([`super::edit::text_format`]), which the Edit panel shows instead
//! when the page view is too small to hold this one; for a paragraph it adds the Source font
//! section (what the font really is, what of it the document carries, whether this computer has
//! it, and the face that draws what it lacks). It sits above the page but below the inline
//! editors, and never takes the keyboard: the paragraph keeps its caret, and Esc still reaches
//! the editor. The pages take the pointer only where egui has their own layer under it, so a click
//! on the panel never reaches the page under it.

use egui::{Align2, CornerRadius, Margin, Rect, Stroke, pos2};

use crate::PdfCraftApp;
use crate::theme::Tokens;

/// The panel's width: the Format text controls' row, the scroll bar, the margins.
const WIDTH: f32 = 300.0;
/// Space between the panel and the document area's edges.
const GAP: f32 = 10.0;
/// Room along the right edge for the page view's vertical scrollbar.
const SCROLLBAR: f32 = 12.0;
/// The frame's inner margin.
const PAD: i8 = 12;
/// The frame's border width.
const BORDER: f32 = 1.0;

/// Whether view `index` has text being edited or added that Format text applies to.
fn has_target(app: &PdfCraftApp, index: usize) -> bool {
    let Some(view) = app.views.get(index) else { return false };
    if view.line_editor.is_some() || view.content.draft.is_some() {
        return true;
    }
    // A selected image has its own controls among the tools (Edit image).
    if super::edit::image_selected(app, index) {
        return false;
    }
    view.content.selected.is_some() || app.quick_tool == crate::QuickTool::AddText
}

/// The panel's width over view `index`'s page view, or `None` when the view can't hold it.
fn width_in(viewport: Rect) -> Option<f32> {
    if !viewport.is_positive() {
        return None;
    }
    let width = WIDTH.min(viewport.width() - 2.0 * GAP - SCROLLBAR);
    // A page view too small to hold it: the controls would cover the page and the rail.
    (width >= 2.0 * f32::from(PAD) + 120.0).then_some(width)
}

/// Whether the panel floats over view `index` (the Edit panel then leaves Format text to it).
pub(crate) fn floats(app: &PdfCraftApp, index: usize) -> bool {
    let Some(view) = app.views.get(index) else { return false };
    app.active == Some(index) && !app.full_screen && !view.organize && width_in(view.viewport_rect()).is_some() && has_target(app, index)
}

/// The layer document `doc`'s panel is drawn on: above the page, below the inline editors.
pub(crate) fn layer(doc: pdfcraft_engine::DocId) -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, egui::Id::unique(("text-format-panel", doc.0)))
}

/// Whether the point `p` belongs to view `view`'s floating panel (or to a popup over the page,
/// such as its font lists, while the panel shows): a click there keeps the paragraph open.
pub(crate) fn owns(ctx: &egui::Context, view: &crate::DocView, p: egui::Pos2) -> bool {
    view.text_panel.is_positive()
        && match ctx.layer_id_at(p) {
            // A popup (a font list, the colour swatches) is drawn above the page and the editors:
            // a press there leaves the editor only while it looks away from the text.
            Some(l) => l == layer(view.id) || (l.order >= egui::Order::Foreground && egui::Popup::is_any_open(ctx)),
            None => view.text_panel.contains(p),
        }
}

/// Draw the panel over view `index`'s document area, after the page view has been laid out
/// this frame (its `viewport_rect` is current).
pub(crate) fn show(app: &mut PdfCraftApp, index: usize, ctx: &egui::Context) {
    let shown = floats(app, index);
    let Some(view) = app.views.get_mut(index) else { return };
    view.text_panel = Rect::NOTHING;
    if !shown {
        return;
    }
    let (doc_view, viewport) = (view.id, view.viewport_rect());
    let Some(width) = width_in(viewport) else { return };
    let pad = f32::from(PAD);
    let right = viewport.right() - GAP - SCROLLBAR;
    let max_height = (viewport.height() - 2.0 * GAP - 2.0 * pad).max(40.0);
    let t = Tokens::get(ctx);
    let layer = layer(doc_view);
    // Pinned by its top-right corner, and kept inside the page view, so it never spills over the
    // rail.
    let area = egui::Area::new(layer.id).order(layer.order).pivot(Align2::RIGHT_TOP).fixed_pos(pos2(right, viewport.top() + GAP));
    let area = area.constrain_to(viewport).show(ctx, |ui| {
        let frame = egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(BORDER, t.border)).corner_radius(CornerRadius::same(8));
        let frame = frame.inner_margin(Margin::same(PAD)).shadow(ui.visuals().popup_shadow);
        frame.show(ui, |ui| {
            // Room for the scroll bar, which sits beside the controls when they don't fit, and
            // for the border.
            let bar = ui.spacing().scroll.allocated_width();
            ui.set_width((width - 2.0 * (pad + BORDER) - bar).max(0.0));
            // An area keeps the size it had last frame as the room it offers; let the controls
            // take the page view's height.
            ui.set_max_height(max_height);
            let scroll = egui::ScrollArea::vertical().id_salt("text-format-panel-scroll").max_height(max_height).auto_shrink([false, true]);
            scroll.show(ui, |ui| {
                let paragraph = app.views.get(index).is_some_and(|v| v.line_editor.is_some());
                super::edit::text_format(app, ui, &t);
                if paragraph {
                    source_font(app, index, ui, &t);
                }
            });
        });
    });
    if let Some(view) = app.views.get_mut(index) {
        view.text_panel = area.response.rect;
    }
}

/// Sintec.PDF: the paragraph's font as it really is, what of it the document carries, whether
/// this computer has it, and which face draws the characters it lacks; Replace font… offers the
/// closest installed faces, and Installed fonts… lists them, the closest first.
fn source_font(app: &mut PdfCraftApp, i: usize, ui: &mut egui::Ui, t: &Tokens) {
    let Some(ed) = app.views.get(i).and_then(|v| v.line_editor.clone()) else { return };
    let Some(doc_id) = app.views.get(i).map(|v| v.id) else { return };
    ui.add_space(4.0);
    crate::widgets::section_title(ui, tl!("Source font"));
    let source = app.session.get(doc_id).and_then(|d| {
        let b = d.text_blocks(ed.page).into_iter().nth(ed.block)?;
        let line = b.lines.first().and_then(|li| d.text_lines(ed.page).into_iter().nth(*li))?;
        let lacks = d.first_unencodable(ed.page, &line.font, &ed.text);
        d.font_facts(ed.page, &line.font).map(|f| (f, b.size, b.color, lacks))
    });
    // The same installed faces the Font substitution dialog uses (a known set when embedders or
    // tests set one), so the panel and the dialog never disagree about what is installed.
    #[cfg(not(target_arch = "wasm32"))]
    let installed = app.installed_faces();
    let Some((facts, size, color, lacks)) = &source else { return };
    // What the font really is (its program's family, the detected weight and slant), the
    // effective size and colour: what new text in the paragraph is set in.
    let [r, g, b] = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let look = format!("{} · {size:.1} pt · #{r:02X}{g:02X}{b:02X}", styled(facts));
    ui.label(egui::RichText::new(look).small()).on_hover_text(detection(facts));
    let carried = if facts.embedded && !facts.subset {
        tl!("embedded")
    } else if facts.embedded {
        tl!("embedded in part")
    } else {
        tl!("not embedded")
    };
    #[cfg(not(target_arch = "wasm32"))]
    let on_system = installed.as_ref().map(|fonts| {
        let want = pdfcraft_engine::text_fonts::Wanted::from_facts(facts, None, None, "");
        fonts.iter().any(|f| !f.bundled && pdfcraft_engine::text_fonts::family_match(&want, f) == pdfcraft_engine::text_fonts::Match::Exact)
    });
    #[cfg(target_arch = "wasm32")]
    let on_system: Option<bool> = None;
    let system = match on_system {
        Some(true) => format!(", {}", tl!("installed")),
        Some(false) => format!(", {}", tl!("not installed")),
        None => String::new(),
    };
    let tip = tl!("The paragraph's font as the PDF names it, how much of it the document carries, and whether this computer has it.");
    ui.label(egui::RichText::new(format!("{} — {carried}{system}", facts.base_font)).small().color(t.text_muted)).on_hover_text(tip);
    // Which face draws the characters the document's font lacks, and why.
    #[cfg(not(target_arch = "wasm32"))]
    if facts.subset || !facts.embedded || lacks.is_some() {
        let (line, tip) = match app.stand_in(doc_id, facts, &ed.text) {
            Some(stand) => stand_line(&stand),
            None if installed.is_none() => (tl!("Finding installed fonts…").to_string(), String::new()),
            None => (tl!("Missing characters: the document's font is kept").to_string(), String::new()),
        };
        let color = if lacks.is_some() { t.text } else { t.text_muted };
        ui.label(egui::RichText::new(line).small().color(color)).on_hover_text(tip);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = lacks;
    if ui.button(tl!("Replace font…")).clicked() {
        app.offer_for_paragraph(doc_id, ed.page, ed.block, &ed.text);
    }
    #[cfg(not(target_arch = "wasm32"))]
    craft_ui::ComboBox::from_id_salt("installed-document-font").selected_text(tl!("Installed fonts…")).show_ui(ui, |ui| {
        let Some(fonts) = installed.as_ref() else {
            ui.label(tl!("Finding document fonts…"));
            return;
        };
        if fonts.is_empty() {
            ui.label(tl!("No embeddable installed fonts found. Load a font file."));
        }
        // The closest to the paragraph's font first.
        let want = pdfcraft_engine::text_fonts::Wanted::from_facts(facts, None, None, &ed.text);
        for font in pdfcraft_engine::text_fonts::rank(&want, fonts) {
            if ui.selectable_label(false, crate::font_substitute::listed_name(font)).clicked() {
                app.use_paths(crate::files::FilePurpose::TextFont { page: ed.page }, std::slice::from_ref(&font.path));
            }
        }
    });
}

/// Sintec.PDF: "Calibri Bold Italic": the font's family with its detected weight and slant.
fn styled(facts: &pdfcraft_engine::FontFacts) -> String {
    let weight = match facts.weight {
        0 => None,
        1..=150 => Some(tl_ctx!("font weight", "Thin")),
        151..=250 => Some(tl_ctx!("font weight", "Extra Light")),
        251..=350 => Some(tl_ctx!("font weight", "Light")),
        351..=450 => None,
        451..=550 => Some(tl_ctx!("font weight", "Medium")),
        551..=650 => Some(tl_ctx!("font weight", "Semibold")),
        651..=750 => Some(tl_ctx!("font weight", "Bold")),
        751..=850 => Some(tl_ctx!("font weight", "Extra Bold")),
        _ => Some(tl_ctx!("font weight", "Black")),
    };
    let weight = if facts.weight == 0 && facts.is_bold() { Some(tl_ctx!("font weight", "Bold")) } else { weight };
    // A name without a program says its style itself (`Arial-BoldMT`): show its family part.
    let mut s = match &facts.program_family {
        Some(family) => family.clone(),
        None => facts.base_font.split(['-', ',']).next().unwrap_or(&facts.base_font).to_string(),
    };
    for part in [weight, facts.is_italic().then(|| tl_ctx!("font weight", "Italic"))].into_iter().flatten() {
        s.push(' ');
        s.push_str(part);
    }
    s
}

/// Sintec.PDF: how the weight and slant were found, for the look's tooltip.
fn detection(facts: &pdfcraft_engine::FontFacts) -> String {
    use pdfcraft_engine::StyleSource as S;
    let how = |source: S| match source {
        S::Program => tl!("from the embedded font program"),
        S::Descriptor => tl!("from the PDF's font descriptor"),
        S::Name => tl!("from the font's name"),
        S::ForceBold => tl!("from the font's ForceBold flag"),
        S::Stems => tl!("from the thickness of the letters' stems"),
        S::StemV => tl!("from the font descriptor's stem width"),
        S::Default | S::Unknown => tl!("nothing says otherwise"),
    };
    let slant = match facts.italic_from {
        Some(source) => crate::i18n::fmt(tl!("italic, {how}"), &[("how", how(source))]),
        None => tl!("upright").to_string(),
    };
    crate::i18n::fmt(
        tl!(
            "Detected weight {weight}, {how}; slant: {slant}. The size is the font size times the text's scale on the page. New text in the paragraph is set in this style."
        ),
        &[("weight", &facts.weight.to_string()), ("how", how(facts.weight_from)), ("slant", &slant)],
    )
}

/// Sintec.PDF: the status line about the face that draws what the document's font lacks, and its
/// tooltip.
#[cfg(not(target_arch = "wasm32"))]
fn stand_line(stand: &crate::font_substitute::Stand) -> (String, String) {
    use pdfcraft_engine::text_fonts::Match;
    let kind = if stand.chosen {
        tl!("your choice")
    } else {
        match stand.quality {
            Match::Exact => tl!("installed"),
            Match::Fingerprint => tl!("same glyph widths"),
            Match::Alias if stand.face.bundled => tl!("metric-compatible, included with Sintec.PDF"),
            Match::Alias => tl!("metric-compatible substitute"),
            Match::Other => tl!("no similar font: you'll be asked"),
        }
    };
    let mut face = stand.face.display();
    if stand.synthetic_bold {
        face = crate::i18n::fmt(tl!("{face} (synthetic bold)"), &[("face", &face)]);
    }
    let line = crate::i18n::fmt(tl!("Missing characters: {face} — {kind}"), &[("face", &face), ("kind", kind)]);
    let tip = tl!(
        "Characters the document's copy of the font doesn't have are drawn in this face, sized to the font's capital height and widths; a regular face standing in for a bold font is drawn heavier. Replace font… chooses another; the Format text controls change family, weight and slant."
    );
    (line, tip.to_string())
}
