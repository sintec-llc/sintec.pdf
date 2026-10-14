//! Sintec.PDF: the Font substitution dialog. Text that needs glyphs the paragraph's font lacks
//! asks for a substitute instead of failing; cancelling leaves the paragraph as it was.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{BlockStyle, Edit};
use pdfcraft_ui_egui::PdfCraftApp;

/// One page with a Helvetica paragraph (no font program embedded).
fn fixture() -> Vec<u8> {
    let body = "BT /F1 24 Tf 20 150 Td (Invoice) Tj ET";
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

#[test]
fn text_the_font_cant_show_opens_the_substitution_dialog() {
    if !pdfcraft_fonts::CRAFT_FONTS.is_empty() {
        // With craft-fonts built in, the generated fallback shows the text and nothing is asked
        // unless an installed face is found first; that depends on the machine.
        return;
    }
    let mut builder = Harness::builder().with_size(egui::vec2(1200.0, 800.0));
    if std::env::var_os("SINTEC_SHOT").is_some() {
        builder = builder.wgpu();
    }
    let mut h = builder.build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("invoice.pdf", None, fixture()).expect("fixture opens");
        app
    });
    h.run_steps(3);
    let before = h.state().session.get(h.state().views[0].id).unwrap().text_blocks(0)[0].editor_text.clone();
    let edit = Edit::EditTextBlock { page: 0, block: 0, text: "Счёт Invoice".into(), style: BlockStyle::default() };
    // Not applied: it waits for a substitute.
    assert!(!h.state_mut().apply_edit(edit));
    h.run_steps(4);
    h.get_by_label("Font substitution");
    h.get_by_label("Keep the original font");
    if let Ok(path) = std::env::var("SINTEC_SHOT") {
        h.run_steps(30);
        if let Ok(image) = h.render() {
            image.save(path).unwrap();
        }
    }
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label("Font substitution").is_none());
    let after = h.state().session.get(h.state().views[0].id).unwrap().text_blocks(0)[0].editor_text.clone();
    assert_eq!(after, before);
}
