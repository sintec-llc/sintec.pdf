//! Sintec.PDF: what a font's weight and slant are, and how tall and heavy its letters are, from
//! the names PDFs give fonts and from the glyph outlines themselves.
//!
//! PDFs often hide a font behind a made-up name (`CIDFont+F1`) and leave out the descriptor's
//! `/FontWeight`, so the edited text's weight is found from whatever there is: the program's
//! OS/2 weight class, the descriptor, the name's style suffix (`-Bold`, `,Bold`, `Black`), and, for
//! an anonymised subset with none of these, the thickness of its vertical stems.

use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

/// The weight a font name's style suffix says (`Arial-BoldMT` 700, `Calibri-Light` 300,
/// `Montserrat-SemiBoldItalic` 600), `None` when it names no weight. A subset tag (`ABCDEF+`) is
/// ignored, and so is a weight word inside the family name ("Boldini" says nothing).
pub fn weight_from_name(name: &str) -> Option<u16> {
    let name = name.rsplit('+').next().unwrap_or(name);
    // The style part: after the first `-` or `,`, else the CamelCase words after the first.
    let style = match name.find(['-', ',']) {
        Some(at) => name.get(at..).unwrap_or_default().to_ascii_lowercase(),
        None => {
            let lower = name.to_ascii_lowercase();
            // "ArialBold", "TimesNewRomanPSBoldItalic": a weight word that ends the name or is
            // followed by Italic/Oblique/MT/PS.
            let trimmed =
                ["mt", "ps", "italic", "oblique", "it"].iter().fold(lower.clone(), |s, t| s.strip_suffix(t).map(str::to_string).unwrap_or(s));
            return WEIGHTS.iter().find(|(w, _)| trimmed.len() > w.len() && trimmed.ends_with(w)).map(|(_, v)| *v);
        }
    };
    WEIGHTS.iter().find(|(w, _)| style.contains(w)).map(|(_, v)| *v)
}

/// Weight words, the longer (more specific) first: "semibold" before "bold".
const WEIGHTS: &[(&str, u16)] = &[
    ("extralight", 200),
    ("ultralight", 200),
    ("hairline", 100),
    ("thin", 100),
    ("semilight", 350),
    ("light", 300),
    ("book", 400),
    ("regular", 400),
    ("roman", 400),
    ("medium", 500),
    ("semibold", 600),
    ("demibold", 600),
    ("demi", 600),
    ("extrabold", 800),
    ("ultrabold", 800),
    ("extrablack", 950),
    ("ultrablack", 950),
    ("bold", 700),
    ("heavy", 800),
    ("black", 900),
    ("bd", 700),
];

/// Whether a font name's style suffix says italic or oblique (`Arial-ItalicMT`, `Times,Italic`,
/// `Helvetica-Oblique`, `ArialBdIt`).
pub fn italic_from_name(name: &str) -> bool {
    let name = name.rsplit('+').next().unwrap_or(name);
    let lower = name.to_ascii_lowercase();
    ["italic", "oblique", "slanted", "inclined", "kursiv"].iter().any(|w| lower.contains(w))
        || lower.ends_with("it")
            && (lower.ends_with("bdit")
                || lower.ends_with("-it")
                || lower.ends_with(",it")
                || lower.ends_with("boldit")
                || lower.ends_with("lightit"))
}

/// A font program's letter proportions, per em.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FaceMetrics {
    /// The height of flat capitals (H) and of flat lowercase letters (x) above the baseline.
    pub cap_height: Option<f64>,
    pub x_height: Option<f64>,
    /// The median thickness of the vertical stems of the letters it has (l, n, o, п, и…), measured
    /// on the outlines across the middle of the letter.
    pub stem: Option<f64>,
    /// The OS/2 weight class, when the program has an OS/2 table.
    pub os2_weight: Option<u16>,
}

