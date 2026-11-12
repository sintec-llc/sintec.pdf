//! Sintec.PDF: tables drawn with lines (ruled tables, the "lattice" kind), read from the page's
//! horizontal and vertical lines ([`Page::rules`]) and placed text lines ([`Page::lines`]).
//!
//! Lines that touch form a table's frame. Its distinct line positions are the grid; a grid
//! boundary with no line drawn along it (most of its length) merges the cells on either side, so
//! a header spanning columns or a label spanning rows becomes one cell. Each text line goes into
//! the cell its centre falls in. Tables found from text alignment ([`crate::tables`]) cover what
//! has no lines.

use crate::{Block, Cell, Page, Table};

/// Lines this close (points) are the same grid line; ends this close still touch.
const TOL: f64 = 2.0;
/// A boundary counts as drawn when lines cover this share of it.
const DRAWN: f64 = 0.6;
/// The most segments considered on a page, the most cells a table may have, and the most
/// touching tests run: a page of hatching or a hostile file stays cheap.
const MAX_SEGMENTS: usize = 6_000;
const MAX_CELLS: usize = 20_000;
const MAX_PAIRS: usize = 4_000_000;
/// Word's limit (see [`crate::MAX_COLS`]).
const MAX_COLS: usize = crate::MAX_COLS;
/// A merged cell's extent: first and last row, first and last column, and how many grid cells.
type Extent = (usize, usize, usize, usize, usize);

#[derive(Clone, Copy, Debug)]
struct Line {
    /// Horizontal: y, from x0 to x1. Vertical: x, from y0 to y1 (stored the same way).
    at: f64,
    from: f64,
    to: f64,
}

/// Merge collinear lines that overlap or nearly meet.
fn merge(mut ls: Vec<Line>) -> Vec<Line> {
    ls.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
    let mut out: Vec<Line> = Vec::new();
    for l in ls {
        match out.iter_mut().rev().take_while(|m| (m.at - l.at).abs() <= TOL).find(|m| l.from <= m.to + TOL && l.to >= m.from - TOL) {
            Some(m) => {
                m.from = m.from.min(l.from);
                m.to = m.to.max(l.to);
            }
            None => out.push(l),
        }
    }
    out
}

/// Positions within [`TOL`] of each other, as one (their mean).
fn cluster(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    let mut out: Vec<(f64, usize)> = Vec::new();
    for x in v {
        match out.last_mut() {
            Some((m, n)) if (x - *m).abs() <= TOL => {
                *m = (*m * *n as f64 + x) / (*n + 1) as f64;
                *n += 1;
            }
            _ => out.push((x, 1)),
        }
    }
    out.into_iter().map(|(m, _)| m).collect()
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = gp;
        }
        i = p;
    }
    i
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb
        && let Some(slot) = parent.get_mut(ra.max(rb))
    {
        *slot = ra.min(rb);
    }
}

/// How much of `[from, to]` the lines at position `at` (within [`TOL`]) cover, as a share.
fn coverage(lines: &[Line], at: f64, from: f64, to: f64) -> f64 {
    let len = (to - from).max(1e-6);
    let mut pieces: Vec<(f64, f64)> =
        lines.iter().filter(|l| (l.at - at).abs() <= TOL).map(|l| (l.from.max(from), l.to.min(to))).filter(|(a, b)| b > a).collect();
    pieces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut covered = 0.0;
    let mut end = from;
    for (a, b) in pieces {
        let a = a.max(end);
        if b > a {
            covered += b - a;
            end = b;
        }
    }
    covered / len
}

