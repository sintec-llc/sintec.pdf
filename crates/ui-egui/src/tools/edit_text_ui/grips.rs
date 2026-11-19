//! Sintec.PDF: where a paragraph box is grabbed (tasks 003, 011, 013, 015). Each box spans where
//! its text wraps ([`frame`]); a move grip sits just above its top-left corner, and wide width
//! handles reach outside its left and right edges (its top and bottom on a quarter-turned page).
//! The box open for typing keeps its grip and handles, grabbed from outside its text.

use egui::{Pos2, Rect};
use pdfcraft_render::DocInfo;

use super::Grip;
use crate::canvas::PageXform;

/// How far an edge grip reaches outside a box's edge (screen pixels); with [`EDGE_INSIDE`] the
/// grabbable strip is 22 px wide, so it is caught without aiming for the hairline (task 011).
const EDGE_REACH: f32 = 14.0;
/// How far it reaches inside the edge (a narrow box keeps its middle for moving), and above and
/// below the box (#2203): no further, so a press just above a paragraph near its corner is the
/// page's (an area selection), not a rewrap.
const EDGE_INSIDE: f32 = 8.0;
/// The narrowest a box can be dragged to (screen pixels).
pub(super) const MIN_BOX_WIDTH: f32 = 12.0;
/// Side of the visible move grip (screen pixels).
pub(super) const MOVE_GRIP: f32 = 18.0;
/// How far the grip's invisible hit area extends beyond what is drawn (24 px in all).
const MOVE_GRIP_SLOP: f32 = 3.0;

/// A paragraph box's frame (user space): its rows' height, and across, where its text wraps
/// ([`pdfcraft_engine::TextBlock::wrap`], or a width kept from a handle drag), so the box shows
/// where typed text will wrap, not just its ink.
pub(crate) fn frame(b: &pdfcraft_engine::TextBlock) -> [f64; 4] {
    frame_of(b.rect, b.wrap)
}

/// [`frame`] of a paragraph whose ink is `rect` and which wraps between `wrap` (user space).
fn frame_of(rect: [f64; 4], wrap: [f64; 2]) -> [f64; 4] {
    // A wrap that isn't a number would make the box span nothing (or everything).
    if !wrap.iter().all(|v| v.is_finite()) {
        return rect;
    }
    [wrap[0].min(rect[0]), rect[1], wrap[1].max(rect[2]), rect[3]]
}

/// A rect with its axes swapped: width handles are written for text running left to right and
/// reach a quarter-turned page's boxes through this.
pub(super) fn transposed(r: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(r.min.y, r.min.x), Pos2::new(r.max.y, r.max.x))
}

fn transposed_pos(p: Pos2) -> Pos2 {
    Pos2::new(p.y, p.x)
}

/// Does the paragraph's width run along the screen's vertical here? True on a page turned by a
/// quarter (its own /Rotate plus the view's), where the text's left and right edges are the box's
/// top and bottom.
pub(crate) fn width_is_vertical(xf: &PageXform, info: &DocInfo, page: usize) -> bool {
    let turn = info.pages.get(page).map_or(0, |p| u32::from(p.rotation)) + u32::from(xf.rot);
    !turn.is_multiple_of(180)
}

/// The rewrap handles drawn on a paragraph box's width edges (screen): left and right, or top and
/// bottom when the width runs vertically.
pub(super) fn width_handles(b: Rect, vertical: bool) -> [Rect; 2] {
    let t = if vertical { transposed(b) } else { b };
    let h = (t.height() * 0.5).clamp(24.0, 40.0);
    [t.left(), t.right()].map(|x| {
        let r = Rect::from_center_size(Pos2::new(x, t.center().y), egui::vec2(8.0, h));
        if vertical { transposed(r) } else { r }
    })
}

/// The move grip of a paragraph box: just above its top-left corner so it stays off the text;
/// inside the page's top edge (`bounds`) when the box sits against it.
pub(super) fn move_handle(b: Rect, bounds: Rect) -> Rect {
    let top = if b.top() - MOVE_GRIP - 1.0 < bounds.top() { b.top() + 1.0 } else { b.top() - MOVE_GRIP - 1.0 };
    Rect::from_min_size(Pos2::new(b.left(), top), egui::vec2(MOVE_GRIP, MOVE_GRIP))
}