impl FaceMetrics {
    /// The weight its stems look like (see [`weight_from_stems`]); `None` without stems or an
    /// x-height to compare them with.
    pub fn stem_weight(&self) -> Option<u16> {
        let x = self.x_height.or(self.cap_height.map(|c| c * 0.7))?;
        weight_from_stems(self.stem?, x)
    }
}

/// Letters whose middle crosses only vertical strokes, so the runs a horizontal line through it
/// meets are stem thicknesses (no bars, no diagonals at that height).
const STEM_LETTERS: &str = "lindhumobpqпишцомгртHINUODTПИШЦОГТ";

/// The weight class a stem `stem` em thick suggests for letters `x_height` em tall. The ratio of
/// the two, measured on Windows' faces, is 0.16–0.19 for regular text faces (Arial, Times New
/// Roman, Calibri, Georgia, Segoe UI), 0.23 for Segoe UI Semibold and 0.26–0.33 for the bold ones,
/// 0.13 for light ones; `None` for numbers that aren't a text face's.
pub fn weight_from_stems(stem: f64, x_height: f64) -> Option<u16> {
    if !stem.is_finite() || !x_height.is_finite() || stem <= 0.0 || x_height <= 0.05 {
        return None;
    }
    let ratio = stem / x_height;
    Some(match ratio {
        r if r > 0.6 => return None,
        r if r >= 0.245 => 700,
        r if r >= 0.21 => 600,
        r if r >= 0.195 => 500,
        r if r >= 0.135 => 400,
        r if r >= 0.09 => 300,
        _ => 200,
    })
}

/// [`FaceMetrics`] of a TrueType/OpenType program (a whole face or a PDF's subset of one: only the
/// letters its character map has are measured). `None` when it isn't one.
pub fn face_metrics(bytes: &[u8]) -> Option<FaceMetrics> {
    let font = FontRef::new(bytes).ok()?;
    let loc = LocationRef::default();
    let upem = f64::from(font.metrics(Size::unscaled(), loc).units_per_em);
    if upem <= 0.0 {
        return None;
    }
    let os2 = font.os2().ok();
    let declared = |v: Option<i16>| v.map(f64::from).filter(|v| *v > 0.0).map(|v| v / upem).filter(|v| *v < 2.0);
    let charmap = font.charmap();
    let advances = font.glyph_metrics(Size::unscaled(), loc);
    let outline = |c: char| {
        let gid = charmap.map(c)?;
        let width = f64::from(advances.advance_width(gid).unwrap_or(0.0)) / upem;
        crate::script::bounded_outline(&font, gid, if width.is_finite() && width > 0.0 { width } else { 0.5 }).ok().filter(|g| !g.contours.is_empty())
    };
    let top = |letters: &str| letters.chars().find_map(|c| outline(c).map(|g| g.bbox[3])).filter(|v| *v > 0.05 && *v < 2.0);
    let cap_height = declared(os2.as_ref().and_then(|t| t.s_cap_height())).or_else(|| top("HIEНПТ"));
    let x_height = declared(os2.as_ref().and_then(|t| t.sx_height())).or_else(|| top("xzvnuпитнв"));
    let mut stems = Vec::new();
    for c in STEM_LETTERS.chars() {
        let height = if c.is_lowercase() { x_height } else { cap_height };
        let (Some(height), Some(glyph)) = (height, outline(c)) else { continue };
        if let Some(w) = thinnest_run(&glyph.contours, height * 0.5) {
            stems.push(w);
        }
        if stems.len() >= 12 {
            break;
        }
    }
    stems.sort_by(f64::total_cmp);
    let stem = stems.get(stems.len() / 2).copied();
    Some(FaceMetrics { cap_height, x_height, stem, os2_weight: os2.map(|t| t.us_weight_class()) })
}

