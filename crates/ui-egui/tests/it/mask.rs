//! Sintec.PDF «Закрыть белым» (Redact a PDF ▸ Mask areas with white) in the real shell: the
//! area dragged is covered with white once applied and what was under it is gone from the file;
//! the rest of the page is untouched and the redaction properties are not changed.

use egui::Pos2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_render::{PageRenderer, RenderRequest, RequestKind};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

/// A 300 × 300 pt page with a grey image over all of it (30 × 30 samples, 10 pt each), SECRET
/// near the top and PUBLIC near the bottom. An image, not a filled path: redaction removes a whole
/// path it touches but clears only an image's covered samples, to black, so a white area after
/// applying is the mask's own box, and the image's grey elsewhere shows the rest stayed.
fn fixture() -> Vec<u8> {
    let content = "q 300 0 0 300 0 0 cm /Im1 Do Q 0 g BT /F1 18 Tf 40 230 Td (SECRET) Tj ET BT /F1 18 Tf 40 40 Td (PUBLIC) Tj ET";
    let samples = "d".repeat(30 * 30);
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << \
         /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> /XObject << /Im1 5 0 R >> >> >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!(
            "<< /Type /XObject /Subtype /Image /Width 30 /Height 30 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n{samples}\nendstream",
            samples.len()
        ),
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

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, fixture()).unwrap();
        app.set_option("zoom", "150").unwrap();
        app
    });
    crate::support::settle(&mut h);
    h
}

/// A point of page 1 on screen, in points from its top-left.
fn at(h: &Harness<'static, PdfCraftApp>, x: f32, y: f32) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).expect("on screen");
    let k = r.width() / 300.0;
    r.min + egui::vec2(x * k, y * k)
}

fn marks(h: &Harness<'static, PdfCraftApp>) -> usize {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().redaction_marks()
}

fn drag(h: &mut Harness<'static, PdfCraftApp>, a: Pos2, b: Pos2) {
    h.hover_at(a);
    h.run_steps(1);
    h.drag_at(a);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(a + (b - a) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(b);
    h.run_steps(4);
}

#[test]
fn mask_areas_with_white_removes_what_is_under_them_for_good() {
    let mut h = harness();
    // Redact a PDF lists the tool; picking it turns the Redact tool into the white mask.
    assert!(h.state_mut().execute("redact.mark"));
    h.run_steps(2);
    assert!(!h.state().mask_mode);
    h.get_by_label("Mask areas with white").click();
    h.run_steps(2);
    assert!(h.state().mask_mode);
    assert_eq!(h.state().quick_tool, pdfcraft_ui_egui::QuickTool::Redact);

    // Drag over SECRET (y 230 from the bottom is 70 from the top).
    let (a, b) = (at(&h, 20.0, 45.0), at(&h, 280.0, 85.0));
    drag(&mut h, a, b);
    assert_eq!(marks(&h), 1);
    // Applied from the panel's footer, as any redaction.
    h.get_by_label("Redact all").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::RedactApply));
    h.get_by_label("Apply").click();
    h.run_steps(4);
    assert_eq!(marks(&h), 0, "no mark annotation remains");

    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    let mut r = PageRenderer::new(doc.bytes.clone(), Default::default());
    let text = r.render(RenderRequest { page: 0, kind: RequestKind::Text, scale: 1.0, ..Default::default() });
    let text = text.text.map(|t| t.plain_text()).unwrap_or_default();
    assert!(!text.contains("SECRET") && text.contains("PUBLIC"), "{text}");
    let px = r.render(RenderRequest { page: 0, scale: 1.0, ..Default::default() });
    let pixel = |x: usize, y: usize| {
        let i = (y * px.width as usize + x) * 4;
        [px.rgba[i], px.rgba[i + 1], px.rgba[i + 2]]
    };
    assert_eq!(pixel(150, 65), [255, 255, 255], "white where the secret was");
    assert!(pixel(150, 150).iter().all(|c| (90..=110).contains(c)), "the rest of the page keeps its grey: {:?}", pixel(150, 150));
    // The redaction properties still say black, and Redact marks with them again.
    assert_eq!(s.redact_prefs.fill, Some([0.0, 0.0, 0.0]));
    assert!(h.state_mut().execute("redact.mark"));
    assert!(!h.state().mask_mode);
    assert_eq!(h.state().mark_prefs().fill, Some([0.0, 0.0, 0.0]));
}