/// The ruled tables on `page`, and the boxes they cover (to leave their text out of the
/// paragraph flow).
pub(crate) fn ruled_tables(page: &Page) -> Vec<Table> {
    let mut hs = Vec::new();
    let mut vs = Vec::new();
    for s in page.rules.iter().take(MAX_SEGMENTS) {
        if !s.iter().all(|v| v.is_finite()) {
            continue;
        }
        let [x0, y0, x1, y1] = *s;
        if (y1 - y0).abs() <= TOL && (x1 - x0).abs() > TOL {
            hs.push(Line { at: (y0 + y1) / 2.0, from: x0.min(x1), to: x0.max(x1) });
        } else if (x1 - x0).abs() <= TOL && (y1 - y0).abs() > TOL {
            vs.push(Line { at: (x0 + x1) / 2.0, from: y0.min(y1), to: y0.max(y1) });
        }
    }
    let (hs, vs) = (merge(hs), merge(vs));
    if hs.len() < 2 || vs.len() < 2 || hs.len().saturating_mul(vs.len()) > MAX_PAIRS {
        return Vec::new();
    }
    // Frames: lines that touch, joined (horizontal i is i, vertical j is hs.len() + j).
    let mut parent: Vec<usize> = (0..hs.len() + vs.len()).collect();
    for (i, h) in hs.iter().enumerate() {
        for (j, v) in vs.iter().enumerate() {
            let touches = v.at >= h.from - TOL && v.at <= h.to + TOL && h.at >= v.from - TOL && h.at <= v.to + TOL;
            if touches {
                union(&mut parent, i, hs.len() + j);
            }
        }
    }
    let mut groups: Vec<(usize, Vec<Line>, Vec<Line>)> = Vec::new();
    for (i, h) in hs.iter().enumerate() {
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|g| g.0 == root) {
            Some(g) => g.1.push(*h),
            None => groups.push((root, vec![*h], Vec::new())),
        }
    }
    for (j, v) in vs.iter().enumerate() {
        let root = find(&mut parent, hs.len() + j);
        if let Some(g) = groups.iter_mut().find(|g| g.0 == root) {
            g.2.push(*v);
        }
    }
    groups.into_iter().filter_map(|(_, h, v)| table_from_frame(&h, &v, &page.lines)).collect()
}