/// The box whose move grip (hit area included) is under `p`, the topmost first. Only the open box
/// (`active`) has one while a paragraph is open for typing. A grip gives way to another paragraph's
/// text it lies over: a press on that text is that paragraph's.
pub(super) fn handle_at(boxes: &[Rect], bounds: Rect, p: Pos2, active: Option<usize>) -> Option<usize> {
    boxes
        .iter()
        .enumerate()
        .rev()
        .filter(|(i, _)| active.is_none_or(|a| a == *i))
        .find(|(i, b)| {
            move_handle(**b, bounds).expand(MOVE_GRIP_SLOP).contains(p) && !boxes.iter().enumerate().any(|(j, o)| j != *i && o.contains(p))
        })
        .map(|(i, _)| i)
}

/// The edge of `b` that `p` grabs, and how far `p` is from it: anywhere along its height, within
/// reach of the line. A narrow box keeps its middle for moving. A box open for typing (`editing`)
/// owns its inside (the text editor sits there), so its edges are grabbed from outside only.
fn edge_at(b: Rect, p: Pos2, editing: bool, vertical: bool) -> Option<(Grip, f32)> {
    let (b, p) = if vertical { (transposed(b), transposed_pos(p)) } else { (b, p) };
    if p.y < b.top() - EDGE_INSIDE || p.y > b.bottom() + EDGE_INSIDE {
        return None;
    }
    let inside = if editing { 0.0 } else { EDGE_INSIDE.min(b.width() / 4.0) };
    let (dl, dr) = (p.x - b.left(), b.right() - p.x);
    match (dl >= -EDGE_REACH && dl <= inside, dr >= -EDGE_REACH && dr <= inside) {
        (true, true) if dl <= dr => Some((Grip::Left, dl.abs())),
        (_, true) => Some((Grip::Right, dr.abs())),
        (true, false) => Some((Grip::Left, dl.abs())),
        _ => None,
    }
}

/// The box under `p` and what it grabs there. `Grip::Left` is the box's edge where the text starts
/// on screen (its left, or its top when the width runs vertically). Edges count when `edges` is
/// set; while a paragraph is open for typing (`active`), only that box's, from outside it.
/// An edge reaches a little above and below its box, so where that reach overlaps another box the
/// one `p` is level with wins: a press beside one paragraph's edge doesn't rewrap the paragraph
/// just under it. Then the nearest edge wins (the boxes of neighbouring table cells sit close
/// enough for their grab strips to overlap), and an edge wins over a box `p` is inside, but not
/// from outside its own box: there another paragraph's text is that paragraph's. Among boxes
/// equally near, the topmost.
pub(super) fn grab_at(boxes: &[Rect], p: Pos2, edges: bool, vertical: bool, active: Option<usize>) -> Option<(usize, Grip)> {
    let mut best: Option<((f32, f32), usize, Grip)> = None;
    let in_other = |i: usize| boxes.iter().enumerate().any(|(j, o)| j != i && o.contains(p));
    for (i, b) in boxes.iter().enumerate().rev() {
        let edge = (edges && active.is_none_or(|a| a == i) && (b.contains(p) || !in_other(i)))
            .then(|| edge_at(*b, p, active == Some(i), vertical))
            .flatten();
        let (grip, d) = match edge {
            Some(e) => e,
            None if b.contains(p) => (Grip::Move, f32::INFINITY),
            None => continue,
        };
        // How far above or below the box `p` is (across the text's rows): 0 level with it.
        let t = if vertical { transposed(*b) } else { *b };
        let q = if vertical { transposed_pos(p) } else { p };
        let off = (t.top() - q.y).max(q.y - t.bottom()).max(0.0);
        if best.is_none_or(|(k, ..)| (off, d) < k) {
            best = Some(((off, d), i, grip));
        }
    }
    best.map(|(_, i, g)| (i, g))
}

