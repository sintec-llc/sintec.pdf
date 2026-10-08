//! Stamps and signatures on pages shown rotated (`/Rotate`) read upright and at their size, as
//! on any page: the stamp appearance turns with the page, signatures are laid out as shown.

use egui_kittest::Harness;
use pdfcraft_engine::{Edit, StampKind};
use pdfcraft_ui_egui::fill_sign::{FillTool, SavedSig};
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

fn click(h: &mut Harness<'static, PdfCraftApp>, fx: f32, fy: f32) {
    let page = h.state().views[0].page_screen_rect(0).unwrap();
    let at = page.min + egui::vec2(page.width() * fx, page.height() * fy);
    h.hover_at(at);
    h.run_steps(2);
    h.drag_at(at);
    h.drop_at(at);
    h.run_steps(3);
}

/// The bounding box (pixels) of the rendered pixels matching `pick`.
fn bbox(px: &pdfcraft_render::RenderedPage, pick: impl Fn([u8; 4]) -> bool) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for y in 0..px.height {
        for x in 0..px.width {
            let i = ((y * px.width + x) * 4) as usize;
            if pick([px.rgba[i], px.rgba[i + 1], px.rgba[i + 2], px.rgba[i + 3]]) {
                b = Some(b.map_or([x, y, x, y], |b| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]));
            }
        }
    }
    b
}

#[test]
fn stamps_and_signatures_read_upright_on_rotated_pages() {
    for rot in [90i64, 180, 270] {
        let mut h = Harness::builder().with_size(egui::vec2(1300.0, 1000.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            let bytes = app.session.create_from_text("t", " ").unwrap();
            app.open_bytes("d.pdf", None, bytes.as_ref().clone()).unwrap();
            assert!(app.apply_edit(Edit::RotatePages { pages: vec![0], degrees: rot }));
            app
        });
        h.run_steps(6);
        h.state_mut().quick_tool = QuickTool::Stamp(StampKind::Approved);
        click(&mut h, 0.4, 0.2);
        h.state_mut().signature = Some(SavedSig::Drawn(vec![vec![[0.0, 0.1], [0.5, 0.2], [1.0, 0.1]]]));
        h.state_mut().quick_tool = QuickTool::Fill(FillTool::Signature);
        click(&mut h, 0.2, 0.7);
        let id = h.state().views[0].id;
        assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 2, "rot {rot}");
        let saved = h.state().session.save_bytes(id).unwrap();
        let mut r = pdfcraft_render::PageRenderer::new(saved, pdfcraft_render::RenderConfig::default());
        let px = r.render(pdfcraft_render::RenderRequest { page: 0, scale: 1.0, ..Default::default() });
        // The green stamp: as wide as it is placed (about 3:1), not turned on its side.
        let s = bbox(&px, |p| p[1] > 100 && p[0] < 80 && p[2] < 80).expect("the stamp is drawn");
        let (w, h_) = (s[2] - s[0], s[3] - s[1]);
        assert!(w > h_ * 2, "rot {rot}: stamp {w}×{h_} px");
        // The drawn signature (black strokes) runs across the page as shown.
        let g = bbox(&px, |p| p[0] < 60 && p[1] < 60 && p[2] < 60 && p[3] > 0).expect("the signature is drawn");
        assert!(g[2] - g[0] > (g[3] - g[1]) * 2, "rot {rot}: signature {:?}", g);
    }
}