/// One frame of touching lines as a table, if it is one: at least two rows or columns of cells,
/// with text in some of them.
fn table_from_frame(hs: &[Line], vs: &[Line], lines: &[Block]) -> Option<Table> {
    if hs.len() < 2 || vs.len() < 2 {
        return None;
    }
    let xs = cluster(vs.iter().map(|v| v.at).collect());
    let mut ys = cluster(hs.iter().map(|h| h.at).collect());
    ys.reverse(); // top to bottom
    let (ncols, nrows) = (xs.len().checked_sub(1)?, ys.len().checked_sub(1)?);
    if ncols == 0 || nrows == 0 || ncols > MAX_COLS || ncols.saturating_mul(nrows) > MAX_CELLS || ncols * nrows < 2 {
        return None;
    }
    let (left, right, top, bottom) = (*xs.first()?, *xs.last()?, *ys.first()?, *ys.last()?);
    if right - left < 20.0 || top - bottom < 8.0 {
        return None;
    }
    let idx = |r: usize, c: usize| r * ncols + c;
    let mut parent: Vec<usize> = (0..nrows * ncols).collect();
    // Within each row, no line along a cell's right boundary: it runs on into the next column.
    // (Indexing below stays within the grid: r < nrows = ys.len() - 1, c < ncols = xs.len() - 1.)
    for r in 0..nrows {
        let (y_top, y_bottom) = (ys[r], ys[r + 1]);
        for c in 0..ncols.saturating_sub(1) {
            if coverage(vs, xs[c + 1], y_bottom, y_top) < DRAWN {
                union(&mut parent, idx(r, c), idx(r, c + 1));
            }
        }
    }
    // Each row's runs of joined cells, as (first column, last column) per grid cell.
    let mut runs = vec![(0usize, 0usize); nrows * ncols];
    for r in 0..nrows {
        let mut c = 0;
        while c < ncols {
            let start = c;
            let root = find(&mut parent, idx(r, c));
            while c + 1 < ncols && find(&mut parent, idx(r, c + 1)) == root {
                c += 1;
            }
            for k in start..=c {
                runs[idx(r, k)] = (start, c);
            }
            c += 1;
        }
    }
    // No line along a run's bottom: it runs on into the run below, when that one covers the same
    // columns. Lines that would merge cells into other shapes are drawn imperfectly; the cells
    // stay apart then, so every merged cell is a rectangle.
    for r in 0..nrows.saturating_sub(1) {
        let mut c = 0;
        while c < ncols {
            let (c0, c1) = runs[idx(r, c)];
            if runs[idx(r + 1, c0)] == (c0, c1) && coverage(hs, ys[r + 1], xs[c0], xs[c1 + 1]) < DRAWN {
                union(&mut parent, idx(r, c0), idx(r + 1, c0));
            }
            c = c1 + 1;
        }
    }
    // One cell that is the whole frame is a box around something, not a table.
    let root0 = find(&mut parent, 0);
    if (0..nrows * ncols).all(|i| find(&mut parent, i) == root0) {
        return None;
    }
    // Each merged region's extent (a rectangle, by construction; checked all the same).
    let mut extent: Vec<Option<Extent>> = vec![None; nrows * ncols];
    for r in 0..nrows {
        for c in 0..ncols {
            let root = find(&mut parent, idx(r, c));
            let e = extent.get_mut(root)?;
            *e = Some(match *e {
                None => (r, r, c, c, 1),
                Some((r0, r1, c0, c1, n)) => (r0.min(r), r1.max(r), c0.min(c), c1.max(c), n + 1),
            });
        }
    }
    if extent.iter().flatten().any(|&(r0, r1, c0, c1, n)| (r1 - r0 + 1) * (c1 - c0 + 1) != n) {
        return None;
    }
    // The text of each region: the lines whose centres fall in it, top to bottom, left to right.
    let mut texts: Vec<Vec<&Block>> = vec![Vec::new(); nrows * ncols];
    for l in lines {
        let (cx, cy) = ((l.rect[0] + l.rect[2]) / 2.0, (l.rect[1] + l.rect[3]) / 2.0);
        if !(cx > left && cx < right && cy < top && cy > bottom) || l.text.trim().is_empty() {
            continue;
        }
        let Some(c) = xs.windows(2).position(|w| cx >= w[0] && cx < w[1]) else { continue };
        let Some(r) = ys.windows(2).position(|w| cy <= w[0] && cy > w[1]) else { continue };
        let root = find(&mut parent, idx(r, c));
        if let Some(t) = texts.get_mut(root) {
            t.push(l);
        }
    }
    if texts.iter().all(Vec::is_empty) {
        return None;
    }
    let mut rows = Vec::with_capacity(nrows);
    for r in 0..nrows {
        let mut row = Vec::new();
        let mut c = 0;
        while c < ncols {
            let root = find(&mut parent, idx(r, c));
            let Some((r0, r1, c0, c1, _)) = extent.get(root).copied().flatten() else { break };
            let span = c1 - c0 + 1;
            if r == r0 {
                let parts = texts.get(root).cloned().unwrap_or_default();
                row.push(cell_of(&parts, span, r1 - r0 + 1));
            } else {
                row.push(Cell { span, continued: true, ..Cell::empty() });
            }
            c += span;
        }
        rows.push(row);
    }
    Some(Table { rect: [left, bottom, right, top], cols: xs[..ncols].to_vec(), rows, ruled: true, row_edges: ys })
}

