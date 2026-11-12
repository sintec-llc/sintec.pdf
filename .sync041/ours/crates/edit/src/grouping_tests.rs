//! Which runs of text editing groups into one paragraph ([`text::text_blocks`]): the separate
//! fields of a form and the cells of a ruled table stay apart, while paragraphs, wrapped rows
//! and a heading over its text still join. Synthetic pages in Courier 10 (6 pt a character).

use std::sync::Arc;

use pdfcraft_cos::Document;

use super::*;

/// One page drawing `content`, with Courier as /F1 and Courier-Bold as /F2.
fn page(content: &str) -> Document {
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier-Bold /Encoding /WinAnsiEncoding >>".into(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    Document::open(Arc::new(out)).unwrap()
}

fn editor_texts(doc: &Document) -> Vec<String> {
    text::text_blocks(doc, 0).unwrap().into_iter().map(|b| b.editor_text).collect()
}

/// The gap (in ems of 10 pt) between the runs `a` and `b` on one row.
fn gap(doc: &Document, a: &str, b: &str) -> f64 {
    let lines = text::text_lines(doc, 0).unwrap();
    let find = |t: &str| lines.iter().find(|l| l.text == t).unwrap_or_else(|| panic!("no {t}: {lines:?}")).rect;
    (find(b)[0] - find(a)[2]) / 10.0
}

#[test]
fn form_labels_far_apart_on_one_baseline_are_separate_paragraphs() {
    let doc = page(
        "BT /F2 10 Tf 72 700 Td (Moscow office network) Tj ET BT /F1 10 Tf 210 700 Td (User) Tj ET \
         BT /F1 10 Tf 72 650 Td (Network access) Tj ET BT /F1 10 Tf 168 650 Td (Login) Tj ET \
         BT /F1 10 Tf 72 600 Td (Hello) Tj ET BT /F1 10 Tf 108 600 Td (world) Tj ET \
         BT /F1 10 Tf 72 550 Td (justified) Tj 66 0 Td (words) Tj ET",
    );
    // Further apart than a word space, closer than the 1.5 em at which runs always split.
    for (a, b) in [("Moscow office network", "User"), ("Network access", "Login")] {
        let g = gap(&doc, a, b);
        assert!((1.1..1.4).contains(&g), "{a} … {b}: {g} em");
    }
    // The justified words are one line: their gap is between its word boxes.
    let lines = text::text_lines(&doc, 0).unwrap();
    let justified = lines.iter().find(|l| l.text == "justified words").unwrap_or_else(|| panic!("{lines:?}"));
    let g = (justified.ink[1][0] - justified.ink[0][2]) / 10.0;
    assert!((1.1..1.4).contains(&g), "{g} em");
    // Hello and world are one line too, as before.
    assert!(lines.iter().any(|l| l.text == "Hello world"), "{lines:?}");
    let texts = editor_texts(&doc);
    // Separate text objects that far apart are separate fields, whatever their style.
    for label in ["Moscow office network", "User", "Network access", "Login"] {
        assert!(texts.iter().any(|t| t == label), "{label:?} is its own paragraph: {texts:?}");
    }
    // A word space between text objects, or a justified gap inside one, still joins.
    assert!(texts.iter().any(|t| t == "Hello world"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "justified words"), "{texts:?}");
}

#[test]
fn table_cells_split_at_their_borders() {
    let doc = page(
        // A two-column header row: the cells are only 0.8 em apart, across the column border.
        "0.5 w 66 700 234 18 re S 160 700 m 160 718 l S \
         BT /F1 10 Tf 72 705 Td (Network access) Tj ET BT /F1 10 Tf 164 705 Td (Granted) Tj ET \
         66 582 234 36 re S 66 600 m 300 600 l S \
         BT /F2 10 Tf 72 605 Td (Network access to the Moscow office) Tj ET \
         BT /F1 10 Tf 72 587 Td (User) Tj ET",
    );
    assert!(gap(&doc, "Network access", "Granted") < 1.0);
    let texts = editor_texts(&doc);
    for cell in ["Network access", "Granted", "Network access to the Moscow office", "User"] {
        assert!(texts.iter().any(|t| t == cell), "{cell:?} is its own paragraph: {texts:?}");
    }
    assert_eq!(texts.len(), 4, "{texts:?}");
}

#[test]
fn turned_table_cells_split_at_their_borders() {
    // The one-column form above, drawn running up the page: its rows stack to the right.
    let doc = page(
        "0.5 w 312 94 m 312 400 l S \
         BT /F2 10 Tf 0 1 -1 0 305 100 Tm (Network access to the Moscow office) Tj ET \
         BT /F1 10 Tf 0 1 -1 0 323 100 Tm (User) Tj ET",
    );
    assert_eq!(editor_texts(&doc), ["Network access to the Moscow office", "User"]);
    // Without the border they are a heading over its value.
    let doc = page(
        "BT /F2 10 Tf 0 1 -1 0 305 100 Tm (Network access to the Moscow office) Tj ET \
         BT /F1 10 Tf 0 1 -1 0 323 100 Tm (User) Tj ET",
    );
    assert_eq!(editor_texts(&doc), ["Network access to the Moscow office\nUser"]);
}

#[test]
fn paragraphs_inside_a_ruled_cell_still_join() {
    // A cell holding an underlined heading over two wrapped rows, and a paragraph beside the
    // table: borders around them, an underline and a strike-through don't split them.
    let doc = page(
        "0.5 w 66 600 234 60 re S 300 600 m 300 660 l S \
         BT /F2 10 Tf 72 645 Td (Heading) Tj ET 72 643.5 m 114 643.5 l S \
         BT /F1 10 Tf 12 TL 72 630 Td (first row of the cell) Tj T* (second row) Tj ET 72 621 m 132 621 l S \
         BT /F1 10 Tf 12 TL 320 645 Td (A paragraph that) Tj T* (wraps over three) Tj T* (rows.) Tj ET",
    );
    let blocks = text::text_blocks(&doc, 0).unwrap();
    let texts: Vec<&str> = blocks.iter().map(|b| b.editor_text.as_str()).collect();
    assert_eq!(texts, ["Heading\nfirst row of the cell\nsecond row", "A paragraph that\nwraps over three\nrows."]);
    assert_eq!(blocks[1].text, "A paragraph that wraps over three rows.");
}

#[test]
fn hostile_rules_do_not_disturb_grouping() {
    // Rules that aren't finite, or a page of hatching, change nothing and crash nothing.
    let mut content = String::from("BT /F1 10 Tf 12 TL 72 700 Td (one) Tj T* (two) Tj ET 0 0 m 1e400 0 l S 0 0 m nan 5 l S ");
    for i in 0..25_000 {
        content.push_str(&format!("0 {} m 5 {} l ", i % 50, i % 50));
    }
    content.push('S');
    assert_eq!(editor_texts(&page(&content)), ["one\ntwo"]);
}
