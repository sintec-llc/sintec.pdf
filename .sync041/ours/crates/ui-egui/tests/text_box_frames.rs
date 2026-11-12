//! Task 015: a paragraph box's frame is where its text wraps. Checked against the rows an edit
//! really produces, on a synthetic table-like page: a multi-word cell beside a one-word cell, a
//! heading over its value, and a paragraph soft-wrapped onto five rows. Also after a width drag
//! (widening or narrowing), and on a quarter-turned page.
#[path = "support/mod.rs"]
mod support;
use egui::Pos2;
use egui_kittest::Harness;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;
use support::standard_faces;

const PARAGRAPH: &str = "the quick brown fox jumps over the lazy dog while seven bold zebras quietly graze near a calm river bank";
/// Courier 12: every character (and space) is 7.2 pt.
const CHAR: f64 = 7.2;

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

/// A 400 × 300 page in Courier 12.
fn fixture() -> Vec<u8> {
    let mut body = String::from(
        "BT /F1 12 Tf 20 260 Td (alpha beta gamma) Tj ET\nBT /F1 12 Tf 200 260 Td (solo) Tj ET\nBT /F1 12 Tf 20 200 Td (Name:) Tj 0 -14 Td (John Smith Junior) Tj ET\nBT /F1 12 Tf 14 TL 20 140 Td",
    );
    for (i, row) in greedy_rows(PARAGRAPH, 24).iter().enumerate() {
        body += if i > 0 { " T*" } else { "" };
        body += &format!(" ({row}) Tj");
    }
    body += " ET\n";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 400 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
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

fn harness(rotation: i64) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("table.pdf", None, fixture()).expect("fixture opens");
        app.fonts.installed_override = Some(standard_faces());
        if rotation != 0 {
            assert!(app.apply_edit(Edit::RotatePages { pages: vec![0], degrees: rotation }));
        }
        app
    });
    h.run_steps(4);
    h.state_mut().views[0].set_zoom(1.5);
    h.run_steps(4);
    assert!(h.state_mut().execute("edit.edit_text"));
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    for _ in 0..60 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The box shown for the paragraph starting with `prefix` (user space).
fn shown(h: &Harness<'static, PdfCraftApp>, prefix: &str) -> [f64; 4] {
    let boxes = h.state().views[0].shown_text_boxes(0);
    boxes.iter().find(|(t, _)| t.starts_with(prefix)).unwrap_or_else(|| panic!("no box {prefix}: {boxes:?}")).1
}

type Rows = Vec<(String, [f64; 4])>;

/// The rows of the paragraph starting with `prefix`, as the document has them.
fn rows(h: &Harness<'static, PdfCraftApp>, prefix: &str) -> Rows {
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    let lines = doc.text_lines(0);
    let blocks = doc.text_blocks(0);
    let b = blocks.iter().find(|b| b.text.starts_with(prefix)).unwrap_or_else(|| panic!("no {prefix}: {blocks:?}"));
    b.rows
        .iter()
        .map(|row| {
            let text = row.iter().map(|i| lines[*i].text.trim().to_owned()).collect::<Vec<_>>().join(" ");
            let rect = row.iter().map(|i| lines[*i].rect).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]).unwrap();
            (text, rect)
        })
        .collect()
}

/// User space → screen, for the page turned by `rotation`.
fn to_screen(h: &Harness<'static, PdfCraftApp>, rotation: i64, x: f64, y: f64) -> Pos2 {
    let page = h.state().views[0].page_screen_rect(0).unwrap();
    let (u, v) = (x as f32 / 400.0, (300.0 - y as f32) / 300.0);
    let (a, b) = match rotation {
        90 => (1.0 - v, u),
        180 => (1.0 - u, 1.0 - v),
        270 => (v, 1.0 - u),
        _ => (u, v),
    };
    page.min + egui::vec2(a * page.width(), b * page.height())
}

/// Screen pixels per point.
fn scale(h: &Harness<'static, PdfCraftApp>, rotation: i64) -> f32 {
    let page = h.state().views[0].page_screen_rect(0).unwrap();
    if rotation % 180 == 0 { page.width() / 400.0 } else { page.width() / 300.0 }
}

