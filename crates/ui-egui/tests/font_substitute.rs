//! Sintec.PDF: the Font substitution dialog. Text that needs glyphs the paragraph's font lacks
//! asks for a substitute instead of failing; cancelling leaves the paragraph as it was.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::FontFamily;
use pdfcraft_engine::text_fonts::FontFile;
use pdfcraft_engine::{BlockStyle, Edit};
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

#[path = "../../engine/tests/support/font_fixture.rs"]
mod font_fixture;

/// One page with a Helvetica paragraph (no font program embedded).
fn fixture() -> Vec<u8> {
    paragraph_fixture("Invoice", "Helvetica")
}

/// One page with a paragraph `text` in the non-embedded Type 1 font `base_font`.
fn paragraph_fixture(text: &str, base_font: &str) -> Vec<u8> {
    let body = format!("BT /F1 24 Tf 20 150 Td ({text}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base_font} >>"),
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

/// An installed face as the system scan would report it, backed by `bytes` written to a file.
fn installed_face(name: &str, family: &str, face: &str, weight: u16, bytes: &[u8]) -> FontFile {
    let dir = std::env::temp_dir().join(format!("sintec-font-substitute-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    FontFile {
        path,
        family: family.into(),
        face: face.into(),
        style: pdfcraft_fonts::embedding::FaceStyle { weight, italic: false, monospace: false, serif: Some(false), cyrillic: true },
    }
}

fn app_with(bytes: Vec<u8>, faces: Vec<FontFile>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("paragraph.pdf", None, bytes).expect("fixture opens");
        app.fonts.installed_override = Some(faces);
        app
    })
}

/// Bold on a paragraph whose real family has an installed bold face is set in that face, not in
/// the standard Helvetica-Bold (which has no Cyrillic and isn't the document's font).
#[test]
fn bold_uses_the_installed_bold_face_of_the_paragraphs_family() {
    let faces = vec![installed_face("RectFont-Bold.otf", "Rect Font", "RectFont-Bold", 700, &font_fixture::cff_font_with_weight(700))];
    let mut h = app_with(paragraph_fixture("A A", "RectFont"), faces);
    h.run_steps(3);
    let style = BlockStyle { family: Some((FontFamily::Helvetica, true, false)), ..BlockStyle::default() };
    assert!(h.state_mut().apply_edit(Edit::EditTextBlock { page: 0, block: 0, text: "A A".into(), style }));
    let doc = h.state().session.get(h.state().views[0].id).unwrap();
    let lines = doc.text_lines(0);
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|l| l.base_font != "Helvetica-Bold"), "{lines:?}");
    assert!(lines.iter().all(|l| l.base_font == "RectFont-Regular"), "the loaded face draws it: {lines:?}");
    assert!(doc.text_editor_fonts(0).iter().any(|f| f.resource.starts_with("PCFull")));
}

/// Editing text shows a font that has no similar installed face in the Font substitution
/// dialog once; dismissing it doesn't bring it back.
#[test]
fn edit_pdf_suggests_a_font_with_no_similar_installed_face_once() {
    let faces = vec![installed_face("Other.otf", "Other Sans", "OtherSans-Regular", 400, &font_fixture::cff_font())];
    let mut h = app_with(paragraph_fixture("A A", "Zapfino"), faces);
    h.run_steps(3);
    assert!(h.query_by_label("Font substitution").is_none(), "not while reading");
    h.state_mut().quick_tool = QuickTool::EditText;
    h.run_steps(4);
    h.get_by_label("Font substitution");
    h.get_by_label("Cancel").click();
    h.run_steps(6);
    assert!(h.query_by_label("Font substitution").is_none(), "asked once per font");
}
