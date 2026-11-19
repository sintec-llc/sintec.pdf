//! The paragraph box's move grip and width handles: reachable from the box, held until release,
//! usable while the box is open for typing and on rotated pages (tasks 003, 011).

use egui::Pos2;
use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

use super::{first_block, open_editor_at, page_pdf, press, release, screen_rect, width_of};

/// A 300 × 300 page with one two-row Helvetica paragraph.
fn fixture() -> Vec<u8> {
    page_pdf(300, 300, "Helvetica", "BT /F1 14 Tf 16 TL 60 200 Td (The quick brown fox jumps) Tj T* (over the lazy dog again) Tj ET")
}

fn harness(rotation: i64) -> Harness<'static, PdfCraftApp> {
    super::harness(fixture(), 1200.0, rotation, None)
}

/// The paragraph's box on screen. Its frame is its ink here: it is the page's only paragraph and
/// wraps at its own width.
fn screen_box(h: &Harness<'static, PdfCraftApp>, rotation: i64) -> egui::Rect {
    screen_rect(h, 300.0, 300.0, rotation, super::shown(h, "The quick"))
}

fn scale(h: &Harness<'static, PdfCraftApp>) -> f32 {
    h.state().views[0].page_screen_rect(0).unwrap().width() / 300.0
}

/// The centre of the move grip (just above the box's top-left corner).
fn grip(b: egui::Rect) -> Pos2 {
    Pos2::new(b.left() + 9.0, b.top() - 10.0)
}

fn moved_by(before: &pdfcraft_engine::TextBlock, after: &pdfcraft_engine::TextBlock) -> [f64; 2] {
    [after.rect[0] - before.rect[0], after.rect[1] - before.rect[1]]
}

#[test]
fn the_grip_stays_grabbable_when_the_pointer_climbs_from_the_box() {
    let mut h = harness(0);
    let (before, b) = (first_block(&h), screen_box(&h, 0));
    // Hover the box, then step up onto the grip in small moves, then press and drag from there.
    h.hover_at(b.center());
    h.run_steps(2);
    for k in 0..=5 {
        h.hover_at(Pos2::new(b.left() + 9.0, b.top() + 4.0 - k as f32 * 3.0));
        h.run_steps(1);
    }
    press(&mut h, grip(b));
    release(&mut h, grip(b) + egui::vec2(40.0, 30.0));
    let d = moved_by(&before, &first_block(&h));
    let s = scale(&h) as f64;
    assert!((d[0] - 40.0 / s).abs() < 1.0 && (d[1] + 30.0 / s).abs() < 1.0, "moved by {d:?}");
}

#[test]
fn a_grip_drag_follows_the_pointer_to_release_wherever_it_goes() {
    let mut h = harness(0);
    let (before, b) = (first_block(&h), screen_box(&h, 0));
    press(&mut h, grip(b));
    // Far outside the box and the page; the grab holds.
    let far = grip(b) + egui::vec2(-20.0, -45.0);
    h.hover_at(far);
    h.run_steps(3);
    release(&mut h, far);
    let d = moved_by(&before, &first_block(&h));
    let s = scale(&h) as f64;
    assert!((d[0] + 20.0 / s).abs() < 1.0 && (d[1] - 45.0 / s).abs() < 1.0, "moved by {d:?}");
}

#[test]
fn each_side_handle_changes_the_width() {
    for (right, delta) in [(true, 150.0f32), (true, -100.0), (false, -150.0), (false, 100.0)] {
        let mut h = harness(0);
        let (before, b) = (first_block(&h), screen_box(&h, 0));
        // 8 px outside the box: on the handle, but off the text, where a press used to start a marquee.
        let from = Pos2::new(if right { b.right() + 8.0 } else { b.left() - 8.0 }, b.center().y);
        press(&mut h, from);
        release(&mut h, from + egui::vec2(delta, 0.0));
        let after = first_block(&h);
        let grew = (right && delta > 0.0) || (!right && delta < 0.0);
        if grew {
            assert!(width_of(&after) > width_of(&before) + 1.0, "right {right} delta {delta}: {:?} -> {:?}", before.rect, after.rect);
        } else {
            assert!(
                width_of(&after) < width_of(&before) - 1.0 || after.rows.len() > before.rows.len(),
                "right {right} delta {delta}: {:?} -> {:?}",
                before.rect,
                after.rect
            );
        }
    }
}

#[test]
fn grip_and_handles_work_while_the_box_is_open_for_typing() {
    let mut h = harness(0);
    let (before, b) = (first_block(&h), screen_box(&h, 0));
    open_editor_at(&mut h, b.center());
    // Width, from just outside the left edge (the Format text panel floats on the right).
    let from = Pos2::new(b.left() - 6.0, b.center().y);
    press(&mut h, from);
    assert!(h.state().views[0].line_editor.is_some(), "the press on a handle keeps the paragraph open");
    release(&mut h, from + egui::vec2(-150.0, 0.0));
    let wider = first_block(&h);
    assert!(width_of(&wider) > width_of(&before) + 1.0, "{:?} -> {:?}", before.rect, wider.rect);
    assert!(h.state().views[0].line_editor.is_none());
    // The grip, with the editor open again.
    let b = screen_box(&h, 0);
    open_editor_at(&mut h, b.center());
    press(&mut h, grip(b));
    release(&mut h, grip(b) + egui::vec2(0.0, 40.0));
    let d = moved_by(&wider, &first_block(&h));
    assert!((d[1] + 40.0 / scale(&h) as f64).abs() < 1.0, "moved by {d:?}");
}

#[test]
fn typing_goes_along_when_the_open_box_is_moved_by_its_grip() {
    let mut h = harness(0);
    let b = screen_box(&h, 0);
    open_editor_at(&mut h, b.center());
    h.state_mut().views[0].line_editor.as_mut().unwrap().text.push_str(" now");
    h.run_steps(2);
    press(&mut h, grip(b));
    release(&mut h, grip(b) + egui::vec2(0.0, 40.0));
    let after = first_block(&h);
    assert!(after.text.ends_with("again now"), "{:?}", after.text);
}

#[test]
fn the_grip_works_on_rotated_pages() {
    for rotation in [90i64, 180, 270] {
        let mut h = harness(rotation);
        let (before, b) = (first_block(&h), screen_box(&h, rotation));
        press(&mut h, grip(b));
        release(&mut h, grip(b) + egui::vec2(30.0, 40.0));
        let d = moved_by(&before, &first_block(&h));
        let s = scale(&h) as f64;
        let mut got = [d[0].abs() * s, d[1].abs() * s];
        got.sort_by(f64::total_cmp);
        assert!((got[0] - 30.0).abs() < 1.5 && (got[1] - 40.0).abs() < 1.5, "rotation {rotation}: moved by {d:?}");
    }
}