/// Open the paragraph starting with `prefix` for typing, change its text with `edit`, and apply.
fn type_into(h: &mut Harness<'static, PdfCraftApp>, rotation: i64, prefix: &str, edit: impl FnOnce(&mut String)) {
    let r = rows(h, prefix);
    let first = r[0].1;
    let p = to_screen(h, rotation, first[0] + 2.0 * CHAR, (first[1] + first[3]) / 2.0);
    h.hover_at(p);
    h.run_steps(2);
    h.drag_at(p);
    h.drop_at(p);
    h.run_steps(6);
    let ed = h.state_mut().views[0].line_editor.as_mut().expect("the click opens the editor");
    assert!(ed.text.starts_with(prefix), "{:?}", ed.text);
    edit(&mut ed.text);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    settle(h);
    assert!(h.state().views[0].line_editor.is_none());
}

/// Drag the far width edge (the paragraph's right in user space) of the box of `prefix` by `by`
/// points, outward when positive.
fn drag_right_edge(h: &mut Harness<'static, PdfCraftApp>, rotation: i64, prefix: &str, by: f64) {
    let f = shown(h, prefix);
    let k = scale(h, rotation);
    // Just outside the box's edge, near its first row.
    let edge = to_screen(h, rotation, f[2], f[3] - 6.0);
    let outward = to_screen(h, rotation, f[2] + 1.0, f[3] - 6.0) - edge;
    let dir = outward / outward.length();
    let from = edge + dir * 4.0;
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=4 {
        h.hover_at(from + dir * (by as f32 * k) * (i as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(from + dir * (by as f32 * k));
    settle(h);
}

/// Every row lies inside the frame across, and each row in `full` was wrapped at its right edge:
/// a space and the next row's first word would have passed it.
fn assert_wraps_at(rows: &Rows, frame: [f64; 4], full: &[usize]) {
    for (text, r) in rows {
        assert!(r[0] >= frame[0] - 0.5 && r[2] <= frame[2] + 0.6, "{text:?} {r:?} is outside the frame {frame:?}: {rows:?}");
    }
    for &i in full {
        let (row, next) = (&rows[i], &rows[i + 1]);
        let word = next.0.split(' ').next().unwrap().chars().count() as f64 * CHAR;
        assert!(row.1[2] + CHAR + word > frame[2] + 0.5, "{:?} wrapped before the frame's edge {frame:?}: {rows:?}", row.0);
    }
}

/// A box's edges across (where its text wraps).
fn across(b: [f64; 4]) -> [f64; 2] {
    [b[0], b[2]]
}

fn all_but_last(rows: &Rows) -> Vec<usize> {
    (0..rows.len().saturating_sub(1)).collect()
}

/// Typing into each kind of box wraps the text at the box's right edge.
fn frames_show_where_typing_wraps(rotation: i64) {
    // A multi-word cell: up to just before the cell beside it.
    let mut h = harness(rotation);
    let frame = shown(&h, "alpha");
    assert!((frame[2] - 194.0).abs() < 0.5, "stops half an em before the next cell: {frame:?}");
    type_into(&mut h, rotation, "alpha", |t| t.push_str(" delta epsilon zeta eta theta iota kappa lambda"));
    let r = rows(&h, "alpha");
    assert!(r.len() >= 3, "{r:?}");
    assert_wraps_at(&r, frame, &all_but_last(&r));
    assert_eq!(rows(&h, "solo")[0].0, "solo", "the next cell is untouched");

    // A one-word cell: up to the page's margin.
    let mut h = harness(rotation);
    let frame = shown(&h, "solo");
    assert!((frame[2] - 364.0).abs() < 0.5, "{frame:?}");
    type_into(&mut h, rotation, "solo", |t| t.push_str(" one two three four five six seven eight nine ten eleven"));
    let r = rows(&h, "solo");
    assert!(r.len() >= 3, "{r:?}");
    assert_wraps_at(&r, frame, &all_but_last(&r));

    // A heading over its value: the value wraps at the box; the heading keeps its own row.
    let mut h = harness(rotation);
    let frame = shown(&h, "Name:");
    assert!((frame[2] - frame[0] - 17.0 * CHAR).abs() < 0.5, "as wide as its value: {frame:?}");
    type_into(&mut h, rotation, "Name:", |t| t.push_str(" the third of that name from far away"));
    let r = rows(&h, "Name:");
    assert_eq!(r[0].0, "Name:", "{r:?}");
    assert!(r.len() >= 4, "{r:?}");
    assert_wraps_at(&r, frame, &(1..r.len() - 1).collect::<Vec<_>>());

    // A soft-wrapped paragraph of five rows: a longer first row wraps at the box.
    let mut h = harness(rotation);
    let before = rows(&h, "the quick");
    assert!(before.len() >= 4);
    let frame = shown(&h, "the quick");
    let widest = before.iter().map(|(_, r)| r[2]).fold(f64::MIN, f64::max);
    assert!((frame[2] - widest).abs() < 0.5 && (frame[0] - 20.0).abs() < 0.5, "as wide as its widest row: {frame:?}");
    type_into(&mut h, rotation, "the quick", |t| t.insert_str("the quick ".len(), "really "));
    let r = rows(&h, "the quick");
    assert!(r.len() > before.len(), "{r:?}");
    assert_wraps_at(&r, frame, &[0]);
}

#[test]
fn frames_show_where_typing_wraps_each_kind_of_box() {
    frames_show_where_typing_wraps(0);
}

#[test]
fn frames_show_where_typing_wraps_on_a_quarter_turned_page() {
    frames_show_where_typing_wraps(90);
}

/// Drag the paragraph's right edge by `by`, then type at its end: the box keeps the dragged width
/// (no snap-back), and the text wraps at it.
fn drag_then_type(rotation: i64, by: f64) {
    let mut h = harness(rotation);
    let before = shown(&h, "the quick");
    let rows_before = rows(&h, "the quick");
    drag_right_edge(&mut h, rotation, "the quick", by);
    let frame = shown(&h, "the quick");
    assert!((frame[2] - (before[2] + by)).abs() < 1.5, "the box keeps the dragged edge: {before:?} → {frame:?}");
    let r = rows(&h, "the quick");
    if by > 0.0 {
        assert!(r.len() < rows_before.len(), "widening rewraps onto fewer rows: {r:?}");
    } else {
        assert!(r.len() > rows_before.len(), "narrowing rewraps onto more rows: {r:?}");
    }
    assert_wraps_at(&r, frame, &all_but_last(&r));
    // Idle frames keep it.
    h.run_steps(4);
    assert_eq!(shown(&h, "the quick"), frame);

    type_into(&mut h, rotation, "the quick", |t| t.push_str(" and then some more words to fill another row"));
    let r = rows(&h, "the quick");
    assert_wraps_at(&r, frame, &[r.len() - 2]);
    assert!(r.last().unwrap().0.ends_with("another row"), "{r:?}");
    assert_eq!(across(shown(&h, "the quick")), across(frame), "the box still shows where the text wraps");

    // Typing again still wraps there: the width is kept with the paragraph.
    type_into(&mut h, rotation, "the quick", |t| t.push_str(" and yet more"));
    let r = rows(&h, "the quick");
    assert_wraps_at(&r, frame, &[]);
    assert_eq!(across(shown(&h, "the quick")), across(frame));
}

#[test]
fn widening_then_typing_wraps_at_the_frames_right_edge() {
    drag_then_type(0, 60.0);
}

#[test]
fn narrowing_then_typing_wraps_at_the_frame() {
    drag_then_type(0, -50.0);
}

#[test]
fn widening_and_narrowing_on_a_quarter_turned_page() {
    drag_then_type(90, 60.0);
    drag_then_type(90, -50.0);
}

#[test]
fn widening_a_cell_up_to_its_neighbour_grabs_its_own_edge() {
    // The boxes of neighbouring cells sit half an em apart: the nearer edge wins the press.
    let mut h = harness(0);
    let solo = shown(&h, "solo");
    drag_right_edge(&mut h, 0, "alpha", 30.0);
    let frame = shown(&h, "alpha");
    assert!((frame[2] - 224.0).abs() < 1.5, "{frame:?}");
    assert_eq!(shown(&h, "solo"), solo, "the neighbour is untouched");
    type_into(&mut h, 0, "alpha", |t| t.push_str(" delta epsilon zeta eta theta"));
    let r = rows(&h, "alpha");
    assert!(r.len() >= 2, "{r:?}");
    assert_wraps_at(&r, frame, &all_but_last(&r));
}

#[test]
fn a_kept_width_is_dropped_by_any_other_change() {
    let mut h = harness(0);
    drag_right_edge(&mut h, 0, "the quick", 60.0);
    let dragged = shown(&h, "the quick");
    // Another paragraph's edit: the dragged one's box shows the width a rewrite would now use.
    type_into(&mut h, 0, "solo", |t| t.push_str(" more"));
    let after = shown(&h, "the quick");
    let widest = rows(&h, "the quick").iter().map(|(_, r)| r[2]).fold(f64::MIN, f64::max);
    assert!((after[2] - widest).abs() < 0.5 && after[2] <= dragged[2], "{dragged:?} → {after:?}");
    // And that is where typing then wraps.
    type_into(&mut h, 0, "the quick", |t| t.push_str(" plus a few more words here"));
    let r = rows(&h, "the quick");
    assert_wraps_at(&r, after, &[r.len() - 2]);
}
