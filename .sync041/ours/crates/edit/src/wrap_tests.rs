//! A paragraph's frame is where it wraps ([`text::TextBlock::wrap`]): checked against the rows a
//! rewrite really produces, on a synthetic table-like page.

use std::sync::Arc;

use pdfcraft_cos::Document;

use super::*;

const PARAGRAPH: &str = "the quick brown fox jumps over the lazy dog while seven bold zebras quietly graze near a calm river bank";

/// Greedy wrap at `chars` characters (the fixture's soft-wrapped rows).
fn greedy_rows(text: &str, chars: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match rows.last_mut() {
            Some(row) if row.len() + 1 + word.len() <= chars => {
                row.push(' ');
                row.push_str(word);
            }
            _ => rows.push(word.to_owned()),
        }
    }
    rows
}

/// A table-like page in Courier 12 (7.2 pt a character), turned by `rotate`: a multi-word cell
/// with a one-word cell beside it, a heading over its value, and a paragraph soft-wrapped at 24
/// characters.
fn table(rotate: u32) -> Document {
    let mut content = String::from(
        "BT /F1 12 Tf 20 260 Td (alpha beta gamma) Tj ET\nBT /F1 12 Tf 200 260 Td (solo) Tj ET\nBT /F1 12 Tf 20 200 Td (Name:) Tj 0 -14 Td (John Smith Junior) Tj ET\nBT /F1 12 Tf 14 TL 20 140 Td",
    );
    for (i, row) in greedy_rows(PARAGRAPH, 24).iter().enumerate() {
        content += if i > 0 { " T*" } else { "" };
        content += &format!(" ({row}) Tj");
    }
    content += " ET\n";
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Rotate {rotate} /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
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

type Rows = Vec<(String, [f64; 4])>;

/// The paragraph starting with `prefix`: its index, block and rows (text and extent).
fn block(doc: &Document, prefix: &str) -> (usize, text::TextBlock, Rows) {
    let lines = text::text_lines(doc, 0).unwrap();
    let blocks = text::text_blocks(doc, 0).unwrap();
    let (i, b) = blocks.iter().enumerate().find(|(_, b)| b.text.starts_with(prefix)).unwrap_or_else(|| panic!("no {prefix}: {blocks:?}"));
    let rows = b
        .rows
        .iter()
        .map(|row| {
            let text = row.iter().map(|i| lines[*i].text.trim().to_owned()).collect::<Vec<_>>().join(" ");
            let rect = row.iter().map(|i| lines[*i].rect).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]).unwrap();
            (text, rect)
        })
        .collect();
    (i, b.clone(), rows)
}

/// Every row stays inside `wrap`, and each row listed in `full` was wrapped there: a space and
/// the next row's first word would have passed its right edge (Courier 12: 7.2 pt a character).
fn assert_wraps_at(rows: &Rows, wrap: [f64; 2], full: &[usize]) {
    for (text, r) in rows {
        assert!(r[0] >= wrap[0] - 0.5 && r[2] <= wrap[1] + 0.6, "{text:?} {r:?} is outside the frame {wrap:?}: {rows:?}");
    }
    for &i in full {
        let (row, next) = (&rows[i], &rows[i + 1]);
        let word = next.0.split(' ').next().unwrap().chars().count() as f64 * 7.2;
        assert!(row.1[2] + 7.2 + word > wrap[1] + 0.5, "{:?} wrapped before the frame {wrap:?}: {rows:?}", row.0);
    }
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.5 && (a[1] - b[1]).abs() < 0.5
}

/// Rewrite the paragraph starting with `prefix` (on a fresh page) as `draft`: its wrap before,
/// and its rows after.
fn typed(rotate: u32, prefix: &str, draft: &str) -> ([f64; 2], Rows) {
    let mut doc = table(rotate);
    let (i, before, _) = block(&doc, prefix);
    text::rewrite_block(&mut doc, 0, i, Some(draft), &text::BlockStyle::default()).unwrap();
    let (_, _, rows) = block(&doc, prefix);
    (before.wrap, rows)
}

