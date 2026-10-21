//! Sintec.PDF: document tabs in windows of their own.
//!
//! A tab dragged out of the tab strip opens in a new window; dragged onto another window's strip
//! it joins that window; dragged along its own strip it moves there. Every window shows the same
//! tools and panels for its own tabs, and pages drag between documents in any windows.
//!
//! The desktop app opens a torn-off tab in a process of its own ([`crate::tear_off`]); these
//! windows are the fallback where it can't. They are egui viewports in this one process, so they
//! share the documents, undo history, settings and clipboard. `DocView::window` says which window shows a tab (0: the main window);
//! while a window is drawn, `active` is its shown tab and `current_window` is its id.
//!
//! A drag started in one window gets every pointer event until the button is let go, wherever the
//! pointer is, so a drop onto another window is found from where each window drew its tab strip
//! and Pages panel last frame, in screen points ([`crate::WindowGeometry`]). Windows of other
//! processes publish their strips through [`crate::tab_transfer`].

use egui::{Context, Pos2, Rect, Ui, Vec2, vec2};
use pdfcraft_engine::DocId;

use crate::{DocWindow, Mode, PdfCraftApp, chrome, dialogs, palette, panels, theme};

/// Where this window's content starts on the screen (zero when the platform doesn't say, as in
/// tests, where every window is drawn inside the main one).
pub(crate) fn screen_offset(ctx: &Context) -> Vec2 {
    ctx.input(|i| i.viewport().inner_rect).map_or(Vec2::ZERO, |r| r.min.to_vec2())
}

/// The gap (0 = before the first) nearest `y` among thumbnails `rows`, top to bottom.
pub(crate) fn gap_at(rows: &[Rect], y: f32) -> usize {
    rows.iter().position(|r| y < r.center().y).unwrap_or(rows.len())
}

/// The gap nearest `x` among tabs `tabs`, left to right.
pub(crate) fn tab_gap(tabs: &[(DocId, Rect)], x: f32) -> Option<DocId> {
    tabs.iter().find(|(_, r)| x < r.center().x).map(|(d, _)| *d)
}

impl PdfCraftApp {
    fn view_index(&self, doc: DocId) -> Option<usize> {
        self.views.iter().position(|v| v.id == doc)
    }

    /// How many tabs window `window` shows.
    pub(crate) fn window_tab_count(&self, window: u64) -> usize {
        self.views.iter().filter(|v| v.window == window).count()
    }

    /// Whether tab `index` is the shown one in some window (its page tools take input).
    pub(crate) fn shown_anywhere(&self, index: usize) -> bool {
        self.active == Some(index)
            || self.saved_main_active == Some(Some(index))
            || self.views.get(index).is_some_and(|v| self.doc_windows.iter().any(|w| w.active == Some(v.id)))
    }

    /// Show `doc` in window `window` (whichever window is being drawn now).
    pub(crate) fn set_window_active(&mut self, window: u64, doc: DocId) {
        let Some(i) = self.view_index(doc) else { return };
        if window == self.current_window {
            self.active = Some(i);
        } else if window == 0 {
            match &mut self.saved_main_active {
                Some(saved) => *saved = Some(i),
                None => self.active = Some(i),
            }
        } else if let Some(w) = self.doc_windows.iter_mut().find(|w| w.id == window) {
            w.active = Some(doc);
        }
    }

