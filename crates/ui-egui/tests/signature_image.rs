//! Fill & Sign with a picture of a signature: Create signature ▸ Image ▸ Choose picture…
//! (a photo or scan on paper: the paper becomes transparent), placed where clicked, kept in
//! the settings; and the preview of a signature or stamp under the cursor while placing it.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::fill_sign::SavedSig;

/// A synthetic "scan": light grey paper with a dark blue stroke.
fn scan_png(dir: &std::path::Path) -> String {
    let mut img = image::RgbaImage::from_pixel(600, 240, image::Rgba([236, 234, 230, 255]));
    for x in 80..520 {
        for y in 110..126 {
            img.put_pixel(x, y, image::Rgba([25, 35, 130, 255]));
        }
    }
    let path = dir.join("signature-scan.png");
    img.save(&path).unwrap();
    path.to_string_lossy().into_owned()
}

fn click_at(h: &mut Harness<'static, PdfCraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(2);
    h.drag_at(at);
    h.drop_at(at);
    h.run_steps(4);
}

#[test]
fn a_picture_signature_is_prepared_placed_and_kept() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-sigimg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scan = scan_png(&dir);
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        let bytes = app.session.create_from_text("t", "Sign below").unwrap();
        app.open_bytes("contract.pdf", None, bytes.as_ref().clone()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().save_override = Some(scan);

    // No signature yet: the tool opens Create signature.
    assert!(h.state_mut().execute("sign.fill.signature"));
    h.run_steps(2);
    h.get_by_label("Image").click();
    h.run_steps(2);
    h.get_by_label("Choose picture…").click();
    h.run_steps(3);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let Some(SavedSig::Image(png)) = h.state().signature.clone() else { panic!("an image signature: {:?}", h.state().signature.is_some()) };
    let back = image::load_from_memory(&png).unwrap().to_rgba8();
    assert!(back.width() < 480 && back.get_pixel(0, 0).0[3] == 0, "trimmed, paper transparent: {}×{}", back.width(), back.height());

    // Hovering shows it; clicking places it there, 150 pt wide.
    let page = h.state().views[0].page_screen_rect(0).expect("page on screen");
    let at = page.min + egui::vec2(page.width() * 0.3, page.height() * 0.4);
    h.hover_at(at);
    h.run_steps(3);
    let id = h.state().views[0].id;
    let before = h.state().session.get(id).unwrap().info.annotations.len();
    click_at(&mut h, at);
    let annots = h.state().session.get(id).unwrap().info.annotations.clone();
    assert_eq!(annots.len(), before + 1, "one signature placed");
    let r = annots.last().unwrap().rect;
    assert!(((r[2] - r[0]) - 150.0).abs() < 1.0, "150 pt wide: {r:?}");

    // Kept in the settings like typed and drawn signatures.
    let json = h.state().persist();
    let mut other = PdfCraftApp::new();
    other.restore(&json);
    assert_eq!(other.signature, Some(SavedSig::Image(png)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_pictures_are_refused_with_a_message() {
    let mut app = PdfCraftApp::new();
    app.signature_draft = pdfcraft_ui_egui::fill_sign::SigDraft::new(false, "");
    app.use_signature_picture("blank.png", &{
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(40, 40, image::Rgba([255, 255, 255, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    });
    assert!(app.signature_draft.image.is_none());
    app.use_signature_picture("notes.txt", b"hello");
    assert!(app.signature_draft.image.is_none());
    // A settings file with a broken picture doesn't bring it back.
    app.restore(r#"{"signature_image": {"Image": "bm90IGEgcG5n"}}"#);
    assert!(app.signature.is_none());
}
