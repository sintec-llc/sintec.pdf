//! The paragraph box's width handles in the real code paths: beside the Format text panel, on
//! quarter-turned pages, at several zoom levels, and what the drag leaves behind (task 013).

use egui::Pos2;
use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

use super::{first_block, open_editor_at, page_pdf, press, release, screen_rect, width_of};
use crate::support::settle;

/// A 300 × 300 page with one four-row Courier paragraph.
fn fixture() -> Vec<u8> {
    let row = "(a b c d e f g h i j) Tj";
    page_pdf(300, 300, "Courier", &format!("BT /F1 10 Tf 12 TL 150 200 Td {row} T* {row} T* {row} T* {row} ET"))
}

fn harness(rotation: i64, zoom: Option<f32>) -> Harness<'static, PdfCraftApp> {
    super::harness(fixture(), 1200.0, rotation, zoom)
}

fn screen_box(h: &Harness<'static, PdfCraftApp>, rotation: i64) -> egui::Rect {
    screen_rect(h, 300.0, 300.0, rotation, super::shown(h, "a b c"))
}

fn scale(h: &Harness<'static, PdfCraftApp>) -> f32 {
    h.state().views[0].page_screen_rect(0).unwrap().width() / 300.0
}

/// A point `out` px outside the paragraph's side (0 = its left, 1 = its right) of the screen box,
/// and the screen direction pointing outward from it.
fn handle_point(b: egui::Rect, rotation: i64, side: usize, out: f32) -> (Pos2, egui::Vec2) {
    let (on_x, positive) = match (rotation, side) {
        (0, 0) => (true, false),
        (0, _) => (true, true),
        (180, 0) => (true, true),
        (180, _) => (true, false),
        (90, 0) => (false, false),
        (90, _) => (false, true),
        (_, 0) => (false, true),
        _ => (false, false),
    };
    let sign = if positive { 1.0 } else { -1.0 };
    if on_x {
        let x = if positive { b.right() } else { b.left() };
        (Pos2::new(x + sign * out, b.center().y), egui::vec2(sign, 0.0))
    } else {
        let y = if positive { b.bottom() } else { b.top() };
        (Pos2::new(b.center().x, y + sign * out), egui::vec2(0.0, sign))
    }
}

/// Dragging the right handle outward by 100 px widens the paragraph by 100/scale points.
fn widen_right(rotation: i64, zoom: Option<f32>, open: bool) {
    let mut h = harness(rotation, zoom);
    let (before, b) = (first_block(&h), screen_box(&h, rotation));
    if open {
        open_editor_at(&mut h, b.center());
    }
    let (from, dir) = handle_point(b, rotation, 1, 8.0);
    press(&mut h, from);
    release(&mut h, from + dir * 100.0);
    let after = first_block(&h);
    let want = 100.0 / scale(&h) as f64;
    let got = width_of(&after) - width_of(&before);
    // The text wraps at word boundaries (a word and its space are 12 pt here).
    assert!(
        got > 0.0 && got <= want + 1.5 && got >= want - 13.0,
        "rot {rotation} zoom {zoom:?} open {open}: widened by {got}, wanted {want}: {:?} -> {:?}",
        before.rect,
        after.rect
    );
}

#[test]
fn right_handle_widens_at_several_zooms() {
    for zoom in [0.5f32, 1.0, 1.5] {
        widen_right(0, Some(zoom), false);
    }
}

#[test]
fn right_handle_of_an_open_box_beside_the_format_panel() {
    for zoom in [None, Some(1.0f32), Some(1.5), Some(2.5)] {
        let mut h = harness(0, zoom);
        let (before, b) = (first_block(&h), screen_box(&h, 0));
        open_editor_at(&mut h, b.center());
        h.run_steps(2);
        let panel = h.state().views[0].text_panel_rect().expect("the Format text panel is up");
        let (from, dir) = handle_point(b, 0, 1, 6.0);
        assert!(!panel.contains(from), "the panel keeps clear of the handle: zoom {zoom:?}, panel {panel:?}, box {b:?}");
        press(&mut h, from);
        release(&mut h, from + dir * 100.0);
        let after = first_block(&h);
        let want = 100.0 / scale(&h) as f64;
        let got = width_of(&after) - width_of(&before);
        assert!(got > 0.0 && got <= want + 1.5 && got >= want - 13.0, "zoom {zoom:?}, panel {panel:?}, box {b:?}: widened by {got}, wanted {want}");
    }
}

#[test]
fn width_handles_work_on_quarter_turned_pages() {
    for rotation in [90i64, 270, 180] {
        for open in [false, true] {
            widen_right(rotation, Some(1.0), open);
        }
        // And the left one, narrowing: the text rewraps into more rows.
        let mut h = harness(rotation, Some(1.0));
        let (before, b) = (first_block(&h), screen_box(&h, rotation));
        let (from, dir) = handle_point(b, rotation, 0, 8.0);
        press(&mut h, from);
        release(&mut h, from - dir * 40.0);
        let after = first_block(&h);
        assert!(width_of(&after) < width_of(&before) - 1.0, "rot {rotation}: {:?} -> {:?}", before.rect, after.rect);
        assert!(after.rows.len() >= before.rows.len());
    }
}

#[test]
fn a_dragged_width_survives_undo_redo_save_and_reopen() {
    let mut h = harness(0, Some(1.5));
    let (before, b) = (first_block(&h), screen_box(&h, 0));
    let (from, dir) = handle_point(b, 0, 1, 8.0);
    press(&mut h, from);
    release(&mut h, from - dir * 60.0);
    let narrowed = first_block(&h);
    assert!(narrowed.rows.len() > before.rows.len(), "the text reflows: {:?} -> {:?}", before.rows, narrowed.rows);
    assert!(h.state_mut().execute("edit.undo"));
    settle(&mut h);
    assert_eq!(first_block(&h).rows.len(), before.rows.len());
    assert!(h.state_mut().execute("edit.redo"));
    settle(&mut h);
    assert_eq!(first_block(&h).rows.len(), narrowed.rows.len());
    let id = h.state().views[0].id;
    let saved = h.state_mut().session.save_bytes(id).unwrap();
    let mut s = pdfcraft_engine::Session::new();
    let rid = s.open("saved.pdf", None, saved, None).unwrap();
    let again = s.get(rid).unwrap().text_blocks(0).remove(0);
    assert_eq!(again.rows.len(), narrowed.rows.len());
    assert!((width_of(&again) - width_of(&narrowed)).abs() < 1.0);
}