    /// Move tab `doc` into window `window`, before tab `before` (`None`: after that window's last
    /// tab). It becomes the shown tab there; its old window shows a neighbour.
    pub fn move_tab(&mut self, doc: DocId, window: u64, before: Option<DocId>) {
        let Some(from) = self.view_index(doc) else { return };
        if before == Some(doc) {
            return;
        }
        // Indices shift: remember the shown tabs by document.
        let shown = self.active.and_then(|i| self.views.get(i)).map(|v| v.id);
        let saved_main = self.saved_main_active.flatten().and_then(|i| self.views.get(i)).map(|v| v.id);
        let old_window = self.views[from].window;
        let mut view = self.views.remove(from);
        view.window = window;
        let at = before
            .and_then(|b| self.view_index(b))
            .unwrap_or_else(|| self.views.iter().rposition(|v| v.window == window).map_or(self.views.len(), |i| i + 1));
        self.views.insert(at.min(self.views.len()), view);
        let current = self.current_window;
        self.active = shown.and_then(|d| self.view_index(d)).filter(|&i| self.views.get(i).is_some_and(|v| v.window == current));
        if let Some(saved) = &mut self.saved_main_active {
            *saved = saved_main.and_then(|d| self.views.iter().position(|v| v.id == d)).filter(|&i| self.views.get(i).is_some_and(|v| v.window == 0));
        }
        self.set_window_active(window, doc);
        if old_window != window {
            // The window it left shows another of its tabs (or, emptied, closes or shows Home).
            let neighbour = self.views.iter().find(|v| v.window == old_window).map(|v| v.id);
            let showed_it = if old_window == current {
                self.active.is_none()
            } else if old_window == 0 {
                self.saved_main_active.is_some_and(|s| s.is_none()) || (self.saved_main_active.is_none() && self.active.is_none())
            } else {
                self.doc_windows.iter().any(|w| w.id == old_window && (w.active == Some(doc) || w.active.is_none()))
            };
            match (showed_it, neighbour) {
                (true, Some(n)) => self.set_window_active(old_window, n),
                (true, None) if old_window != 0 => self.doc_windows.retain(|w| w.id != old_window),
                _ => {}
            }
        }
    }

    /// Open tab `doc` in a new window of this process, at `at` on the screen when known (see
    /// [`Self::detach_tab`] for a process of its own).
    pub fn tear_off_tab(&mut self, doc: DocId, at: Option<Pos2>) {
        let id = self.next_window_id;
        self.next_window_id = self.next_window_id.saturating_add(1);
        self.doc_windows.push(DocWindow { id, active: Some(doc), spawn_at: at });
        self.move_tab(doc, id, None);
    }

    /// Close torn-off window `id`: its tabs go back to the main window, so nothing unsaved is lost.
    pub fn close_doc_window(&mut self, id: u64) {
        for v in self.views.iter_mut().filter(|v| v.window == id) {
            v.window = 0;
        }
        self.doc_windows.retain(|w| w.id != id);
    }