/// The thinnest filled run a horizontal line at height `y` crosses in closed `contours` (even-odd
/// fill), ignoring slivers; `None` when it crosses none.
fn thinnest_run(contours: &[Vec<[f64; 2]>], y: f64) -> Option<f64> {
    let mut xs = Vec::new();
    for contour in contours {
        let n = contour.len();
        for i in 0..n {
            let (Some(p), Some(q)) = (contour.get(i), contour.get((i + 1) % n)) else { continue };
            if (p[1] <= y && y < q[1]) || (q[1] <= y && y < p[1]) {
                let dy = q[1] - p[1];
                if dy.abs() > f64::EPSILON {
                    xs.push(p[0] + (y - p[1]) * (q[0] - p[0]) / dy);
                }
            }
        }
        if xs.len() > 256 {
            return None;
        }
    }
    xs.retain(|x| x.is_finite());
    xs.sort_by(f64::total_cmp);
    xs.as_chunks::<2>().0.iter().map(|[a, b]| b - a).filter(|w| *w > 0.004).min_by(f64::total_cmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_comes_from_the_names_style_suffix() {
        for (name, weight) in [
            ("Arial-BoldMT", Some(700)),
            ("ABCDEF+Arial,Bold", Some(700)),
            ("Arial,BoldItalic", Some(700)),
            ("TimesNewRomanPS-BoldItalicMT", Some(700)),
            ("Calibri-Light", Some(300)),
            ("Montserrat-SemiBoldItalic", Some(600)),
            ("Helvetica-Black", Some(900)),
            ("OpenSans-ExtraBold", Some(800)),
            ("Roboto-Medium", Some(500)),
            ("ArialBold", Some(700)),
            ("ArialBdIt", Some(700)),
            ("Arial-BdIt", Some(700)),
            ("Times-Roman", Some(400)),
            ("Calibri", None),
            ("CIDFont+F1", None),
            ("Boldini", None),
            ("Blackadder-Regular", Some(400)),
        ] {
            assert_eq!(weight_from_name(name), weight, "{name}");
        }
    }

    #[test]
    fn italic_comes_from_the_names_style_suffix() {
        for name in ["Arial-ItalicMT", "Times,Italic", "Helvetica-Oblique", "Arial-BdIt", "ABCDEF+Calibri-BoldItalic"] {
            assert!(italic_from_name(name), "{name}");
        }
        for name in ["Arial-BoldMT", "Calibri", "Digit", "CIDFont+F2", "Transit"] {
            assert!(!italic_from_name(name), "{name}");
        }
    }

    #[test]
    fn stems_say_regular_or_bold() {
        // Calibri: x-height 0.466 em, stems about 0.07 em (regular) and 0.115 em (bold).
        assert_eq!(weight_from_stems(0.07, 0.466), Some(400));
        assert_eq!(weight_from_stems(0.115, 0.466), Some(700));
        // Arial Bold: x-height 0.519 em, stems about 0.15 em.
        assert_eq!(weight_from_stems(0.15, 0.519), Some(700));
        assert_eq!(weight_from_stems(0.105, 0.48), Some(600));
        assert_eq!(weight_from_stems(0.06, 0.46), Some(300));
        assert_eq!(weight_from_stems(f64::NAN, 0.5), None);
        assert_eq!(weight_from_stems(0.1, 0.0), None);
        assert_eq!(weight_from_stems(0.5, 0.5), None, "not a text face");
    }

    #[test]
    fn a_horizontal_line_meets_the_thinnest_stem() {
        // Two stems 0.1 and 0.2 em wide, like an n.
        let left = vec![[0.0, 0.0], [0.1, 0.0], [0.1, 0.5], [0.0, 0.5]];
        let right = vec![[0.3, 0.0], [0.5, 0.0], [0.5, 0.5], [0.3, 0.5]];
        let w = thinnest_run(&[left.clone(), right], 0.25).unwrap_or_default();
        assert!((w - 0.1).abs() < 1e-9, "{w}");
        assert_eq!(thinnest_run(&[left], 0.8), None, "above the letter");
        assert_eq!(thinnest_run(&[], 0.2), None);
    }

    #[test]
    fn metrics_of_garbage_are_none() {
        assert_eq!(face_metrics(b"not a font"), None);
        assert_eq!(face_metrics(&[]), None);
    }
}
