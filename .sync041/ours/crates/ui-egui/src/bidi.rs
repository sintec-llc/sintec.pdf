//! Visual order for single-line interface text that may hold right-to-left script (file names,
//! document titles).
//!
//! egui shapes each font-face run with the script's own direction, so the letters of an Arabic
//! word join and run right to left, but it places the runs themselves left to right in logical
//! order: `واحد اثنين.pdf` would show its two words swapped. [`visual`] puts the runs in display
//! order and leaves the letters of each word alone for the shaper.

use std::borrow::Cow;
use std::ops::Range;

use unicode_bidi::{BidiInfo, Level, ParagraphInfo};

/// `text` rearranged for an egui call that lays out one line. Text without right-to-left
/// characters comes back borrowed and unchanged.
///
/// The base direction is left to right, as the interface is: a file name keeps its extension
/// at the end, the way the window title shows it. Use it for painting only; accessibility
/// labels and anything stored keep the logical text.
pub fn visual(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_rtl_script) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    push_visual(&mut out, text, Level::ltr());
    Cow::Owned(out)
}

/// `text` in display order for the paragraph direction `base`, appended to `out`.
fn push_visual(out: &mut String, text: &str, base: Level) {
    let info = BidiInfo::new(text, Some(base));
    for para in &info.paragraphs {
        push_line(out, text, &info, para, para.range.clone());
    }
}

/// The `line` of paragraph `para` (a non-empty byte range of `text`) in display order, as egui
/// needs it, appended to `out`.
fn push_line(out: &mut String, text: &str, info: &BidiInfo<'_>, para: &ParagraphInfo, line: Range<usize>) {
    let (levels, runs) = info.visual_runs(para, line);
    // First the true display order, every character left to right as it appears on screen.
    let mut shown = String::with_capacity(text.len());
    for run in runs {
        let rtl = levels.get(run.start).is_some_and(Level::is_rtl);
        // Runs lie on character boundaries; a missing slice is skipped, never a panic.
        let Some(piece) = text.get(run) else { continue };
        let chars = piece.chars().filter(|c| !is_bidi_control(*c));
        if rtl {
            shown.extend(chars.rev().map(mirrored));
        } else {
            shown.extend(chars);
        }
    }
    push_for_shaper(out, &shown);
}

/// `shown` (in screen order) as egui must get it: egui hands each stretch of right-to-left
/// script to the shaper as one run, which lays it out right to left (and joins its letters), so
/// each such stretch goes back to logical order. What lies between them (spaces, punctuation,
/// Latin) is drawn by the Latin face left to right and stays as it is. The stretches include
/// Persian and Arabic-Indic digits: Arabic script to the shaper, though they read left to right.
fn push_for_shaper(out: &mut String, shown: &str) {
    let mut stretch: Vec<char> = Vec::new();
    for c in shown.chars() {
        if is_rtl_script(c) || is_joiner(c) {
            stretch.push(c);
        } else {
            out.extend(stretch.drain(..).rev());
            out.push(c);
        }
    }
    out.extend(stretch.drain(..).rev());
}

/// One line of wrapped text (see [`wrap`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The line in display order, for one egui call that lays out one line.
    pub text: String,
    /// The paragraph reads right to left: the line belongs against the right edge.
    pub rtl: bool,
}

/// Whether `text` holds any right-to-left script, so [`wrap`] or [`visual_auto`] is needed.
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_script)
}

/// Multi-line text with right-to-left script (a comment in Persian, Arabic, Hebrew), broken
/// into lines that `fits` accepts (it measures a line in logical order) and each put in display
/// order by the Unicode bidirectional algorithm (UAX #9).
///
/// Every paragraph takes its direction from its first strong character, as `dir="auto"` does,
/// so a Persian comment reads right to left with its Latin words and numbers left to right
/// inside it. Lines break between words, in reading order, and a word wider than a line
/// between characters. Use it for painting; copying and accessibility keep the logical text.
pub fn wrap(text: &str, mut fits: impl FnMut(&str) -> bool) -> Vec<Line> {
    let info = BidiInfo::new(text, None);
    let mut lines = Vec::new();
    for para in &info.paragraphs {
        let whole = text.get(para.range.clone()).unwrap_or_default();
        // unicode-bidi ends a paragraph at the `\r` of `\r\n`; the `\n` is no new paragraph.
        if whole == "\n" && text.get(..para.range.start).is_some_and(|before| before.ends_with('\r')) {
            continue;
        }
        let content = para.range.start..para.range.start.saturating_add(whole.trim_end_matches(is_paragraph_separator).len());
        let rtl = para.level.is_rtl();
        for line in break_lines(text, content, &mut fits) {
            let mut out = String::new();
            // unicode-bidi indexes the first level of a line, so an empty one would panic.
            if !line.is_empty() {
                push_line(&mut out, text, &info, para, line);
            }
            lines.push(Line { text: out, rtl });
        }
    }
    lines
}