    /// Pages dragged out of this window's Pages panel and let go over another window: into its
    /// Pages panel at the gap there, or onto one of its tabs (after the last page). The drag ends
    /// with the button, in the window it started in.
    pub(crate) fn finish_page_transfer_drop(&mut self, ctx: &Context) {
        let Some(tr) = self.page_transfer.as_ref().filter(|tr| tr.window == self.current_window) else { return };
        let from = tr.from;
        if ctx.input(|i| i.pointer.primary_down())
            && let Some(p) = ctx.input(|i| i.pointer.latest_pos())
        {
            let count = tr.pages.len();
            let title = format!("{} · {}", crate::i18n::trn(crate::i18n::current(), count as u64, "{n} page", "{n} pages"), tr.name);
            let page = tr.pages.first().copied().unwrap_or(0);
            self.drag_ghost = Some(crate::drag_ghost::Ghost { screen: p + screen_offset(ctx), title, doc: from, page, pages: count, preview: true });
        }
        if ctx.input(|i| i.pointer.primary_released())
            && let Some(p) = ctx.input(|i| i.pointer.latest_pos())
        {
            let screen = p + screen_offset(ctx);
            let remove = ctx.input(|i| i.modifiers.shift);
            let mut target: Option<(DocId, Option<usize>)> = None;
            for g in self.window_geometry.iter().filter(|g| g.window != self.current_window) {
                if let Some((doc, area, rows)) = &g.pages
                    && area.contains(screen)
                    && *doc != from
                {
                    target = Some((*doc, Some(gap_at(rows, screen.y))));
                    break;
                }
                if let Some((doc, _)) = g.tabs.iter().find(|(d, r)| r.contains(screen) && *d != from) {
                    target = Some((*doc, None));
                    break;
                }
            }
            if let Some((doc, at)) = target
                && let Some(index) = self.view_index(doc)
            {
                self.drop_page_transfer(index, at, remove);
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.page_transfer = None;
            self.tab_hover = None;
        }
    }

    /// Draw the torn-off windows (from the main window's frame). Only as the platform's own
    /// windows: drawn inside the main one (the web, tests), their tools would clash with its.
    pub(crate) fn show_doc_windows(&mut self, ctx: &Context) {
        // A window whose tabs have all gone closes.
        let views = &self.views;
        self.doc_windows.retain(|w| views.iter().any(|v| v.window == w.id));
        if ctx.embed_viewports() {
            return;
        }
        let ids: Vec<u64> = self.doc_windows.iter().map(|w| w.id).collect();
        for id in ids {
            let Some(w) = self.doc_windows.iter().find(|w| w.id == id) else { continue };
            let name = w
                .active
                .and_then(|d| self.session.get(d))
                .or_else(|| self.views.iter().find(|v| v.window == id).and_then(|v| self.session.get(v.id)))
                .map(|d| d.display_name());
            let title = name.map_or_else(|| "Sintec.PDF".to_owned(), |n| format!("{n} — Sintec.PDF"));
            let mut builder = egui::ViewportBuilder::default().with_title(title).with_inner_size([1100.0, 780.0]).with_min_inner_size([560.0, 380.0]);
            if let Some(p) = w.spawn_at {
                // Under the pointer, as if carried there by its tab.
                builder = builder.with_position(p - vec2(120.0, 18.0));
            }
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("sintec-doc-window", id)), builder, |ui, _class| self.doc_window_ui(id, ui));
            if let Some(w) = self.doc_windows.iter_mut().find(|w| w.id == id) {
                w.spawn_at = None;
            }
        }
    }

    /// One torn-off window: its tabs, the tools and panels for its shown tab, the page.
    fn doc_window_ui(&mut self, id: u64, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.close_doc_window(id);
            return;
        }
        let main_active = self.active;
        self.saved_main_active = Some(main_active);
        self.current_window = id;
        let wanted = self.doc_windows.iter().find(|w| w.id == id).and_then(|w| w.active);
        self.active = wanted
            .and_then(|d| self.views.iter().position(|v| v.id == d && v.window == id))
            .or_else(|| self.views.iter().position(|v| v.window == id));
        if self.active.is_some() {
            self.shortcuts(&ctx);
            if let Some(view) = self.active.and_then(|i| self.views.get_mut(i)) {
                view.objects.set_input_blocked(false);
            }
            let t = theme::Tokens::get(&ctx);
            chrome::tab_strip(self, ui);
            chrome::mode_bar(self, ui);
            // As the main window: the docking workspace lays out the tool panels, the inspector and
            // the page; Read mode shows the page with only the navigation panes.
            let shown = self.active.filter(|&i| self.views.get(i).is_some_and(|v| v.window == id));
            if shown.is_some() {
                chrome::right_rail(self, ui);
                if self.mode == Mode::Read && self.right.is_some() {
                    egui::Panel::right("read_right_panel")
                        .resizable(true)
                        .default_size(330.0)
                        .size_range(260.0..=520.0)
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| panels::right_panel(self, ui));
                }
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| {
                if let Some(i) = shown {
                    if self.mode == Mode::Read {
                        crate::canvas::document_area(self, i, ui);
                    } else {
                        crate::docking::show(self, ui);
                    }
                }
            });
            crate::widgets::page_cursor_only_on_the_page(&ctx);
            self.process_pending_edits();
            let overlay = self.dialog.is_some() || self.palette_open;
            if overlay && self.overlay_window.is_none() {
                self.overlay_window = Some(id);
            }
            if self.overlay_window == Some(id) {
                palette::show(self, &ctx);
                dialogs::show(self, &ctx);
            }
            self.finish_page_transfer_drop(&ctx);
            crate::drag_ghost::paint(self, &ctx);
            crate::widgets::hand_over_clickables(&ctx);
        }
        let shown = self.active.and_then(|i| self.views.get(i)).filter(|v| v.window == id).map(|v| v.id);
        if let (Some(d), Some(w)) = (shown, self.doc_windows.iter_mut().find(|w| w.id == id)) {
            w.active = Some(d);
        }
        let restored = self.saved_main_active.take().unwrap_or(main_active);
        self.active = restored.filter(|&i| self.views.get(i).is_some_and(|v| v.window == 0));
        self.current_window = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_follow_the_pointer() {
        let rows = [Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(10.0, 10.0)), Rect::from_min_size(Pos2::new(0.0, 20.0), vec2(10.0, 10.0))];
        assert_eq!(gap_at(&rows, -5.0), 0);
        assert_eq!(gap_at(&rows, 12.0), 1);
        assert_eq!(gap_at(&rows, 99.0), 2);
        assert_eq!(gap_at(&[], 3.0), 0);
    }
}
