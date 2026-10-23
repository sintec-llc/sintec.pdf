//! Document font selection and contextual glyph save/reopen acceptance on synthetic PDFs.
mod support {
    pub mod font_fixture;
}
use pdfcraft_cos::{Dict, Document, Object, SaveOptions, Stream};
use pdfcraft_engine::{BlockStyle, Edit, Session};
use pdfcraft_render::{PageRenderer, RenderRequest};
use std::sync::Arc;
fn document() -> Arc<Vec<u8>> {
    let mut d = Document::new_empty();
    let pages = d.add(Object::Dict(Dict::new()));
    let contents = d.add(Object::Stream(Stream::flate(Dict::new(), b"BT /F1 18 Tf 30 160 Td (Source) Tj ET")));
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("Type1"));
    font.set(b"BaseFont".to_vec(), Object::name("Helvetica"));
    let mut fonts = Dict::new();
    fonts.set(b"F1".to_vec(), Object::Dict(font));
    let mut res = Dict::new();
    res.set(b"Font".to_vec(), Object::Dict(fonts));
    let mut p = Dict::new();
    p.set(b"Type".to_vec(), Object::name("Page"));
    p.set(b"Parent".to_vec(), Object::Ref(pages));
    p.set(b"Contents".to_vec(), Object::Ref(contents));
    p.set(b"Resources".to_vec(), Object::Dict(res));
    p.set(b"MediaBox".to_vec(), Object::Array([0, 0, 300, 240].map(Object::Int).to_vec()));
    let page = d.add(Object::Dict(p));
    d.update_dict(pages, |p| {
        p.set(b"Type".to_vec(), Object::name("Pages"));
        p.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(page)]));
        p.set(b"Count".to_vec(), Object::Int(1));
    })
    .unwrap();
    let mut root = Dict::new();
    root.set(b"Type".to_vec(), Object::name("Catalog"));
    root.set(b"Pages".to_vec(), Object::Ref(pages));
    let root = d.add(Object::Dict(root));
    d.trailer_mut().set(b"Root".to_vec(), Object::Ref(root));
    Arc::new(pdfcraft_cos::write_full(&d, &SaveOptions::default()).unwrap())
}
fn pixels(bytes: Arc<Vec<u8>>) -> Vec<u8> {
    let mut renderer = PageRenderer::new(bytes, Default::default());
    let p = renderer.render(RenderRequest { page: 0, scale: 2.0, ..Default::default() });
    assert!(p.error.is_none(), "{:?}", p.error);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    p.rgba.to_vec()
}
fn exercise(font: Vec<u8>, text: &str, expected_family: &str) {
    let mut s = Session::new();
    let id = s.open("source.pdf", None, document(), None).unwrap();
    let original = pixels(s.get(id).unwrap().display.clone());
    s.apply(id, Edit::RegisterTextFont { page: 0, bytes: Arc::new(font.clone()) }).unwrap();
    let resource = s.get(id).unwrap().text_editor_fonts(0).into_iter().find(|f| f.bytes == font).unwrap().resource;
    let style = BlockStyle { resource: Some(resource), width: Some(240.0), ..Default::default() };
    s.apply(id, Edit::EditTextBlock { page: 0, block: 0, text: text.into(), style: style.clone() }).unwrap();
    let edited = s.get(id).unwrap();
    let lines = edited.text_lines(0);
    assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<String>(), text);
    assert!(lines.iter().all(|l| l.base_font == expected_family));
    assert!((lines[0].baseline()[1] - 160.0).abs() < 0.01);
    let current = pixels(edited.display.clone());
    assert_ne!(current, original);
    let saved = s.save_bytes(id).unwrap();
    let re = s.open("saved.pdf", None, saved, None).unwrap();
    assert_eq!(pixels(s.get(re).unwrap().display.clone()), current);
    assert_eq!(s.get(re).unwrap().text_blocks(0)[0].editor_text, text);
    let twice = format!("{text} {text}");
    s.apply(re, Edit::EditTextBlock { page: 0, block: 0, text: twice.clone(), style: style.clone() }).unwrap();
    assert_eq!(s.get(re).unwrap().text_blocks(0)[0].editor_text, twice);
    let again = s.save_bytes(re).unwrap();
    let next = s.open("twice.pdf", None, again, None).unwrap();
    assert_eq!(s.get(next).unwrap().text_blocks(0)[0].editor_text, twice);
    assert_eq!(pixels(s.get(next).unwrap().display.clone()), pixels(s.get(re).unwrap().display.clone()));
    s.apply(re, Edit::EditTextBlock { page: 0, block: 0, text: String::new(), style }).unwrap();
    assert!(s.get(re).unwrap().text_lines(0).is_empty(), "every contextual glyph is deleted");
    let empty = pixels(s.get(re).unwrap().display.clone());
    assert!(empty.as_chunks::<4>().0.iter().all(|p| p[..3] == [255; 3]));
    s.undo(id).unwrap();
    assert_eq!(pixels(s.get(id).unwrap().display.clone()), original);
}
#[test]
fn opentype_cff_document_font_preserves_program_codes_and_rendering_after_reopen() {
    exercise(support::font_fixture::cff_font(), "A A", "RectFont-Regular");
}
#[test]
fn contextual_arabic_marks_ligatures_and_mixed_bidi_keep_logical_text_after_reopen() {
    if let Some(path) = std::env::var_os("PDFCRAFT_TEST_ARABIC_FONT") {
        exercise(std::fs::read(path).unwrap(), "سَلَام 123", "NotoSansArabic-Regular");
        return;
    }
    let Some(font) = pdfcraft_fonts::document_arabic_font() else {
        eprintln!("set CRAFT_FONTS_DIR to test full Arabic glyph programs");
        return;
    };
    if pdfcraft_fonts::embedding::validate(font.bytes).is_err() {
        eprintln!("set PDFCRAFT_TEST_ARABIC_FONT to an openly licensed static Noto Sans Arabic face");
        return;
    }
    exercise(font.bytes.to_vec(), "سَلَام 123", "NotoSansArabic-Regular");
}