fn check_frames_match_wrapping(rotate: u32) {
    let doc = table(rotate);
    let paragraph_rows = greedy_rows(PARAGRAPH, 24);
    assert!(paragraph_rows.len() >= 4);
    let longest = paragraph_rows.iter().map(|r| r.len()).max().unwrap() as f64 * 7.2;
    // A one-row cell grows up to its neighbour (half an em before it) or the page's margin.
    let (_, cell, _) = block(&doc, "alpha");
    assert!(near(cell.wrap, [20.0, 194.0]), "{:?}", cell.wrap);
    let (_, word, _) = block(&doc, "solo");
    assert!(near(word.wrap, [200.0, 364.0]), "{:?}", word.wrap);
    // Several rows keep their width.
    let (_, heading, _) = block(&doc, "Name:");
    assert!(near(heading.wrap, [20.0, 20.0 + 17.0 * 7.2]), "{:?}", heading.wrap);
    let (_, para, _) = block(&doc, "the quick");
    assert_eq!(para.rows.len(), paragraph_rows.len());
    assert!(near(para.wrap, [20.0, 20.0 + longest]), "{:?}", para.wrap);

    // Typing into each wraps where its frame said.
    let (wrap, rows) = typed(rotate, "alpha", "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda");
    assert!(rows.len() >= 3, "{rows:?}");
    assert_wraps_at(&rows, wrap, &(0..rows.len() - 1).collect::<Vec<_>>());
    let (wrap, rows) = typed(rotate, "solo", "solo one two three four five six seven eight nine ten eleven");
    assert!(rows.len() >= 3, "{rows:?}");
    assert_wraps_at(&rows, wrap, &(0..rows.len() - 1).collect::<Vec<_>>());
    let (wrap, rows) = typed(rotate, "Name:", "Name:\nJohn Smith Junior the third of that name from far away");
    assert_eq!(rows[0].0, "Name:", "the heading keeps its own row: {rows:?}");
    assert!(rows.len() >= 4, "{rows:?}");
    assert_wraps_at(&rows, wrap, &(1..rows.len() - 1).collect::<Vec<_>>());
    // The inline editor's draft keeps a newline between rows.
    let mut draft = paragraph_rows.join("\n");
    draft.insert_str("the quick ".len(), "really ");
    let (wrap, rows) = typed(rotate, "the quick", &draft);
    assert!(rows.len() > paragraph_rows.len(), "the longer first row wrapped: {rows:?}");
    assert_wraps_at(&rows, wrap, &[0]);
}

#[test]
fn a_paragraphs_wrap_is_where_typing_wraps_it() {
    check_frames_match_wrapping(0);
}

#[test]
fn a_paragraphs_wrap_on_a_turned_page_is_where_typing_wraps_it() {
    for rotate in [90, 180, 270] {
        check_frames_match_wrapping(rotate);
    }
}

#[test]
fn a_single_line_stops_before_text_on_its_row_only() {
    let doc = table(0);
    // "alpha" stops before "solo" beside it; the heading below has nothing beside it.
    let (_, heading, _) = block(&doc, "Name:");
    let (_, cell, _) = block(&doc, "alpha");
    assert!(cell.wrap[1] < 200.0 && heading.wrap[1] < 200.0);
    // Reading blocks don't derive a wrap: theirs is the text's extent.
    let reading = text::reading_blocks(&doc, 0).unwrap();
    assert!(reading.iter().all(|b| b.wrap == [b.rect[0], b.rect[2]]), "{reading:?}");
}

#[test]
fn rewrap_text_keeps_a_heading_apart_and_joins_soft_wrapped_rows() {
    let doc = table(0);
    let lines = text::text_lines(&doc, 0).unwrap();
    let (_, heading, _) = block(&doc, "Name:");
    assert_eq!(heading.rewrap_text(&lines), "Name:\nJohn Smith Junior");
    let (_, para, _) = block(&doc, "the quick");
    assert_eq!(para.text, PARAGRAPH);
    assert_eq!(para.rewrap_text(&lines), para.text);
    let (_, cell, _) = block(&doc, "alpha");
    assert_eq!(cell.rewrap_text(&lines), "alpha beta gamma");
    // Lines that don't match the block's rows (another page's) change nothing.
    assert_eq!(heading.rewrap_text(&[]), heading.text);
}

#[test]
fn an_explicit_width_rewraps_rows_but_keeps_a_heading_apart() {
    // Narrowing the paragraph: its soft-wrapped rows rewrap at the new width.
    let mut doc = table(0);
    let lines = text::text_lines(&doc, 0).unwrap();
    let (i, para, _) = block(&doc, "the quick");
    let style = text::BlockStyle { width: Some(120.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, i, Some(&para.rewrap_text(&lines)), &style).unwrap();
    let (_, _, rows) = block(&doc, "the quick");
    assert!(rows.len() > para.rows.len(), "{rows:?}");
    assert_wraps_at(&rows, [20.0, 140.0], &(0..rows.len() - 1).collect::<Vec<_>>());
    // Widening the heading block: its value doesn't join the heading.
    let mut doc = table(0);
    let lines = text::text_lines(&doc, 0).unwrap();
    let (i, heading, _) = block(&doc, "Name:");
    let style = text::BlockStyle { width: Some(300.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, i, Some(&heading.rewrap_text(&lines)), &style).unwrap();
    let (_, _, rows) = block(&doc, "Name:");
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["Name:", "John Smith Junior"]);
}
