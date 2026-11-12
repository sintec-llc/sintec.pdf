//! Sintec.PDF: the paragraph box in Edit PDF, driven through the real page input: its move grip
//! and width handles (tasks 003, 011, 013), the width they set kept for later rewrites (014), and
//! the box as the boundary where typed text wraps (015).
//!
//! Every page here is built from Type1 standard-14 fonts that the PDF doesn't embed (the
//! renderer's own faces and the engine's metrics, the same on every machine), with
//! `standard_faces` standing in for the machine's installed fonts, and every point is computed
//! from the page's own geometry (task 020: no test depends on the fonts or the screen it runs on).

use egui::Pos2;
use egui_kittest::Harness;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::{settle, standard_faces};

mod frames;
mod grip;
mod width;

/// A one-page PDF of `width` × `height` points drawing `body` with `/F1`, the standard font
/// `font`, which the file doesn't embed.
fn page_pdf(width: u32, height: u32, font: &str, body: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 {width} {height}] >>"),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{font} >>"),
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

/// The app in a `width` × 900 window with `pdf` open, its first page turned by `rotation`, at
/// `zoom` (or the default), in Edit text with every page drawn.
fn harness(pdf: Vec<u8>, width: f32, rotation: i64, zoom: Option<f32>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("p.pdf", None, pdf).expect("fixture opens");
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

/// The first paragraph on the first page, as the document has it.
fn first_block(h: &Harness<'static, PdfCraftApp>) -> pdfcraft_engine::TextBlock {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().text_blocks(0).remove(0)
}

/// User space → screen on the first page (`page_w` × `page_h` points, no crop offset), turned by
/// `rotation`.
fn to_screen(h: &Harness<'static, PdfCraftApp>, page_w: f64, page_h: f64, rotation: i64, x: f64, y: f64) -> Pos2 {
    let page = h.state().views[0].page_screen_rect(0).expect("the page is on screen");
    let (u, v) = ((x / page_w) as f32, ((page_h - y) / page_h) as f32);
    let (a, b) = match rotation {
        90 => (1.0 - v, u),
        180 => (1.0 - u, 1.0 - v),
        270 => (v, 1.0 - u),
        _ => (u, v),
    };
    page.min + egui::vec2(a * page.width(), b * page.height())
}

/// A user-space rect on screen, with the 2 px of room the drawn box has around it.
fn screen_rect(h: &Harness<'static, PdfCraftApp>, page_w: f64, page_h: f64, rotation: i64, r: [f64; 4]) -> egui::Rect {
    egui::Rect::from_two_pos(to_screen(h, page_w, page_h, rotation, r[0], r[1]), to_screen(h, page_w, page_h, rotation, r[2], r[3])).expand(2.0)
}

/// The box shown for the paragraph starting with `prefix` (user space).
fn shown(h: &Harness<'static, PdfCraftApp>, prefix: &str) -> [f64; 4] {
    let boxes = h.state().views[0].shown_text_boxes(0);
    boxes.iter().find(|(t, _)| t.starts_with(prefix)).unwrap_or_else(|| panic!("no box {prefix}: {boxes:?}")).1
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
    h.run_steps(2);
    settle(h);
}

/// Click at `p`, which opens the paragraph there for typing.
fn open_editor_at(h: &mut Harness<'static, PdfCraftApp>, p: Pos2) {
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