/// Sintec.PDF: a face loaded into the page draws the characters the paragraph's own font can't
/// (here a Cyrillic letter in a Helvetica paragraph), instead of the generated Japanese
/// fallback, which builds without craft-fonts don't have; the rest keeps its font.
#[test]
fn a_loaded_substitute_draws_what_the_source_font_lacks() {
    let mut s = Session::new();
    let id = s.open("source.pdf", None, document(), None).unwrap();
    let facts = s.get(id).unwrap().font_facts(0, "F1").unwrap();
    assert_eq!(facts.base_font, "Helvetica");
    assert!(!facts.embedded && !facts.completed);
    let edit = Edit::EditTextBlock { page: 0, block: 0, text: "Source Ж".into(), style: BlockStyle::default() };
    if pdfcraft_fonts::CRAFT_FONTS.is_empty() {
        let e = s.apply(id, edit.clone()).unwrap_err().to_string();
        assert!(e.contains("fallback font"), "{e}");
    }
    let font = support::font_fixture::cff_font();
    s.apply(id, Edit::RegisterTextFont { page: 0, bytes: Arc::new(font) }).unwrap();
    s.apply(id, edit).unwrap();
    let lines = s.get(id).unwrap().text_lines(0);
    assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<String>().replace(' ', ""), "SourceЖ");
    assert!(lines.iter().any(|l| l.base_font == "Helvetica"), "{lines:?}");
    assert!(lines.iter().any(|l| l.base_font == "RectFont-Regular" && l.text.contains('Ж')), "{lines:?}");
}

