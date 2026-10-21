//! Task 013: the paragraph box's width handles in the real code paths: under the Format text
//! panel, on quarter-turned pages, at several zoom levels, and what the drag leaves behind.
use egui::Pos2;
use egui_kittest::Harness;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

fn fixture() -> Vec<u8> {
    let body = "BT /F1 10 Tf 12 TL 150 200 Td (a b c d e f g h i j) Tj T* (a b c d e f g h i j) Tj T* (a b c d e f g h i j) Tj T* (a b c d e f g h i j) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
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

fn standard_faces() -> Vec<pdfcraft_engine::text_fonts::FontFile> {
    ["Arial", "Times New Roman", "Courier New"]
        .map(|family| pdfcraft_engine::text_fonts::FontFile {
            path: std::path::PathBuf::from(format!("/nonexistent/{family}.ttf")),
            family: family.into(),
            face: family.replace(' ', ""),
            style: pdfcraft_fonts::embedding::FaceStyle {
                weight: 400,
                italic: false,
                monospace: family == "Courier New",
                serif: Some(family == "Times New Roman"),
                cyrillic: true,
            },
            widths: Vec::new(),
            bundled: false,
        })
        .to_vec()
}

fn harness(rotation: i64, zoom: Option<f32>) -> Harness<'static, PdfCraftApp> {
    harness_sized(rotation, zoom, 1200.0)
}

fn harness_sized(rotation: i64, zoom: Option<f32>, width: f32) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_eframe(move |_cc| {
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
    if let Some(z) = zoom {
        h.state_mut().views[0].set_zoom(z);
        h.run_steps(4);
    }
    assert!(h.state_mut().execute("edit.edit_text"));
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    for _ in 0..60 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn block(h: &Harness<'static, PdfCraftApp>) -> pdfcraft_engine::TextBlock {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().text_blocks(0).remove(0)
}

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

fn open_editor(h: &mut Harness<'static, PdfCraftApp>, b: egui::Rect) {
    let p = b.center();
    h.hover_at(p);
    h.run_steps(2);
    h.drag_at(p);
    h.drop_at(p);
    h.run_steps(6);
    assert!(h.state().views[0].line_editor.is_some(), "the click opens the editor");
}

fn width_of(b: &pdfcraft_engine::TextBlock) -> f64 {
    b.rect[2] - b.rect[0]
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
    let (before, b) = (block(&h), screen_box(&h, rotation));
    if open {
        open_editor(&mut h, b);
    }
    let (from, dir) = handle_point(b, rotation, 1, 8.0);
    press(&mut h, from);
    release(&mut h, from + dir * 100.0);
    let after = block(&h);
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
fn right_handle_of_an_open_box_under_the_format_panel() {
    for zoom in [None, Some(1.0f32), Some(1.5), Some(2.5)] {
        let mut h = harness(0, zoom);
        let (before, b) = (block(&h), screen_box(&h, 0));
        open_editor(&mut h, b);
        let panel = h.state().views[0].text_panel_rect().expect("the Format text panel is up");
        let (from, dir) = handle_point(b, 0, 1, 6.0);
        press(&mut h, from);
        release(&mut h, from + dir * 100.0);
        let after = block(&h);
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
        let (before, b) = (block(&h), screen_box(&h, rotation));
        let (from, dir) = handle_point(b, rotation, 0, 8.0);
        press(&mut h, from);
        release(&mut h, from - dir * 40.0);
        let after = block(&h);
        assert!(width_of(&after) < width_of(&before) - 1.0, "rot {rotation}: {:?} -> {:?}", before.rect, after.rect);
        assert!(after.rows.len() >= before.rows.len());
    }
}

#[test]
fn a_dragged_width_survives_undo_redo_save_and_reopen() {
    let mut h = harness(0, Some(1.5));
    let (before, b) = (block(&h), screen_box(&h, 0));
    let (from, dir) = handle_point(b, 0, 1, 8.0);
    press(&mut h, from);
    release(&mut h, from - dir * 60.0);
    let narrowed = block(&h);
    assert!(narrowed.rows.len() > before.rows.len(), "the text reflows: {:?} -> {:?}", before.rows, narrowed.rows);
    assert!(h.state_mut().execute("edit.undo"));
    settle(&mut h);
    assert_eq!(block(&h).rows.len(), before.rows.len());
    assert!(h.state_mut().execute("edit.redo"));
    settle(&mut h);
    assert_eq!(block(&h).rows.len(), narrowed.rows.len());
    let id = h.state().views[0].id;
    let saved = h.state_mut().session.save_bytes(id).unwrap();
    let mut s = pdfcraft_engine::Session::new();
    let rid = s.open("saved.pdf", None, saved, None).unwrap();
    let again = s.get(rid).unwrap().text_blocks(0).remove(0);
    assert_eq!(again.rows.len(), narrowed.rows.len());
    assert!((width_of(&again) - width_of(&narrowed)).abs() < 1.0);
}

/// A window so narrow that the panel covers the box on either side: the handle still wins the press.
#[test]
fn the_handle_wins_over_the_panel_where_it_cannot_keep_clear() {
    let mut h = harness_sized(0, None, 640.0);
    let (before, b) = (block(&h), screen_box(&h, 0));
    open_editor(&mut h, b);
    let panel = h.state().views[0].text_panel_rect().expect("the Format text panel is up");
    let (from, dir) = handle_point(b, 0, 1, 6.0);
    assert!(panel.contains(from), "the handle is under the panel: {panel:?} {from:?}");
    press(&mut h, from);
    release(&mut h, from - dir * 30.0);
    let after = block(&h);
    assert!(width_of(&after) < width_of(&before) - 1.0 || after.rows.len() > before.rows.len(), "{:?} -> {:?}", before.rect, after.rect);
}
