//! Sintec.PDF: the rules a page draws (a table's borders, a form's field lines), as editing reads
//! them: text on either side of one is never one line or one paragraph, however close.

use super::Document;
use super::interp::to_frame;

/// A rule as a straight segment in user space, `[x0, y0, x1, y1]`: horizontal (`y0 == y1`) or
/// vertical (`x0 == x1`), with `x0 <= x1` and `y0 <= y1`.
pub(crate) type Segment = [f64; 4];

/// Page `page`'s rules ([`crate::page_rules`]), each along its middle: a thin filled bar is one
/// rule there. None when the page's content can't all be read.
pub(crate) fn page_segments(doc: &Document, page: usize) -> Vec<Segment> {
    crate::rules::page_rules(doc, page)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|[x0, y0, x1, y1]| {
            let (w, h) = (x1 - x0, y1 - y0);
            if !(w.is_finite() && h.is_finite()) {
                return None;
            }
            Some(if w >= h {
                let y = (y0 + y1) / 2.0;
                [x0, y, x1, y]
            } else {
                let x = (x0 + x1) / 2.0;
                [x, y0, x, y1]
            })
        })
        .collect()
}

/// Rule `s` in the frame of text running in direction `dir`: `Some((true, at, [lo, hi]))` when it
/// crosses the baseline direction (at `along` = `at`, spanning `across` from `lo` to `hi`),
/// `Some((false, at, [lo, hi]))` when it runs along it (at `across` = `at`), `None` when it is
/// slanted in that frame or not finite.
pub(super) fn rule_in_frame(dir: (f64, f64), s: &Segment) -> Option<(bool, f64, [f64; 2])> {
    let a = to_frame(dir, (s[0], s[1]));
    let b = to_frame(dir, (s[2], s[3]));
    if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (d_along, d_across) = ((b.0 - a.0).abs(), (b.1 - a.1).abs());
    if d_along <= 1.0 && d_across > d_along {
        Some((true, (a.0 + b.0) / 2.0, [a.1.min(b.1), a.1.max(b.1)]))
    } else if d_across <= 1.0 && d_along > d_across {
        Some((false, (a.1 + b.1) / 2.0, [a.0.min(b.0), a.0.max(b.0)]))
    } else {
        None
    }
}

/// Whether one of `rules` crosses the baseline direction `dir` between `end` (where a run of text
/// ends along it) and `start` (where the next begins), give or take `slack`, through the letters
/// of both: across the baseline, it reaches every one of `heights` (each run's baseline plus a
/// third of its size). As a table's column border between two cells does.
pub(super) fn ruled_apart_along(rules: &[Segment], dir: (f64, f64), end: f64, start: f64, slack: f64, heights: [f64; 2]) -> bool {
    let (from, to) = (end.min(start) - slack, end.max(start) + slack);
    rules
        .iter()
        .filter_map(|s| rule_in_frame(dir, s))
        .any(|(crossing, at, [lo, hi])| crossing && at >= from && at <= to && heights.iter().all(|h| lo <= *h && hi >= *h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_between_two_runs_keeps_them_apart() {
        let upright = (1.0, 0.0);
        // A column border at x = 100 from y = 90 to 120, between text ending at 98 and starting
        // at 103 on a baseline at 100 (letters up to 103).
        let border = [100.0, 90.0, 100.0, 120.0];
        assert!(ruled_apart_along(&[border], upright, 98.0, 103.0, 1.0, [103.0, 103.0]));
        // Too short to reach the letters, or outside the gap: no.
        assert!(!ruled_apart_along(&[[100.0, 105.0, 100.0, 120.0]], upright, 98.0, 103.0, 1.0, [103.0, 103.0]));
        assert!(!ruled_apart_along(&[[150.0, 90.0, 150.0, 120.0]], upright, 98.0, 103.0, 1.0, [103.0, 103.0]));
        // An underline runs along the text, not across it.
        assert!(!ruled_apart_along(&[[90.0, 98.0, 110.0, 98.0]], upright, 98.0, 103.0, 1.0, [103.0, 103.0]));
        assert_eq!(rule_in_frame(upright, &[0.0, f64::NAN, 0.0, 1.0]), None);
    }
}
