//! Sintec.PDF: a translucent copy of what is being dragged, under the pointer: a document tab
//! (reordering it, pulling it out into a window of its own, moving it to another window) or
//! pages leaving the Pages panel for another document.
//!
//! Pointer events go to the window a drag started in, but the pointer may be over another
//! window: the source records where the ghost is on the screen ([`PdfCraftApp::drag_ghost`]),
//! and every window draws it when that point falls inside it.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use pdfcraft_engine::DocId;

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, icons};

/// Where the ghost hangs from the pointer: below and to the right, clear of the cursor.
const OFFSET: Vec2 = vec2(14.0, 16.0);
/// The ghost card's widest.
const CARD_MAX_WIDTH: f32 = 240.0;
/// The page preview's width.
const PREVIEW_WIDTH: f32 = 96.0;

/// What is being dragged, and where on the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    /// The pointer, in screen points.
    pub screen: Pos2,
    /// The card's text: the document's name, or "3 pages · name".
    pub title: String,
    /// The document whose page is previewed, and that page.
    pub doc: DocId,
    pub page: usize,
    /// How many pages are dragged (0: a whole document, its tab).
    pub pages: usize,
    /// Show the page preview under the card (pages; a tab once it leaves the strip).
    pub preview: bool,
}

/// Draw the ghost in this window if the pointer is over it. Nothing while no drag is under way.
pub(crate) fn paint(app: &PdfCraftApp, ctx: &egui::Context) {
    if app.tab_drag.is_none() && app.page_transfer.is_none() {
        return;
    }
    let Some(g) = &app.drag_ghost else { return };
    let at = g.screen - crate::doc_windows::screen_offset(ctx);
    if !ctx.content_rect().contains(at) {
        return;
    }
    let t = Tokens::get(ctx);
    let thumb = app.views.iter().find(|v| v.id == g.doc).and_then(|v| v.thumb(g.page)).map(|tex| (tex.id(), tex.size_vec2()));
    egui::Area::new(egui::Id::new("drag-ghost")).order(egui::Order::Tooltip).interactable(false).fixed_pos(at + OFFSET).show(ctx, |ui| {
        ui.set_opacity(0.88);
        card(ui, &t, g);
        if g.preview {
            ui.add_space(4.0);
            preview(ui, &t, g, thumb);
        }
    });
    ctx.request_repaint();
}

/// The tab-like card: the document icon and the title.
fn card(ui: &mut egui::Ui, t: &Tokens, g: &Ghost) {
    let font = theme::regular(13.0);
    let title = crate::bidi::visual(&g.title).into_owned();
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(title, font, t.text));
    let width = (galley.size().x + 40.0).min(CARD_MAX_WIDTH);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(7), t.chrome, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    icons::paint(
        ui,
        Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0)),
        if g.pages > 0 { "files" } else { "file-text" },
        15.0,
        t.accent,
    );
    let text_rect = Rect::from_min_max(rect.min + vec2(30.0, 0.0), rect.max - vec2(8.0, 0.0));
    painter.with_clip_rect(text_rect).galley(pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0), galley, t.text);
}

/// The dragged page: its thumbnail when the Pages panel has rendered one, else a blank sheet;
/// with a count badge when several pages are dragged.
fn preview(ui: &mut egui::Ui, t: &Tokens, g: &Ghost, thumb: Option<(egui::TextureId, Vec2)>) {
    // A4 portrait until the thumbnail says otherwise.
    let aspect = thumb.map_or(1.414, |(_, s)| if s.x > 0.0 { (s.y / s.x).clamp(0.3, 3.0) } else { 1.414 });
    let (rect, _) = ui.allocate_exact_size(vec2(PREVIEW_WIDTH, PREVIEW_WIDTH * aspect), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect.translate(vec2(2.0, 3.0)), CornerRadius::same(3), t.page_shadow);
    match thumb {
        Some((id, _)) => {
            painter.image(id, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(rect, CornerRadius::same(2), Color32::WHITE);
        }
    }
    painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    if g.pages > 1 {
        let badge = Rect::from_center_size(rect.right_top() + vec2(-2.0, 2.0), vec2(26.0, 20.0));
        painter.rect_filled(badge, CornerRadius::same(10), t.accent);
        painter.text(badge.center(), egui::Align2::CENTER_CENTER, g.pages.to_string(), theme::medium(12.0), t.accent_text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_drawn_without_a_drag() {
        let app = PdfCraftApp {
            drag_ghost: Some(Ghost { screen: pos2(10.0, 10.0), title: "a.pdf".into(), doc: DocId(1), page: 0, pages: 0, preview: true }),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let out = ctx.run_ui(Default::default(), |ui| paint(&app, ui.ctx()));
        let drawn = !out.shapes.is_empty();
        out.drop_without_applying_deltas();
        assert!(!drawn, "a stale ghost after the drag ended is not drawn");
    }
}
