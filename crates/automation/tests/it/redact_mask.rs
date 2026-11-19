//! Sintec.PDF «Закрыть белым» through the tools: `redact_mask` marks an area like `redact_mark`,
//! but always white and without overlay text; once `redact_apply` runs, what was under it is gone
//! from the file, the area is white, and the rest of the page stays.
use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// A 200 × 200 pt page with a grey image over all of it (20 × 20 samples, 10 pt each), SECRET
/// near the top and PUBLIC near the bottom. An image, not a filled path: redaction removes a whole
/// path it touches but clears only an image's covered samples, to black, so a white area after
/// applying is the mask's own box, and the image's grey elsewhere shows the rest stayed.
fn pdf() -> Vec<u8> {
    let content = "q 200 0 0 200 0 0 cm /Im1 Do Q 0 g BT /F1 12 Tf 20 150 Td (SECRET) Tj ET BT /F1 12 Tf 20 40 Td (PUBLIC) Tj ET";
    let samples = "d".repeat(20 * 20);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << \
         /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> /XObject << /Im1 5 0 R >> >> >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!(
            "<< /Type /XObject /Subtype /Image /Width 20 /Height 20 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n{samples}\nendstream",
            samples.len()
        ),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn ok(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => v,
            other => panic!("{tool}: expected JSON, got {other:?}"),
        },
        Err(e) => panic!("{tool} {args}: {e}"),
    }
}

fn text(a: &mut Automation, doc: u64) -> String {
    ok(a, "text_extract", json!({ "doc": doc }))["pages"][0]["text"].as_str().unwrap().to_string()
}

/// The RGB of pixel (x, y), in points from the top-left, of page 1 rendered at 72 dpi.
fn pixel(a: &mut Automation, doc: u64, x: usize, y: usize) -> [u8; 3] {
    let rendered = a.call("page_render", &json!({ "doc": doc, "page": 1, "dpi": 72 })).unwrap();
    let Content::Png { data, width, .. } = &rendered[0] else { panic!("expected a PNG") };
    let mut reader = png::Decoder::new(std::io::Cursor::new(data)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let channels = info.color_type.samples();
    let at = (y * *width as usize + x) * channels;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

/// The image's grey (sample 0x64, 100), give or take the renderer's rounding.
fn grey(p: [u8; 3]) -> bool {
    p.iter().all(|c| (90..=110).contains(c))
}

#[test]
fn redact_mask_covers_an_area_with_white_and_removes_what_is_under_it() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-redact-mask-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("memo.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = ok(&mut a, "doc_open", json!({ "path": "memo.pdf" }))["doc"].as_u64().unwrap();
    assert!(grey(pixel(&mut a, doc, 100, 50)), "the page is grey before");

    // The tool belongs to the «Закрыть белым» command and refuses what would change its look.
    let def = pdfcraft_automation::tools().into_iter().find(|t| t.name == "redact_mask").expect("redact_mask is a tool");
    assert_eq!(def.command, Some("redact.mask"));
    for (key, value) in [("fill", json!("black")), ("overlay", json!("X")), ("code_set", json!("foia"))] {
        let mut args = json!({ "doc": doc, "page": 1, "rect": [10, 35, 190, 65] });
        args[key] = value;
        assert!(matches!(a.call("redact_mask", &args), Err(ToolError::InvalidArgs(_))), "{key} is refused");
    }

    // SECRET's line (y 150 from the bottom is 50 from the top).
    let r = ok(&mut a, "redact_mask", json!({ "doc": doc, "page": 1, "rect": [10, 35, 190, 65] }));
    assert_eq!((r["marked"].as_u64(), r["marks_pending"].as_u64()), (Some(1), Some(1)), "{r}");
    assert!(text(&mut a, doc).contains("SECRET"), "a mark alone removes nothing");
    let r = ok(&mut a, "redact_apply", json!({ "doc": doc }));
    assert_eq!(r["marks_pending"].as_u64(), Some(0), "no mark remains: {r}");

    let after = text(&mut a, doc);
    assert!(!after.contains("SECRET") && after.contains("PUBLIC"), "{after}");
    assert_eq!(pixel(&mut a, doc, 100, 50), [255, 255, 255], "white where the secret was");
    assert!(grey(pixel(&mut a, doc, 100, 100)), "the rest of the page is untouched");
    ok(&mut a, "doc_save", json!({ "doc": doc, "path": "masked.pdf" }));
    let saved = std::fs::read(dir.join("masked.pdf")).unwrap();
    assert!(!saved.windows(6).any(|w| w == b"SECRET"), "the saved file has no trace of the text");
    let _ = std::fs::remove_dir_all(&dir);
}
