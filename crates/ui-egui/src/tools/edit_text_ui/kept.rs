//! Sintec.PDF (task 014): the width a paragraph's box was dragged to. PDFs store no wrap width, so
//! after a handle drag the engine would derive one from the rewritten text's layout
//! ([`pdfcraft_engine::TextBlock::wrap`]) and the box would snap back to the text's ink. The width
//! is kept here instead: the box shows it, and every further rewrite of the paragraph is sent
//! with it, so the box stays where the text really wraps.

use pdfcraft_engine::Edit;

use crate::canvas::DocView;

/// The widest a kept width can be (user space; the largest page side PDF allows).
const MAX_WIDTH: f64 = 14_400.0;

/// The explicit width a paragraph was last rewritten at ([`pdfcraft_engine::BlockStyle::width`]:
/// a handle drag, or an edit that carried this width on).
#[derive(Clone, Debug, PartialEq)]
pub struct KeptWidth {
    page: usize,
    block: usize,
    /// In user space.
    width: f64,
    /// The document generation the rewrite produced. Any other change drops the width.
    generation: u64,
    /// The rewritten text (see [`text_key`]), to recognise the paragraph.
    key: String,
}

/// A paragraph's text without spacing, breaks or hyphens: what a rewrap leaves unchanged.
fn text_key(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace() && !matches!(c, '-' | '\u{ad}')).collect()
}

/// After `edit` was applied (the document is now at `generation`): an explicit width it set is
/// kept for the paragraph; anything else drops a kept width (see [`keep_width`]).
pub(crate) fn kept_after(view: &mut DocView, edit: &Edit, generation: u64) {
    view.kept_width = match edit {
        Edit::EditTextBlock { page, block, text, style } => style.width.filter(|w| w.is_finite() && *w > 0.0).map(|width| KeptWidth {
            page: *page,
            block: *block,
            width: width.min(MAX_WIDTH),
            generation,
            key: text_key(text),
        }),
        _ => None,
    };
}

/// The width kept for paragraph `block` on `page` while the document is at `generation`.
pub(crate) fn kept_width_of(view: &DocView, page: usize, block: usize, generation: u64) -> Option<f64> {
    view.kept_width.as_ref().filter(|k| k.page == page && k.block == block && k.generation == generation).map(|k| k.width)
}

/// A rewrite of a paragraph whose width is kept carries that width, so it wraps where its box
/// shows (unless it asks for a width of its own).
pub(crate) fn with_kept_width(view: &DocView, generation: u64, mut edit: Edit) -> Edit {
    if let Edit::EditTextBlock { page, block, style, .. } = &mut edit
        && style.width.is_none()
        && let Some(width) = kept_width_of(view, *page, *block, generation)
    {
        style.width = Some(width);
    }
    edit
}

/// Apply the kept width to a freshly read page's paragraphs `blocks`. It is dropped when the
/// document has changed since, or the paragraph at its index is no longer the rewritten one. The
/// box shows the larger of the kept width and the text's extent (the engine never wraps inside a
/// word), and that is the width kept for the next rewrite, so narrowing below the longest word
/// shows the clamp.
pub(crate) fn keep_width(view: &mut DocView, page: usize, generation: u64, blocks: &mut [pdfcraft_engine::TextBlock]) {
    let Some(k) = view.kept_width.as_mut().filter(|k| k.page == page) else { return };
    let Some(b) = blocks.get_mut(k.block).filter(|b| k.generation == generation && k.width.is_finite() && text_key(&b.text) == k.key) else {
        view.kept_width = None;
        return;
    };
    let ink = b.rect[2] - b.rect[0];
    k.width = k.width.max(if ink.is_finite() { ink } else { 0.0 }).min(MAX_WIDTH);
    // An explicit width wraps from the paragraph's left edge, right to left or not.
    b.wrap = [b.rect[0], b.rect[0] + k.width];
}

