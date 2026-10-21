//! The paragraph box's move grip and width handles (Edit PDF tool): reachable from the box, held
//! until release, usable while the box is open for typing and on rotated pages.
#[path = "support/mod.rs"]
mod support;
use egui::Pos2;
use egui_kittest::Harness;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;
use support::standard_faces;

fn fixture() -> Vec<u8> {
    let body = "BT /F1 14 Tf 16 TL 60 200 Td (The quick brown fox jumps) Tj T* (over the lazy dog again) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn harness(rotation: i64) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("p.pdf", None, fixture()).expect("fixture opens");
        app.fonts.installed_override = Some(standard_faces());
        if rotation != 0 {
            assert!(app.apply_edit(Edit::RotatePages { pages: vec![0], degrees: rotation }));
        }
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("edit.edit_text"));
    for _ in 0..60 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h
}

fn block(h: &Harness<'static, PdfCraftApp>) -> pdfcraft_engine::TextBlock {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().text_blocks(0).remove(0)
}

/// The paragraph's screen box (the page is 300×300 pt, no crop offset).
fn screen_box(h: &Harness<'static, PdfCraftApp>, rotation: i64) -> egui::Rect {
    let page = h.state().views[0].page_screen_rect(0).unwrap();
    let r = block(h).rect;
    let map = |x: f64, y: f64| -> Pos2 {
        let (u, v) = (x as f32 / 300.0, (300.0 - y as f32) / 300.0);
        let (a, b) = match rotation {
            90 => (1.0 - v, u),
            180 => (1.0 - u, 1.0 - v),
            270 => (v, 1.0 - u),
            _ => (u, v),
        };
        page.min + egui::vec2(a * page.width(), b * page.height())
    };
    egui::Rect::from_two_pos(map(r[0], r[1]), map(r[2], r[3])).expand(2.0)
}

fn scale(h: &Harness<'static, PdfCraftApp>) -> f32 {
    h.state().views[0].page_screen_rect(0).unwrap().width() / 300.0
}

fn press(h: &mut Harness<'static, PdfCraftApp>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(2);
    h.drag_at(p);
    h.run_steps(2);
}
fn release(h: &mut Harness<'static, PdfCraftApp>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(2);
    h.drop_at(p);
    h.run_steps(6);
}

/// The centre of the move grip (just above the box's top-left corner).
fn grip(b: egui::Rect) -> Pos2 {
    Pos2::new(b.left() + 9.0, b.top() - 10.0)
}

fn open_editor(h: &mut Harness<'static, PdfCraftApp>, b: egui::Rect) {
    let p = b.center();
    h.hover_at(p);
    h.run_steps(2);
    h.drag_at(p);
    h.drop_at(p);
    h.run_steps(6);
    assert!(h.state().views[0].line_editor.is_some(), "the click opens the editor");
}

fn moved_by(before: &pdfcraft_engine::TextBlock, after: &pdfcraft_engine::TextBlock) -> [f64; 2] {
    [after.rect[0] - before.rect[0], after.rect[1] - before.rect[1]]
}

#[test]
fn the_grip_stays_grabbable_when_the_pointer_climbs_from_the_box() {
    let mut h = harness(0);
    let (before, b) = (block(&h), screen_box(&h, 0));
    // Hover the box, then step up onto the grip in small moves, then press and drag from there.
    h.hover_at(b.center());
    h.run_steps(2);
    for k in 0..=5 {
        h.hover_at(Pos2::new(b.left() + 9.0, b.top() + 4.0 - k as f32 * 3.0));
        h.run_steps(1);
    }
    h.hover_at(grip(b));
    h.run_steps(2);
    h.drag_at(grip(b));
    h.run_steps(2);
    release(&mut h, grip(b) + egui::vec2(40.0, 30.0));
    let d = moved_by(&before, &block(&h));
    let s = scale(&h) as f64;
    assert!((d[0] - 40.0 / s).abs() < 1.0 && (d[1] + 30.0 / s).abs() < 1.0, "moved by {d:?}");
}

#[test]
fn a_grip_drag_follows_the_pointer_to_release_wherever_it_goes() {
    let mut h = harness(0);
    let (before, b) = (block(&h), screen_box(&h, 0));
    press(&mut h, grip(b));
    // Far outside the box and the page; the grab holds.
    let far = grip(b) + egui::vec2(-20.0, -45.0);
    h.hover_at(far);
    h.run_steps(3);
    release(&mut h, far);
    let d = moved_by(&before, &block(&h));
    let s = scale(&h) as f64;
    assert!((d[0] + 20.0 / s).abs() < 1.0 && (d[1] - 45.0 / s).abs() < 1.0, "moved by {d:?}");
}

fn width_of(b: &pdfcraft_engine::TextBlock) -> f64 {
    b.rect[2] - b.rect[0]
}

#[test]
fn each_side_handle_changes_the_width() {
    for (right, delta) in [(true, 150.0f32), (true, -100.0), (false, -150.0), (false, 100.0)] {
        let mut h = harness(0);
        let (before, b) = (block(&h), screen_box(&h, 0));
        // 8 px outside the box: on the handle, but off the text, where a press used to start a marquee.
        let from = Pos2::new(if right { b.right() + 8.0 } else { b.left() - 8.0 }, b.center().y);
        press(&mut h, from);
        release(&mut h, from + egui::vec2(delta, 0.0));
        let after = block(&h);
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
    let (before, b) = (block(&h), screen_box(&h, 0));
    open_editor(&mut h, b);
    // Width, from just outside the left edge (the Format text panel floats over the right one).
    let from = Pos2::new(b.left() - 6.0, b.center().y);
    press(&mut h, from);
    release(&mut h, from + egui::vec2(-150.0, 0.0));
    let wider = block(&h);
    assert!(width_of(&wider) > width_of(&before) + 1.0, "{:?} -> {:?}", before.rect, wider.rect);
    assert!(h.state().views[0].line_editor.is_none());
    // The grip, with the editor open again.
    let b = screen_box(&h, 0);
    open_editor(&mut h, b);
    press(&mut h, grip(b));
    release(&mut h, grip(b) + egui::vec2(0.0, 40.0));
    let d = moved_by(&wider, &block(&h));
    assert!((d[1] + 40.0 / scale(&h) as f64).abs() < 1.0, "moved by {d:?}");
}

#[test]
fn the_grip_works_on_rotated_pages() {
    for rotation in [90i64, 180, 270] {
        let mut h = harness(rotation);
        let (before, b) = (block(&h), screen_box(&h, rotation));
        press(&mut h, grip(b));
        release(&mut h, grip(b) + egui::vec2(30.0, 40.0));
        let d = moved_by(&before, &block(&h));
        let s = scale(&h) as f64;
        let mut got = [d[0].abs() * s, d[1].abs() * s];
        got.sort_by(f64::total_cmp);
        assert!((got[0] - 30.0).abs() < 1.5 && (got[1] - 40.0).abs() < 1.5, "rotation {rotation}: moved by {d:?}");
    }
}