/// A cell from its words or text lines: pieces whose centres sit at one height (within half
/// their size) make a line, read left to right and joined by spaces; lines go top to bottom on
/// new lines. The cell takes the style of the text that makes up most of it.
fn cell_of(parts: &[&Block], span: usize, rowspan: usize) -> Cell {
    let mid = |b: &Block| (b.rect[1] + b.rect[3]) / 2.0;
    let mut sorted: Vec<&Block> = parts.to_vec();
    sorted.sort_by(|a, b| mid(b).total_cmp(&mid(a)));
    let mut lines: Vec<(f64, Vec<&Block>)> = Vec::new();
    for b in sorted {
        match lines.last_mut() {
            Some((y, l)) if (*y - mid(b)).abs() < b.size.max(1.0) * 0.5 => l.push(b),
            _ => lines.push((mid(b), vec![b])),
        }
    }
    let text = lines
        .iter_mut()
        .map(|(_, l)| {
            l.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
            l.iter().map(|b| b.text.trim()).collect::<Vec<_>>().join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let chars = |f: &dyn Fn(&Block) -> bool| parts.iter().filter(|l| f(l)).map(|l| l.text.len()).sum::<usize>();
    let total = chars(&|_| true).max(1);
    let first = lines.first().and_then(|(_, l)| l.first());
    Cell {
        span,
        rowspan,
        ..Cell::new(
            text,
            first.map_or(11.0, |l| l.size),
            chars(&|l| l.bold) * 2 > total,
            chars(&|l| l.italic) * 2 > total,
            first.map_or([0.0; 3], |l| l.color),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: f64, y: f64) -> Block {
        Block {
            text: text.into(),
            rect: [x, y, x + 6.0 * text.len() as f64, y + 10.0],
            size: 10.0,
            bold: false,
            italic: false,
            color: [0.0; 3],
            rotation: 0.0,
        }
    }

    /// A 3-column, 3-row grid at x 50/150/250/350 and y 700/680/660/640, with `skip` grid lines
    /// left out.
    fn grid(skip: &[[f64; 4]]) -> Vec<[f64; 4]> {
        let mut r = Vec::new();
        for y in [700.0, 680.0, 660.0, 640.0] {
            r.push([50.0, y, 350.0, y]);
        }
        for x in [50.0, 150.0, 250.0, 350.0] {
            r.push([x, 640.0, x, 700.0]);
        }
        r.retain(|s| !skip.contains(s));
        r
    }

    fn texts(t: &Table) -> Vec<Vec<String>> {
        t.rows.iter().map(|r| r.iter().map(|c| if c.continued { "^".to_string() } else { c.text.clone() }).collect()).collect()
    }

    #[test]
    fn a_ruled_grid_puts_each_line_in_its_cell() {
        let lines = vec![
            line("Name", 55.0, 685.0),
            line("Qty", 155.0, 685.0),
            line("Price", 255.0, 685.0),
            line("Paper", 55.0, 665.0),
            line("10", 155.0, 665.0),
            line("1 250,50", 255.0, 665.0),
            line("Toner", 55.0, 645.0),
            line("2", 155.0, 645.0),
            line("3 400", 255.0, 645.0),
            line("Outside", 400.0, 500.0),
        ];
        let page = Page { rules: grid(&[]), lines, ..Page::default() };
        let tables = ruled_tables(&page);
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert!(t.ruled && t.cols == [50.0, 150.0, 250.0] && t.row_edges == [700.0, 680.0, 660.0, 640.0]);
        assert_eq!(texts(t), [["Name", "Qty", "Price"], ["Paper", "10", "1 250,50"], ["Toner", "2", "3 400"]]);
    }

    #[test]
    fn missing_lines_merge_cells_across_columns_and_rows() {
        // The header's two inner lines left out: one cell over all three columns. The first
        // column's line between rows 2 and 3 left out: one cell over two rows.
        let page = Page {
            rules: {
                let mut r = grid(&[[150.0, 640.0, 150.0, 700.0], [250.0, 640.0, 250.0, 700.0], [50.0, 660.0, 350.0, 660.0]]);
                // Inner verticals drawn below the header only; the row line drawn right of column 1.
                r.extend([[150.0, 640.0, 150.0, 680.0], [250.0, 640.0, 250.0, 680.0], [150.0, 660.0, 350.0, 660.0]]);
                r
            },
            lines: vec![line("Invoice 15", 55.0, 685.0), line("Paper", 55.0, 665.0), line("10", 155.0, 665.0), line("2", 155.0, 645.0)],
            ..Page::default()
        };
        let t = &ruled_tables(&page)[0];
        assert_eq!(t.rows[0][0].span, 3);
        assert_eq!(t.rows[1][0].rowspan, 2);
        assert_eq!(texts(t), [vec!["Invoice 15"], vec!["Paper", "10", ""], vec!["^", "2", ""]]);
    }

    #[test]
    fn per_row_lines_a_little_apart_still_divide_every_row() {
        // Office suites draw each row's cell borders on their own, half a point apart from the
        // row above; the lower rows' lines must not fold into the upper rows' ones.
        let mut rules = vec![[50.0, 700.0, 350.0, 700.0], [50.0, 680.0, 350.0, 680.0], [50.0, 660.0, 350.0, 660.0]];
        for x in [50.0, 150.0, 350.0] {
            rules.push([x, 680.0, x, 700.0]);
            rules.push([x + 0.5, 659.5, x + 0.5, 680.5]);
        }
        let page = Page {
            rules,
            lines: vec![line("A", 55.0, 685.0), line("B", 155.0, 685.0), line("1", 55.0, 665.0), line("2", 155.0, 665.0)],
            ..Page::default()
        };
        assert_eq!(texts(&ruled_tables(&page)[0]), [["A", "B"], ["1", "2"]]);
    }

    #[test]
    fn imperfect_lines_split_cells_instead_of_losing_the_table() {
        // The line under row 1 is missing only in column 1, while row 2 joins columns 1 and 2:
        // merging would make an L shape, so the cells stay apart.
        let mut rules = grid(&[[50.0, 680.0, 350.0, 680.0], [150.0, 640.0, 150.0, 700.0]]);
        rules.extend([[150.0, 680.0, 350.0, 680.0], [150.0, 680.0, 150.0, 700.0], [150.0, 640.0, 150.0, 660.0]]);
        let page = Page { rules, lines: vec![line("x", 55.0, 685.0), line("y", 55.0, 665.0)], ..Page::default() };
        let t = &ruled_tables(&page)[0];
        assert_eq!(texts(t)[0][0], "x");
        assert_eq!(t.rows[1][0].span, 2);
        assert!(!t.rows[1][0].continued);
    }

    #[test]
    fn lines_that_are_not_a_grid_make_no_table() {
        // An underline, a lone box around a paragraph and a page border make no tables.
        let underline = Page { rules: vec![[50.0, 600.0, 300.0, 600.0]], lines: vec![line("Signature", 50.0, 602.0)], ..Page::default() };
        assert!(ruled_tables(&underline).is_empty());
        let boxed = Page {
            rules: vec![[50.0, 500.0, 300.0, 500.0], [50.0, 600.0, 300.0, 600.0], [50.0, 500.0, 50.0, 600.0], [300.0, 500.0, 300.0, 600.0]],
            lines: vec![line("A framed note", 60.0, 550.0)],
            ..Page::default()
        };
        assert!(ruled_tables(&boxed).is_empty(), "one cell is a frame, not a table");
        let empty_grid = Page { rules: grid(&[]), ..Page::default() };
        assert!(ruled_tables(&empty_grid).is_empty(), "a grid with no text is a drawing");
    }

    #[test]
    fn hostile_rules_stay_bounded_and_never_panic() {
        let mut rules = Vec::new();
        for i in 0..5_000 {
            let v = f64::from(i);
            rules.push([0.0, v, 5_000.0, v]);
            rules.push([v, 0.0, v, 5_000.0]);
        }
        rules.push([f64::NAN, 1.0, 2.0, 1.0]);
        rules.push([f64::INFINITY, 0.0, f64::INFINITY, 10.0]);
        let page = Page { rules, lines: vec![line("x", 10.0, 10.0)], ..Page::default() };
        let _ = ruled_tables(&page);
    }
}

#[cfg(test)]
mod writer_tests {
    use super::*;
    use crate::tests_support::unzip;

    /// A ruled invoice table: a header cell over all three columns, a label over two rows, a
    /// two-line cell; and a paragraph above it.
    fn page() -> Page {
        let line = |text: &str, x: f64, y: f64| Block {
            text: text.into(),
            rect: [x, y, x + 6.0 * text.chars().count() as f64, y + 10.0],
            size: 10.0,
            bold: text == "Invoice 15",
            italic: false,
            color: [0.0; 3],
            rotation: 0.0,
        };
        let mut rules = Vec::new();
        for y in [700.0, 640.0] {
            rules.push([50.0, y, 350.0, y]);
        }
        rules.extend([[50.0, 680.0, 350.0, 680.0], [150.0, 660.0, 350.0, 660.0]]);
        rules.extend([[50.0, 640.0, 50.0, 700.0], [350.0, 640.0, 350.0, 700.0], [150.0, 640.0, 150.0, 680.0], [250.0, 640.0, 250.0, 680.0]]);
        let lines = vec![
            line("Invoice 15", 55.0, 685.0),
            line("Paper", 55.0, 665.0),
            line("10", 155.0, 665.0),
            line("1 250,50", 255.0, 669.0),
            line("incl. VAT", 255.0, 661.0),
            line("2", 155.0, 645.0),
        ];
        let blocks = vec![line("Supplier: Example Ltd", 50.0, 720.0), line("Invoice 15", 55.0, 685.0), line("Paper", 55.0, 665.0)];
        Page { width: 400.0, height: 800.0, blocks, rules, lines, ..Page::default() }
    }

    #[test]
    fn ruled_tables_keep_borders_merges_and_lines_in_every_format() {
        let pages = [page()];
        let t = ruled_tables(&pages[0]).remove(0);
        assert_eq!(t.rows[1][2].text, "1 250,50\nincl. VAT");

        let word = unzip(&crate::docx(&pages, "t")).into_iter().find(|f| f.0 == "word/document.xml").unwrap().1;
        assert!(word.contains("<w:insideH w:val=\"single\""), "{word}");
        assert!(word.contains("<w:gridSpan w:val=\"3\"/>"), "{word}");
        assert!(word.contains("<w:vMerge w:val=\"restart\"/>") && word.contains("<w:vMerge/>"), "{word}");
        assert!(word.contains("<w:trHeight w:val=\"400\" w:hRule=\"atLeast\"/>"), "{word}");
        assert!(word.contains(">1 250,50</w:t>") && word.contains(">incl. VAT</w:t>"), "{word}");
        // The table's text isn't repeated as paragraphs; the paragraph above it stays.
        assert_eq!(word.matches(">Paper</w:t>").count(), 1, "{word}");
        assert!(word.contains(">Supplier: Example Ltd</w:t>"), "{word}");

        let sheet = unzip(&crate::xlsx(&pages, "t", "Page")).into_iter().find(|f| f.0 == "xl/worksheets/sheet1.xml").unwrap().1;
        assert!(sheet.contains("<mergeCell ref=\"A3:C3\"/>") && sheet.contains("<mergeCell ref=\"A4:A5\"/>"), "{sheet}");
        assert!(sheet.contains("customHeight=\"1\""), "{sheet}");
        assert!(sheet.contains("<c r=\"C5\" s=\"3\""), "empty ruled cells keep their borders: {sheet}");

        let h = crate::html(&pages, "t");
        assert!(h.contains("<td colspan=\"3\"><strong>Invoice 15</strong></td>") && h.contains("rowspan=\"2\""), "{h}");
        assert!(h.contains("1 250,50<br>incl. VAT"), "{h}");

        let r = crate::rtf(&pages);
        assert!(r.contains("\\clvmgf\\clbrdrt") && r.contains("\\clvmrg"), "{r}");
        assert!(r.contains("1 250,50\\line incl. VAT"), "{r}");

        let slide = unzip(&crate::pptx(&pages, "t")).into_iter().find(|f| f.0 == "ppt/slides/slide1.xml").unwrap().1;
        assert!(slide.contains("rowSpan=\"2\"") && slide.contains("vMerge=\"1\""), "{slide}");
    }
}