/// Where a press on the box open for typing grabs it (grip and width edges, screen). The Format
/// text panel keeps clear of these where it can.
pub(super) fn grab_zones(b: Rect, bounds: Rect, vertical: bool) -> Vec<Rect> {
    let t = if vertical { transposed(b) } else { b };
    let side = |x0: f32, x1: f32| {
        let r = Rect::from_min_max(Pos2::new(x0, t.top() - EDGE_INSIDE), Pos2::new(x1, t.bottom() + EDGE_INSIDE));
        if vertical { transposed(r) } else { r }
    };
    vec![move_handle(b, bounds).expand(MOVE_GRIP_SLOP), side(t.left() - EDGE_REACH, t.left()), side(t.right(), t.right() + EDGE_REACH)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
    }

    #[test]
    fn the_grip_sits_above_the_box_or_inside_the_page_top() {
        let page = r(0.0, 0.0, 600.0, 800.0);
        let g = move_handle(r(100.0, 200.0, 300.0, 260.0), page);
        assert!(g.bottom() < 200.0 && g.left() == 100.0, "{g:?}");
        let g = move_handle(r(100.0, 5.0, 300.0, 60.0), page);
        assert!(g.top() > 5.0, "{g:?}");
    }

    #[test]
    fn a_grip_gives_way_to_another_paragraphs_text() {
        let page = r(0.0, 0.0, 600.0, 800.0);
        let (upper, lower) = (r(100.0, 160.0, 300.0, 190.0), r(100.0, 200.0, 300.0, 260.0));
        let on_grip = Pos2::new(109.0, 185.0);
        assert_eq!(handle_at(&[lower], page, on_grip, None), Some(0));
        assert_eq!(handle_at(&[upper, lower], page, on_grip, None), None);
        // While a paragraph is open, only its grip counts.
        assert_eq!(handle_at(&[lower], page, on_grip, Some(5)), None);
    }

    #[test]
    fn the_nearer_edge_of_neighbouring_cells_wins() {
        let (a, b) = (r(100.0, 100.0, 200.0, 130.0), r(210.0, 100.0, 300.0, 130.0));
        assert_eq!(grab_at(&[a, b], Pos2::new(203.0, 115.0), true, false, None), Some((0, Grip::Right)));
        assert_eq!(grab_at(&[a, b], Pos2::new(208.0, 115.0), true, false, None), Some((1, Grip::Left)));
        assert_eq!(grab_at(&[a, b], Pos2::new(150.0, 115.0), true, false, None), Some((0, Grip::Move)));
        // The open box is grabbed from outside only, and no other box has edges then.
        assert_eq!(grab_at(&[a, b], Pos2::new(195.0, 115.0), true, false, Some(0)), Some((0, Grip::Move)));
        assert_eq!(grab_at(&[a, b], Pos2::new(205.0, 115.0), true, false, Some(0)), Some((0, Grip::Right)));
        assert_eq!(grab_at(&[a, b], Pos2::new(212.0, 115.0), true, false, Some(0)), Some((1, Grip::Move)));
    }

    #[test]
    fn edges_follow_the_width_on_a_quarter_turned_page() {
        let b = r(100.0, 100.0, 140.0, 300.0);
        assert_eq!(grab_at(&[b], Pos2::new(120.0, 92.0), true, true, None), Some((0, Grip::Left)));
        assert_eq!(grab_at(&[b], Pos2::new(120.0, 308.0), true, true, None), Some((0, Grip::Right)));
        let [top, bottom] = width_handles(b, true);
        assert!(top.center().y == 100.0 && bottom.center().y == 300.0, "{top:?} {bottom:?}");
    }

    #[test]
    fn a_frame_spans_where_the_text_wraps() {
        let rect = [20.0, 100.0, 120.0, 140.0];
        assert_eq!(frame_of(rect, [20.0, 180.0]), [20.0, 100.0, 180.0, 140.0]);
        // Never narrower than the ink.
        assert_eq!(frame_of(rect, [30.0, 90.0]), rect);
        assert_eq!(frame_of(rect, [f64::NAN, 180.0]), rect);
    }
}