/// One line of `text` in display order, its direction from its first strong character (an
/// author's name). Text without right-to-left characters comes back borrowed and unchanged.
pub fn visual_auto(text: &str) -> Cow<'_, str> {
    if !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    let lines: Vec<String> = wrap(text, |_| true).into_iter().map(|l| l.text).collect();
    Cow::Owned(lines.join("\n"))
}

/// The byte ranges of the lines `range` of `text` breaks into: as many words as `fits` accepts
/// per line, a word too wide for any line split between characters. Always at least one line.
fn break_lines(text: &str, range: Range<usize>, fits: &mut impl FnMut(&str) -> bool) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut current: Option<Range<usize>> = None;
    for word in words(text, range.clone()) {
        if let Some(line) = current.as_mut() {
            if fits(text.get(line.start..word.end).unwrap_or_default()) {
                line.end = word.end;
                continue;
            }
            lines.push(line.clone());
        }
        // The word starts a line.
        let mut start = word.start;
        loop {
            let end = fitting_prefix(text, start..word.end, fits);
            if end >= word.end {
                current = Some(start..word.end);
                break;
            }
            lines.push(start..end);
            start = end;
        }
    }
    lines.push(current.unwrap_or(range.start..range.start));
    lines
}

/// The end of the longest start of `range` that `fits`, at least one character.
fn fitting_prefix(text: &str, range: Range<usize>, fits: &mut impl FnMut(&str) -> bool) -> usize {
    let piece = text.get(range.clone()).unwrap_or_default();
    let mut end = range.end;
    for (i, (at, c)) in piece.char_indices().enumerate() {
        let next = range.start.saturating_add(at).saturating_add(c.len_utf8());
        if i > 0 && !fits(text.get(range.start..next).unwrap_or_default()) {
            end = range.start.saturating_add(at);
            break;
        }
    }
    end
}

/// The byte ranges of the whitespace-separated words of `range` in `text`.
fn words(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start = None;
    for (at, c) in text.get(range.clone()).unwrap_or_default().char_indices() {
        let at = range.start.saturating_add(at);
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                words.push(s..at);
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push(s..range.end);
    }
    words
}

/// Characters that end a paragraph (bidi class B).
fn is_paragraph_separator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{1C}'..='\u{1E}' | '\u{85}' | '\u{2029}')
}

/// Characters of the right-to-left scripts' blocks: letters, marks and the scripts' own
/// punctuation, all drawn by the script's face.
fn is_rtl_script(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF)
}

/// The zero-width (non-)joiners inside a word (Persian `می‌روم`): egui shapes them with the
/// letter before them.
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Direction marks, embeddings, overrides and isolates: they steer the reordering and are not drawn.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_without_rtl_is_borrowed_unchanged() {
        for s in ["", "report.pdf", "日本語の文字.pdf", "a (b) [c]", "caf\u{00E9} \u{200E}x"] {
            assert!(matches!(visual(s), Cow::Borrowed(v) if v == s), "{s:?}");
        }
    }

    #[test]
    fn arabic_words_swap_and_the_extension_stays_last() {
        // The reported name: "one two.pdf" in Arabic.
        assert_eq!(visual("واحد اثنين.pdf"), "اثنين واحد.pdf");
        assert_eq!(visual("ملف.pdf"), "ملف.pdf");
        assert_eq!(visual("تقرير نهائي جدا.pdf"), "جدا نهائي تقرير.pdf");
    }

    #[test]
    fn latin_and_numbers_keep_their_own_order() {
        assert_eq!(visual("report واحد اثنين final.pdf"), "report اثنين واحد final.pdf");
        // A number inside right-to-left text reads left to right, to the left of what precedes it.
        assert_eq!(visual("ملف 2024.pdf"), "2024 ملف.pdf");
        assert_eq!(visual(r"C:\Users\me\واحد اثنين\a.pdf"), r"C:\Users\me\اثنين واحد\a.pdf");
    }

    #[test]
    fn hebrew_and_brackets() {
        assert_eq!(visual("שלום עולם.pdf"), "עולם שלום.pdf");
        // Brackets between right-to-left words flip with the direction.
        assert_eq!(visual("واحد (اثنين) ثلاثة.pdf"), "ثلاثة (اثنين) واحد.pdf");
    }

    #[test]
    fn direction_controls_are_not_drawn() {
        assert_eq!(visual("\u{202B}واحد اثنين\u{202C}.pdf"), "اثنين واحد.pdf");
        assert_eq!(visual("\u{200F}واحد\u{061C}"), "واحد");
    }

    #[test]
    fn odd_input_never_panics() {
        let long = "ا ب ".repeat(20_000);
        let odd = [
            "\u{064B}",
            "\u{064B}\u{064C} \u{0301}",
            "\u{202E}\u{202E}\u{202E}ا",
            "\u{2067}ا\u{2066}b",
            "ا\nب\r\nج\u{2029}د",
            "\u{FEFF}ا\u{0000}ب",
            "ا\u{10FFFF}\u{E000}ب",
            "((((ا]]]]",
            long.as_str(),
        ];
        for s in odd {
            let v = visual(s);
            assert!(v.chars().count() <= s.chars().count(), "{:?}", s.chars().take(12).collect::<String>());
        }
    }
}
