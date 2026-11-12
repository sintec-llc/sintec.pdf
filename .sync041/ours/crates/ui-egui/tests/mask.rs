//! «Маска» (Скрыть данные ▸ Mask areas with white): the marked area is covered with white and
//! what was under it is gone from the file; the rest of the page is untouched; the redaction
//! properties are not changed.

use egui_kittest::Harness;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

fn page_text(app: &PdfCraftApp, bytes: std::sync::Arc<Vec<u8>>) -> String {
    let _ = app;
    let mut r = pdfcraft_render::PageRenderer::new(bytes, pdfcraft_render::RenderConfig::default());
    let t = r.render(pdfcraft_render::RenderRequest { page: 0, kind: pdfcraft_render::RequestKind::Text, scale: 1.0, ..Default::default() });
    t.text.map(|t| t.plain_text()).unwrap_or_default()
}

#[test]
fn a_white_mask_removes_what_is_under_it_for_good() {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        // Two lines of text: the first gets masked.
        let bytes = app.session.create_from_text("t", "SECRET passport 4512 001234\n\nPublic line stays").unwrap();
        app.open_bytes("doc.pdf", None, bytes.as_ref().clone()).unwrap();
        app
    });
    h.run_steps(3);
    let id = h.state().views[0].id;
    // Multi-line plain Latin text stays on US Letter in Helvetica (line breaks aren't characters).
    assert!((h.state().session.get(id).unwrap().info.pages[0].width - 612.0).abs() < 1.0);
    let before = page_text(h.state(), h.state().session.get(id).unwrap().bytes.clone());
    assert!(before.contains("SECRET") && before.contains("Public"), "{before}");

    assert!(h.state_mut().execute("redact.mask"));
    assert!(h.state().mask_mode);
    // The area over the first line (US Letter, 1-inch margins, 11 pt text near the top).
    let (w, top) = (612.0_f64, 792.0_f64 - 72.0);
    let quad = [72.0, top - 16.0, w - 72.0, top - 16.0, w - 72.0, top + 4.0, 72.0, top + 4.0];
    let mark = h.state().mark_prefs().mark(0, vec![quad], "tester");
    assert!(h.state_mut().apply_edit(mark));
    assert!(h.state_mut().apply_edit(Edit::ApplyRedactions { pages: None }));
    h.run_steps(2);

    let after_bytes = h.state().session.get(id).unwrap().bytes.clone();
    let after = page_text(h.state(), after_bytes.clone());
    assert!(!after.contains("SECRET") && !after.contains("4512"), "the masked text is gone from the file: {after}");
    assert!(after.contains("Public line stays"), "the rest of the page is untouched: {after}");
    // Not just hidden: the words are gone from the saved file too.
    let saved = h.state().session.save_bytes(id).unwrap();
    let saved_text = page_text(h.state(), saved.clone());
    assert!(!saved_text.contains("SECRET") && saved_text.contains("Public line stays"), "{saved_text}");
    // The area is white, and the mask is drawn into the page (no annotation left to delete).
    let mut r = pdfcraft_render::PageRenderer::new(saved, pdfcraft_render::RenderConfig::default());
    let px = r.render(pdfcraft_render::RenderRequest { page: 0, scale: 1.0, ..Default::default() });
    let at = |x: usize, y: usize| {
        let i = (y * px.width as usize + x) * 4;
        [px.rgba[i], px.rgba[i + 1], px.rgba[i + 2]]
    };
    assert_eq!(at(150, 80), [255, 255, 255], "white where the secret was");
    assert_eq!(h.state().session.get(id).unwrap().redaction_marks(), 0, "no mark annotation remains");
    // The redaction properties still say black.
    assert_eq!(h.state().redact_prefs.fill, Some([0.0, 0.0, 0.0]));
}