/// Sintec.PDF: page 0's content, decoded, of the PDF `bytes`.
fn page_content(bytes: &Arc<Vec<u8>>) -> String {
    let doc = Document::open(bytes.clone()).unwrap();
    let page = pdfcraft_model::pages(&doc).into_iter().next().unwrap();
    let contents = page.dict.get(b"Contents").map(|c| doc.resolve(c)).unwrap();
    let refs = match contents.as_array() {
        Some(a) => a.to_vec(),
        None => vec![page.dict.get(b"Contents").cloned().unwrap()],
    };
    refs.iter()
        .filter_map(|r| match &*doc.resolve(r) {
            Object::Stream(s) => s.decoded().ok(),
            _ => None,
        })
        .map(|d| String::from_utf8_lossy(&d).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The top row of dark ink in pixel columns `x0..x1` of `rgba` (`width` pixels per row).
fn ink_top(rgba: &[u8], width: usize, x0: usize, x1: usize) -> Option<usize> {
    rgba.as_chunks::<4>().0.chunks(width).position(|row| row.get(x0..x1).is_some_and(|r| r.iter().any(|p| p[0] < 128)))
}

/// A paragraph in an embedded bold font that maps only "A", "H" and space (a PDF's subset, which
/// says it is bold only in its program's OS/2 table), with installed faces loaded as substitutes.
/// Returns the session, the document and the bold source's resource name.
fn bold_subset_paragraph(substitutes: &[Vec<u8>]) -> (Session, pdfcraft_engine::DocId) {
    let mut s = Session::new();
    let id = s.open("source.pdf", None, document(), None).unwrap();
    let bold = support::font_fixture::cff_font_styled("RectFont-Bold", "Bold", 700, 700, &[' ', 'A', 'H']);
    s.apply(id, Edit::RegisterTextFont { page: 0, bytes: Arc::new(bold.clone()) }).unwrap();
    let resource = s.get(id).unwrap().text_editor_fonts(0).into_iter().find(|f| f.bytes == bold).unwrap().resource;
    let style = BlockStyle { resource: Some(resource.clone()), ..Default::default() };
    s.apply(id, Edit::EditTextBlock { page: 0, block: 0, text: "AHA".into(), style }).unwrap();
    let facts = s.get(id).unwrap().font_facts(0, &resource).unwrap();
    assert!(facts.is_bold() && facts.weight == 700 && facts.weight_from == pdfcraft_engine::StyleSource::Program, "{facts:?}");
    assert_eq!(facts.cap_height, Some(0.7));
    for face in substitutes {
        s.apply(id, Edit::RegisterTextFont { page: 0, bytes: Arc::new(face.clone()) }).unwrap();
    }
    (s, id)
}

/// Sintec.PDF (task 027): a Cyrillic letter added to a bold heading whose embedded subset lacks it
/// comes out bold and as tall as the heading's letters. Only a regular face of the family is
/// loaded, with shorter capitals: it is scaled to the heading's cap height and widths, and drawn
/// stroked (synthetic bold).
#[test]
fn an_added_letter_in_a_bold_subset_heading_is_bold_and_as_tall() {
    let regular = support::font_fixture::cff_font_styled("RectFont-Regular", "Regular", 400, 625, &[' ', 'A', 'H', 'Ж']);
    let (mut s, id) = bold_subset_paragraph(&[regular]);
    s.apply(id, Edit::EditTextBlock { page: 0, block: 0, text: "AHA Ж".into(), style: BlockStyle::default() }).unwrap();
    let doc = s.get(id).unwrap();
    let lines = doc.text_lines(0);
    let added = lines.iter().find(|l| l.text.contains('Ж')).unwrap_or_else(|| panic!("{lines:?}"));
    assert_eq!(added.base_font, "RectFont-Regular", "{lines:?}");
    // The capitals' height: 18 pt × 0.7 em in the heading; the substitute's 0.625 em capitals are
    // set at 18 × 0.7 / 0.625 pt.
    assert!((added.size * 0.625 - 18.0 * 0.7).abs() < 0.05, "{added:?}");
    let content = page_content(&doc.display);
    assert!(content.contains("2 Tr"), "drawn stroked: {content}");
    // Rendered, its top is the heading letters' top.
    let mut renderer = PageRenderer::new(doc.display.clone(), Default::default());
    let p = renderer.render(RenderRequest { page: 0, scale: 2.0, ..Default::default() });
    let width = p.width as usize;
    let [a0, a1] = [lines[0].rect[0], lines[0].rect[0] + 9.0].map(|x| (x * 2.0) as usize + 2);
    let [z0, z1] = [added.rect[0], added.rect[2]].map(|x| (x * 2.0) as usize + 2);
    let (heading, letter) = (ink_top(&p.rgba, width, a0, a1).unwrap(), ink_top(&p.rgba, width, z0, z1.max(z0 + 1)).unwrap());
    assert!(heading.abs_diff(letter) <= 1, "heading top {heading} px, added letter top {letter} px");
}

/// Sintec.PDF (task 027): with the family's bold face loaded too, the added letter is set in that
/// face, not stroked, and not in the regular one.
#[test]
fn an_added_letter_in_a_bold_heading_uses_the_loaded_bold_face() {
    let regular = support::font_fixture::cff_font_styled("RectFont-Regular", "Regular", 400, 700, &[' ', 'A', 'H', 'Ж']);
    let bold = support::font_fixture::cff_font_styled("RectFont-BoldFull", "Bold", 700, 700, &[' ', 'A', 'H', 'Ж']);
    let (mut s, id) = bold_subset_paragraph(&[regular, bold]);
    s.apply(id, Edit::EditTextBlock { page: 0, block: 0, text: "AHA Ж".into(), style: BlockStyle::default() }).unwrap();
    let doc = s.get(id).unwrap();
    let lines = doc.text_lines(0);
    let added = lines.iter().find(|l| l.text.contains('Ж')).unwrap_or_else(|| panic!("{lines:?}"));
    assert_eq!(added.base_font, "RectFont-BoldFull", "{lines:?}");
    assert!((added.size - 18.0).abs() < 0.01, "{added:?}");
    assert!(!page_content(&doc.display).contains("2 Tr"));
}
