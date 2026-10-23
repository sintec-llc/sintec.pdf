//! Sintec.PDF: the Font substitution dialog. Text that needs glyphs the paragraph's font lacks
//! asks for a substitute instead of failing; cancelling leaves the paragraph as it was.

#[path = "support/render.rs"]
mod render_lock;
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
        // No installed face to substitute with: the dialog asks, whatever this machine has.
        app.fonts.installed_override = Some(Vec::new());
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
        if let Ok(image) = render_lock::locked(|| h.render()) {
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
        bundled: false,
        style: pdfcraft_fonts::embedding::FaceStyle { weight, italic: false, monospace: false, serif: Some(false), cyrillic: true },
        widths: Vec::new(),
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

/// Open the paragraph of `paragraph_fixture` in Edit PDF (dismissing a font suggestion first).
fn open_paragraph(h: &mut Harness<'static, PdfCraftApp>) {
    h.run_steps(3);
    assert!(h.state_mut().execute("edit.edit_text"));
    h.run_steps(4);
    if h.query_by_label("Font substitution").is_some() {
        h.get_by_label("Cancel").click();
        h.run_steps(4);
    }
    let r = h.state().views[0].page_screen_rect(0).expect("on screen");
    let at = egui::pos2(r.left() + 40.0 / 300.0 * r.width(), r.top() + (300.0 - 158.0) / 300.0 * r.height());
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(4);
    assert!(h.state().views[0].line_editor.is_some(), "the paragraph opens");
    assert!(h.state().views[0].text_panel_rect().is_some(), "with the floating panel");
}

/// The floating panel says whether this computer has the paragraph's font, from the same list
/// of installed faces the Font substitution dialog uses, and lists those faces.
#[test]
fn the_format_panel_says_whether_the_font_is_installed_and_lists_installed_faces() {
    let other = installed_face("Other.otf", "Other Sans", "OtherSans-Regular", 400, &font_fixture::cff_font());
    let mut h = app_with(paragraph_fixture("Invoice", "RectFont"), vec![other.clone()]);
    open_paragraph(&mut h);
    h.get_by_label_contains("RectFont — not embedded, not installed");
    let combo = h.query_by(|n| [n.label(), n.value()].iter().any(|t| t.as_deref() == Some("Installed fonts…")));
    // The face for missing characters is named once (its status line), then in the list too.
    let before = h.get_all_by_label_contains("Other Sans").count();
    combo.expect("the Installed fonts list").click();
    h.run_steps(3);
    assert!(h.get_all_by_label_contains("Other Sans").count() > before);

    let rect = installed_face("RectFont.otf", "Rect Font", "RectFont-Regular", 400, &font_fixture::cff_font());
    let mut h = app_with(paragraph_fixture("Invoice", "RectFont"), vec![other, rect]);
    open_paragraph(&mut h);
    h.get_by_label_contains("RectFont — not embedded, installed");
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

/// Sintec.PDF (task 027): a letter added to a bold heading, which its font can't show, is drawn in
/// the installed bold face of the heading's family, chosen without asking; the panel says so.
#[test]
fn a_bold_heading_gets_the_installed_bold_face_for_missing_letters() {
    let chars = [' ', 'A', 'Ж'];
    let faces = vec![
        installed_face(
            "Heading-Regular.otf",
            "Rect Font",
            "RectFont-Regular",
            400,
            &font_fixture::cff_font_styled("RectFont-Regular", "Regular", 400, 700, &chars),
        ),
        installed_face(
            "Heading-Bold.otf",
            "Rect Font",
            "RectFont-Bold",
            700,
            &font_fixture::cff_font_styled("RectFont-Bold", "Bold", 700, 700, &chars),
        ),
    ];
    let mut h = app_with(paragraph_fixture("A A", "RectFont-Bold"), faces);
    open_paragraph(&mut h);
    // The panel: the detected style, and the face for missing characters.
    h.get_by_label_contains("RectFont Bold · 24.0 pt · #000000");
    h.get_by_label_contains("Missing characters: Rect Font Bold — installed");
    let edit = Edit::EditTextBlock { page: 0, block: 0, text: "A Ж".into(), style: BlockStyle::default() };
    assert!(h.state_mut().apply_edit(edit), "applied without asking");
    h.run_steps(3);
    assert!(h.query_by_label("Font substitution").is_none());
    let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
    assert!(lines.iter().any(|l| l.text.contains('Ж') && l.base_font == "RectFont-Bold"), "{lines:?}");
}

/// Sintec.PDF (task 027): with the family not installed, its metric-compatible face that comes
/// with the app (Carlito for Calibri) draws the missing letters in the font's weight, without
/// asking.
#[test]
fn a_metric_compatible_bundled_face_draws_missing_letters_without_asking() {
    let chars = [' ', 'A', 'Ж'];
    let mut carlito =
        installed_face("Carlito-Bold.otf", "Carlito", "Carlito-Bold", 700, &font_fixture::cff_font_styled("Carlito-Bold", "Bold", 700, 700, &chars));
    carlito.bundled = true;
    let mut regular = installed_face(
        "Carlito-Regular.otf",
        "Carlito",
        "Carlito-Regular",
        400,
        &font_fixture::cff_font_styled("Carlito-Regular", "Regular", 400, 700, &chars),
    );
    regular.bundled = true;
    let mut h = app_with(paragraph_fixture("A A", "Calibri-Bold"), vec![regular, carlito]);
    h.run_steps(3);
    let edit = Edit::EditTextBlock { page: 0, block: 0, text: "A Ж".into(), style: BlockStyle::default() };
    assert!(h.state_mut().apply_edit(edit), "applied without asking");
    let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
    assert!(lines.iter().any(|l| l.text.contains('Ж') && l.base_font == "Carlito-Bold"), "{lines:?}");
}

/// Sintec.PDF (task 027): while a paragraph whose embedded subset lacks letters is open, those
/// letters are previewed in the font's installed stand-in (here the bold face of its family), not
/// in the interface's own face: the stand-in follows the subset in the editor's font stack.
#[test]
fn the_open_paragraph_previews_missing_letters_in_the_stand_in() {
    // A paragraph set in an embedded bold font that maps only "A" and a space.
    let mut s = pdfcraft_engine::Session::new();
    let id = s.open("p.pdf", None, std::sync::Arc::new(paragraph_fixture("A A", "Helvetica")), None).unwrap();
    let subset = font_fixture::cff_font_styled("RectFont-Bold", "Bold", 700, 700, &[' ', 'A']);
    s.apply(id, Edit::RegisterTextFont { page: 0, bytes: std::sync::Arc::new(subset.clone()) }).unwrap();
    let resource = s.get(id).unwrap().text_editor_fonts(0).into_iter().find(|f| f.bytes == subset).unwrap().resource;
    let style = BlockStyle { resource: Some(resource), ..BlockStyle::default() };
    s.apply(id, Edit::EditTextBlock { page: 0, block: 0, text: "A A".into(), style }).unwrap();
    let bytes = s.save_bytes(id).unwrap().to_vec();
    let chars = [' ', 'A', 'Ж'];
    let faces = vec![
        installed_face(
            "Preview-Regular.otf",
            "Rect Font",
            "RectFont-Regular",
            400,
            &font_fixture::cff_font_styled("RectFont-Regular", "Regular", 400, 700, &chars),
        ),
        installed_face(
            "Preview-Bold.otf",
            "Rect Font",
            "RectFont-Bold",
            700,
            &font_fixture::cff_font_styled("RectFont-Bold", "Bold", 700, 700, &chars),
        ),
    ];
    let mut h = app_with(bytes, faces);
    open_paragraph(&mut h);
    h.run_steps(4);
    let stacks: Vec<Vec<String>> = h.ctx.fonts(|f| {
        f.definitions()
            .families
            .iter()
            .filter(|(family, _)| matches!(family, egui::FontFamily::Name(n) if n.starts_with("pdf-source-")))
            .map(|(_, stack)| stack.clone())
            .collect()
    });
    assert!(!stacks.is_empty(), "the subset is in the editor's fonts");
    assert!(stacks.iter().all(|s| s.get(1).is_some_and(|n| n.starts_with("pdf-substitute-"))), "{stacks:?}");
    let bold = h.ctx.fonts(|f| f.definitions().font_data.get("pdf-substitute-0").map(|d| d.font.to_vec()));
    assert_eq!(bold.as_deref(), Some(font_fixture::cff_font_styled("RectFont-Bold", "Bold", 700, 700, &chars).as_slice()), "the bold face");
}
