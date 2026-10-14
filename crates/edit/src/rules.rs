//! Sintec.PDF: the straight horizontal and vertical lines a page draws (Export to Excel and Word
//! read tables from them, see `pdfcraft_export`): stroked lines and rectangle edges, and filled
//! bars thin enough to be a rule, including those drawn inside form XObjects. Curves, diagonal
//! lines and clipping are ignored; what a table's grid needs is where its lines are.

use pdfcraft_content::{Matrix, parse};
use pdfcraft_cos::{Dict, Document, ObjRef};

use crate::EditError;

/// A filled rectangle at most this thin (points) is a rule, not a shape.
const THIN: f64 = 2.5;
/// Lines shorter than this (points) are marks, not rules.
const MIN_LENGTH: f64 = 3.0;
/// How far from horizontal or vertical (points across) a line may be.
const SKEW: f64 = 0.75;
/// The most segments read from one page: a page of hatching shouldn't cost minutes.
const MAX_SEGMENTS: usize = 20_000;

/// A straight horizontal or vertical segment in user space: `[x0, y0, x1, y1]` with x0 ≤ x1 and
/// y0 ≤ y1 (so a horizontal one has y0 == y1).
pub type Segment = [f64; 4];

/// The horizontal and vertical lines page `page` (0-based) draws, in user space.
pub fn ruling_segments(doc: &Document, page: usize) -> Result<Vec<Segment>, EditError> {
    struct Walk<'a> {
        doc: &'a Document,
        out: Vec<Segment>,
        path: Vec<ObjRef>,
        visits: usize,
    }
    #[derive(Default)]
    struct PathState {
        /// The current subpath's points (user space) and whether each closes back to the start.
        lines: Vec<([f64; 2], [f64; 2])>,
        rects: Vec<[f64; 4]>,
        /// Rectangles drawn as `m l l l h` subpaths (office suites fill table borders and cell
        /// shading that way): filled ones count as `re` rectangles do.
        path_rects: Vec<[f64; 4]>,
        /// The current subpath's points, while it is still straight.
        sub: Vec<[f64; 2]>,
        start: Option<[f64; 2]>,
        current: Option<[f64; 2]>,
    }
    /// The axis-aligned rectangle `pts` traces (4 corners, the start maybe repeated at the end).
    fn rect_of(pts: &[[f64; 2]]) -> Option<[f64; 4]> {
        let mut v: Vec<[f64; 2]> = Vec::with_capacity(5);
        for q in pts {
            if v.last().is_none_or(|l: &[f64; 2]| (l[0] - q[0]).abs() > 1e-6 || (l[1] - q[1]).abs() > 1e-6) {
                v.push(*q);
            }
        }
        if v.len() == 5 && v.first().zip(v.last()).is_some_and(|(a, b)| (a[0] - b[0]).abs() <= 1e-6 && (a[1] - b[1]).abs() <= 1e-6) {
            v.pop();
        }
        let [a, b, c, d] = <[[f64; 2]; 4]>::try_from(v).ok()?;
        let flat = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() <= SKEW || (p[1] - q[1]).abs() <= SKEW;
        if !(flat(a, b) && flat(b, c) && flat(c, d) && flat(d, a)) {
            return None;
        }
        let xs = [a[0], b[0], c[0], d[0]];
        let ys = [a[1], b[1], c[1], d[1]];
        let min = |v: [f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let max = |v: [f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let r = [min(xs), min(ys), max(xs), max(ys)];
        r.iter().all(|v| v.is_finite()).then_some(r)
    }
    fn end_sub(p: &mut PathState) {
        if let Some(r) = rect_of(&p.sub) {
            p.path_rects.push(r);
        }
        p.sub.clear();
    }
    fn push(out: &mut Vec<Segment>, a: [f64; 2], b: [f64; 2]) {
        if out.len() >= MAX_SEGMENTS || !a.iter().chain(&b).all(|v| v.is_finite()) {
            return;
        }
        let (dx, dy) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
        if dy <= SKEW && dx >= MIN_LENGTH {
            let y = (a[1] + b[1]) / 2.0;
            out.push([a[0].min(b[0]), y, a[0].max(b[0]), y]);
        } else if dx <= SKEW && dy >= MIN_LENGTH {
            let x = (a[0] + b[0]) / 2.0;
            out.push([x, a[1].min(b[1]), x, a[1].max(b[1])]);
        }
    }
    fn walk(w: &mut Walk, data: &[u8], resources: &Dict, ctm0: Matrix) {
        let mut ctm = ctm0;
        let mut stack: Vec<Matrix> = Vec::new();
        let mut p = PathState::default();
        for op in parse(data).ops {
            if w.out.len() >= MAX_SEGMENTS {
                return;
            }
            match op.op.as_slice() {
                b"q" => {
                    if stack.len() < 64 {
                        stack.push(ctm);
                    }
                }
                b"Q" => ctm = stack.pop().unwrap_or(ctm),
                b"cm" => {
                    if let Some(m) = op.nums::<6>().filter(|m| m.iter().all(|v| v.is_finite())) {
                        ctm = Matrix(m).then(&ctm);
                    }
                }
                b"m" => {
                    if let Some([x, y]) = op.nums::<2>() {
                        let (a, b) = ctm.apply(x, y);
                        end_sub(&mut p);
                        p.sub.push([a, b]);
                        p.start = Some([a, b]);
                        p.current = Some([a, b]);
                    }
                }
                b"l" => {
                    if let (Some(a), Some([x, y])) = (p.current, op.nums::<2>()) {
                        let (bx, by) = ctm.apply(x, y);
                        p.lines.push((a, [bx, by]));
                        if p.sub.len() < 6 {
                            p.sub.push([bx, by]);
                        } else {
                            p.sub.clear();
                        }
                        p.current = Some([bx, by]);
                    }
                }
                b"h" => {
                    if let (Some(a), Some(s)) = (p.current, p.start) {
                        p.lines.push((a, s));
                        p.current = Some(s);
                    }
                }
                b"c" | b"v" | b"y" => {
                    // A curve ends the straight run; its end point is where the path goes on.
                    p.sub.clear();
                    let n = if op.is("c") { 6 } else { 4 };
                    let v: Vec<f64> = op.operands.iter().filter_map(|o| o.as_f64()).collect();
                    if let (Some(x), Some(y)) = (v.get(n - 2), v.get(n - 1)) {
                        let (a, b) = ctm.apply(*x, *y);
                        p.current = Some([a, b]);
                    }
                }
                b"re" => {
                    end_sub(&mut p);
                    if let Some([x, y, wd, ht]) = op.nums::<4>() {
                        let corners = [(x, y), (x + wd, y), (x + wd, y + ht), (x, y + ht)].map(|(a, b)| {
                            let (u, v) = ctm.apply(a, b);
                            [u, v]
                        });
                        let [a, b, c, d] = corners;
                        // Axis-aligned after the CTM: kept as a rectangle; otherwise its edges.
                        if (a[1] - b[1]).abs() <= SKEW && (b[0] - c[0]).abs() <= SKEW {
                            p.rects.push([a[0].min(c[0]), a[1].min(c[1]), a[0].max(c[0]), a[1].max(c[1])]);
                        } else {
                            p.lines.extend([(a, b), (b, c), (c, d), (d, a)]);
                        }
                        p.start = Some(a);
                        p.current = Some(a);
                    }
                }
                b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*" => {
                    end_sub(&mut p);
                    // Stroked: every straight piece and every rectangle edge is a line.
                    let close = matches!(op.op.as_slice(), b"s" | b"b" | b"b*");
                    if let (true, Some(a), Some(s)) = (close, p.current, p.start) {
                        p.lines.push((a, s));
                    }
                    for (a, b) in p.lines.drain(..) {
                        push(&mut w.out, a, b);
                    }
                    for r in p.rects.drain(..) {
                        push(&mut w.out, [r[0], r[1]], [r[2], r[1]]);
                        push(&mut w.out, [r[0], r[3]], [r[2], r[3]]);
                        push(&mut w.out, [r[0], r[1]], [r[0], r[3]]);
                        push(&mut w.out, [r[2], r[1]], [r[2], r[3]]);
                    }
                    p = PathState::default();
                }
                b"f" | b"F" | b"f*" => {
                    end_sub(&mut p);
                    // Filled: a thin bar is a rule; a larger fill (a cell's shading) bounds the cell.
                    let rects: Vec<[f64; 4]> = p.rects.drain(..).chain(p.path_rects.drain(..)).collect();
                    for r in rects {
                        let (wd, ht) = (r[2] - r[0], r[3] - r[1]);
                        if ht <= THIN && wd >= MIN_LENGTH {
                            push(&mut w.out, [r[0], (r[1] + r[3]) / 2.0], [r[2], (r[1] + r[3]) / 2.0]);
                        } else if wd <= THIN && ht >= MIN_LENGTH {
                            push(&mut w.out, [(r[0] + r[2]) / 2.0, r[1]], [(r[0] + r[2]) / 2.0, r[3]]);
                        } else {
                            push(&mut w.out, [r[0], r[1]], [r[2], r[1]]);
                            push(&mut w.out, [r[0], r[3]], [r[2], r[3]]);
                            push(&mut w.out, [r[0], r[1]], [r[0], r[3]]);
                            push(&mut w.out, [r[2], r[1]], [r[2], r[3]]);
                        }
                    }
                    p = PathState::default();
                }
                b"n" => p = PathState::default(),
                b"Do" => {
                    let Some(name) = op.name(0) else { continue };
                    if let Some(form) = crate::text::form_call(w.doc, resources, name, ctm, &w.path, &mut w.visits) {
                        if let Some(r) = form.obj {
                            w.path.push(r);
                        }
                        walk(w, &form.data, &form.resources, form.ctm);
                        if form.obj.is_some() {
                            w.path.pop();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let p = pdfcraft_model::pages(doc).into_iter().nth(page).ok_or(EditError::NoSuchPage(page))?;
    let res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let mut w = Walk { doc, out: Vec::new(), path: Vec::new(), visits: 0 };
    // The page's streams are one stream in pieces (§7.8.2): read them as one.
    let mut data = Vec::new();
    for (_, d) in crate::images::page_streams(doc, &p.dict) {
        data.extend(d);
        data.push(b'\n');
    }
    walk(&mut w, &data, &res, Matrix::IDENTITY);
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(content: &str) -> Document {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /XObject << /Fm 5 0 R >> >> >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /XObject /Subtype /Form /BBox [0 0 600 800] /Length 25 >>\nstream\n100 100 m 300 100 l S\nendstream".into(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        Document::open(std::sync::Arc::new(pdf)).unwrap()
    }

    #[test]
    fn strokes_rectangles_bars_and_forms_become_segments() {
        let doc = page(
            "50 700 m 250 700 l S \
             q 2 0 0 2 0 0 cm 25 300 100 50 re S Q \
             50 500 200 0.5 re f \
             50 400 m 150 450 l S \
             /Fm Do",
        );
        let segs = ruling_segments(&doc, 0).unwrap();
        // A stroked line.
        assert!(segs.contains(&[50.0, 700.0, 250.0, 700.0]), "{segs:?}");
        // A rectangle stroked under a 2× CTM: its four edges.
        for s in [[50.0, 600.0, 250.0, 600.0], [50.0, 700.0, 250.0, 700.0], [50.0, 600.0, 50.0, 700.0], [250.0, 600.0, 250.0, 700.0]] {
            assert!(segs.contains(&s), "{s:?} in {segs:?}");
        }
        // A thin filled bar is one rule along its middle.
        assert!(segs.contains(&[50.0, 500.25, 250.0, 500.25]), "{segs:?}");
        // The diagonal line is not a rule; the form's line is.
        assert!(!segs.iter().any(|s| s[1] == 400.0 || s[3] == 450.0), "{segs:?}");
        assert!(segs.contains(&[100.0, 100.0, 300.0, 100.0]), "{segs:?}");
    }

    #[test]
    fn rectangles_drawn_as_subpaths_count_when_filled() {
        // A thin bar and a shaded band, each `m l l l h`, in one filled path (as office suites
        // draw table borders); a filled triangle is no rule.
        let doc = page("50 300 m 250 300 l 250 300.5 l 50 300.5 l h 50 200 m 250 200 l 250 220 l 50 220 l h f* 10 10 m 60 10 l 30 40 l h f");
        let segs = ruling_segments(&doc, 0).unwrap();
        assert!(segs.contains(&[50.0, 300.25, 250.0, 300.25]), "{segs:?}");
        for s in [[50.0, 200.0, 250.0, 200.0], [50.0, 220.0, 250.0, 220.0], [50.0, 200.0, 50.0, 220.0], [250.0, 200.0, 250.0, 220.0]] {
            assert!(segs.contains(&s), "{s:?} in {segs:?}");
        }
        assert!(!segs.iter().any(|s| s[1] < 50.0), "{segs:?}");
    }

    #[test]
    fn hostile_paths_are_bounded() {
        let mut content = String::new();
        for i in 0..30_000 {
            content.push_str(&format!("0 {i} m 100 {i} l "));
        }
        content.push('S');
        let doc = page(&content);
        assert_eq!(ruling_segments(&doc, 0).unwrap().len(), MAX_SEGMENTS);
        assert!(ruling_segments(&page("1 2 m 1e400 2 l S 0 0 m nan 0 l S"), 0).is_ok());
        assert!(matches!(ruling_segments(&doc, 5), Err(EditError::NoSuchPage(5))));
    }
}