/// The union of the rects of `lines` the row `row` names, if any.
fn row_rect(row: &[usize], lines: &[pdfcraft_engine::TextLine]) -> Option<[f64; 4]> {
    row.iter().filter_map(|i| lines.get(*i)).map(|l| l.rect).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

/// The text a width drag rewraps paragraph `b` with (`lines`: its page's text lines). The engine's
/// [`pdfcraft_engine::TextBlock::reflow_text`] keeps every unmarked row that ends short of the
/// paragraph's right edge as a hard break, so a paragraph written elsewhere and wrapped at a
/// ragged right edge would keep its rows whatever the new width. Here such a break is kept only
/// where the next row's first word would have fitted on the row (a heading over its value, a
/// table cell's own lines); otherwise the row was wrapped by the old width and rejoins. A break
/// that was typed (`TextLine::after_kept_break`) stays, and breaks the engine rejoins stay
/// rejoined. The next word's width is estimated from its row's average
/// advance. Falls back to `reflow_text` when the two texts can't be lined up.
pub(super) fn rewrap_text(b: &pdfcraft_engine::TextBlock, lines: &[pdfcraft_engine::TextLine]) -> String {
    let parts: Vec<&str> = b.editor_text.split('\n').collect();
    if parts.len() != b.rows.len() || parts.len() < 2 {
        return b.reflow_text.clone();
    }
    let size = if b.size.is_finite() && b.size > 0.0 { b.size } else { 1.0 };
    // Whether the break after row `i` was typed, or the first word of row `i + 1` would have
    // fitted after row `i`.
    let next_fitted = |i: usize| -> bool {
        let typed = b.rows.get(i + 1).and_then(|r| r.first()).and_then(|l| lines.get(*l)).is_some_and(|l| l.after_kept_break());
        if typed {
            return true;
        }
        let (Some(row), Some(next), Some(text)) =
            (b.rows.get(i).and_then(|r| row_rect(r, lines)), b.rows.get(i + 1).and_then(|r| row_rect(r, lines)), parts.get(i + 1))
        else {
            return true;
        };
        let text = text.trim();
        let chars = text.chars().count();
        let word = text.split(char::is_whitespace).next().map_or(0, |w| w.chars().count());
        if chars == 0 || word == 0 {
            return true;
        }
        let word_width = (next[2] - next[0]).max(0.0) * word as f64 / chars as f64;
        let fits = row[2] + 0.3 * size + word_width + 0.2 * size;
        !fits.is_finite() || fits < b.rect[2]
    };
    // Walk `reflow_text`, which is the rows joined by a newline or by the separator a wrap took.
    let mut out = String::with_capacity(b.reflow_text.len());
    let mut rest = b.reflow_text.as_str();
    for (i, part) in parts.iter().enumerate() {
        // A soft wrap after a hyphen drops it.
        let taken =
            if rest.starts_with(part) { part.len() } else { part.strip_suffix('-').filter(|p| rest.starts_with(p)).map_or(usize::MAX, str::len) };
        let Some((row, after)) = rest.split_at_checked(taken) else { return b.reflow_text.clone() };
        out.push_str(row);
        rest = after;
        let Some(next) = parts.get(i + 1) else { break };
        if let Some(after) = rest.strip_prefix('\n') {
            rest = after;
            if next_fitted(i) {
                out.push('\n');
            } else if out.ends_with('-') {
                out.pop();
            } else if !out.ends_with(char::is_whitespace) && !next.starts_with(char::is_whitespace) {
                out.push(' ');
            }
        } else {
            // The separator the engine rejoined with: the spaces before the next row's text.
            let spaces = rest.len() - rest.trim_start_matches(' ').len();
            let Some(gap) = (0..=spaces)
                .find(|k| rest.get(*k..).is_some_and(|r| r.starts_with(next) || next.strip_suffix('-').is_some_and(|n| r.starts_with(n))))
            else {
                return b.reflow_text.clone();
            };
            let Some((sep, after)) = rest.split_at_checked(gap) else { return b.reflow_text.clone() };
            out.push_str(sep);
            rest = after;
        }
    }
    if rest.is_empty() { out } else { b.reflow_text.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_ignores_what_a_rewrap_changes() {
        assert_eq!(text_key("a long-\nword wraps"), text_key("a longword\nwraps"));
        assert_ne!(text_key("one two"), text_key("one three"));
    }
}
