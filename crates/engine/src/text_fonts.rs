//! Explicit document-font discovery. Interface fallbacks never enter this catalogue.
//!
//! Sintec.PDF: the catalogue lists every installed face whose embedding permission allows
//! editable embedding (see `pdfcraft_fonts::embedding::validate_document_input`), with its
//! style, and [`rank`] orders it by how well it stands in for a PDF's font.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct FontFile {
    pub path: PathBuf,
    pub family: String,
    /// The PostScript name.
    pub face: String,
    pub style: pdfcraft_fonts::embedding::FaceStyle,
}

impl FontFile {
    /// "Times New Roman Bold Italic": the family and how this face differs from regular.
    pub fn display(&self) -> String {
        let mut s = self.family.clone();
        if self.style.weight >= 600 && !s.to_ascii_lowercase().contains("bold") {
            s.push_str(" Bold");
        }
        if self.style.italic && !s.to_ascii_lowercase().contains("italic") {
            s.push_str(" Italic");
        }
        s
    }
}

/// Standard user-installed font folders; discovery is performed only on request.
pub fn installed_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        let mut dirs = vec![PathBuf::from("/Library/Fonts")];
        if let Some(home) = home {
            dirs.push(home.join("Library/Fonts"));
        }
        dirs
    } else if cfg!(windows) {
        let mut dirs = std::env::var_os("WINDIR").map(|p| PathBuf::from(p).join("Fonts")).into_iter().collect::<Vec<_>>();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
        dirs
    } else {
        let mut dirs = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(home) = home {
            dirs.extend([home.join(".local/share/fonts"), home.join(".fonts")]);
        }
        dirs
    }
}

/// How many bytes of font files one scan reads at most: a full Windows Fonts folder (≈ 1 GB
/// with the optional language packs) fits; each file is read, checked and dropped in turn.
const SCAN_BUDGET: u64 = 2 << 30;

/// Bounded, symlink-free scan. Only standalone, embeddable faces are listed.
/// The caller controls roots (automation resolves confinement before calling this).
pub fn discover(roots: &[PathBuf]) -> Vec<FontFile> {
    use std::io::Read;
    let mut pending: Vec<_> = roots.iter().take(16).map(|r| (r.clone(), 0usize)).collect();
    let mut files = Vec::new();
    let mut entries = 0usize;
    let mut budget = SCAN_BUDGET;
    let mut seen = std::collections::HashSet::new();
    while let Some((path, depth)) = pending.pop() {
        if entries >= 8192 || budget == 0 {
            break;
        }
        entries += 1;
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() || !seen.insert(path.clone()) {
            continue;
        }
        if meta.is_dir() && depth < 4 {
            if let Ok(children) = std::fs::read_dir(&path) {
                let remaining = 8192usize.saturating_sub(entries.saturating_add(pending.len()));
                pending.extend(children.take(remaining).filter_map(Result::ok).map(|p| (p.path(), depth + 1)));
            }
        } else if meta.is_file() && meta.len() <= (16 << 20) && font_extension(&path) {
            if meta.len() > budget {
                continue;
            }
            budget = budget.saturating_sub(meta.len());
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path).and_then(|f| f.take((16 << 20) + 1).read_to_end(&mut bytes));
            if read.is_err() || pdfcraft_fonts::embedding::validate_document_input(&bytes).is_err() {
                continue;
            }
            if let (Ok((family, face)), Ok(style)) = (pdfcraft_fonts::embedding::names(&bytes), pdfcraft_fonts::embedding::style_of(&bytes)) {
                files.push(FontFile { path, family, face, style });
            }
        }
    }
    files.sort_by(|a, b| {
        a.family.cmp(&b.family).then(a.style.weight.cmp(&b.style.weight)).then(a.style.italic.cmp(&b.style.italic)).then(a.path.cmp(&b.path))
    });
    files
}

fn font_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
}

pub use pdfcraft_fonts::embedding::family_key;

/// Families that stand in for each other (metric-compatible or the usual substitutes), by key.
const ALIASES: &[&[&str]] = &[
    &["arial", "helvetica", "helveticaneue", "liberationsans", "arimo", "nimbussans", "nimbussansl", "freesans", "ptsans"],
    &["timesnewroman", "times", "timesroman", "liberationserif", "tinos", "nimbusroman", "nimbusromanno9l", "freeserif", "ptserif"],
    &["couriernew", "courier", "liberationmono", "cousine", "nimbusmono", "nimbusmonol", "freemono", "ptmono"],
    &["calibri", "carlito"],
    &["cambria", "caladea"],
    &["georgia", "gelasio"],
    &["verdana", "dejavusans", "bitstreamverasans"],
    &["tahoma", "dejavusans"],
    &["segoeui", "selawik", "opensans", "notosans"],
    &["arialnarrow", "liberationsansnarrow"],
    &["bookantiqua", "palatinolinotype", "palatino", "tex gyre pagella"],
    &["centurygothic", "futura", "avantgarde"],
    &["consolas", "dejavusansmono", "liberationmono"],
];

/// The faces each class falls back to first, by key, best first.
const SANS: &[&str] = &["arial", "liberationsans", "dejavusans", "segoeui", "notosans"];
const SERIF: &[&str] = &["timesnewroman", "liberationserif", "dejavuserif", "georgia", "notoserif"];
const MONO: &[&str] = &["couriernew", "liberationmono", "dejavusansmono", "consolas"];

/// What a PDF's font looks like, to find an installed face that can stand in for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wanted {
    /// Its `/BaseFont`.
    pub base_font: String,
    pub bold: bool,
    pub italic: bool,
    /// From the font descriptor's flags, when known.
    pub serif: Option<bool>,
    pub monospace: bool,
    /// The text the face must be able to show (the paragraph being edited).
    pub sample: String,
}

impl Wanted {
    /// Serif, sans serif or monospace, from the flags or else from the name.
    fn class(&self) -> Class {
        let key = family_key(&self.base_font);
        if self.monospace || MONO.contains(&key.as_str()) || group(&key).is_some_and(|g| g.iter().any(|k| MONO.contains(k))) || key.contains("mono") {
            return Class::Mono;
        }
        if let Some(serif) = self.serif {
            return if serif { Class::Serif } else { Class::Sans };
        }
        if group(&key).is_some_and(|g| g.iter().any(|k| SERIF.contains(k)))
            || ["times", "roman", "serif", "georgia", "garamond", "bookman", "minion", "cambria", "palatino", "baskerville"]
                .iter()
                .any(|w| key.contains(w) && !key.contains("sans"))
        {
            Class::Serif
        } else {
            Class::Sans
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Sans,
    Serif,
    Mono,
}

fn group(key: &str) -> Option<&'static [&'static str]> {
    ALIASES.iter().copied().find(|g| g.contains(&key))
}

fn class_of(f: &FontFile) -> Class {
    let key = family_key(&f.family);
    if f.style.monospace || MONO.contains(&key.as_str()) {
        Class::Mono
    } else if f.style.serif == Some(true) || SERIF.contains(&key.as_str()) {
        Class::Serif
    } else {
        Class::Sans
    }
}

/// How the family itself matches: the same family, a stand-in for it, or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    Other,
    Alias,
    Exact,
}

/// How well `f` matches `want`'s family.
pub fn family_match(want: &Wanted, f: &FontFile) -> Match {
    let (a, b) = (family_key(&want.base_font), family_key(&f.family));
    if a.is_empty() {
        Match::Other
    } else if a == b || family_key(&f.face) == a {
        Match::Exact
    } else if group(&a).is_some_and(|g| g.contains(&b.as_str())) {
        Match::Alias
    } else {
        Match::Other
    }
}

/// The installed faces that can show `want.sample`, best stand-in first: the same family, then
/// its usual substitutes, then faces of the same class (serif, sans serif, monospace), the
/// common ones first; within each, the closest weight and the same slant.
pub fn rank<'a>(want: &Wanted, fonts: &'a [FontFile]) -> Vec<&'a FontFile> {
    let needs_cyrillic = want.sample.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    let class = want.class();
    let want_weight: i32 = if want.bold { 700 } else { 400 };
    let mut scored: Vec<(i32, &FontFile)> = fonts
        .iter()
        .filter(|f| !needs_cyrillic || f.style.cyrillic)
        .map(|f| {
            let key = family_key(&f.family);
            let family = match family_match(want, f) {
                Match::Exact => 10_000,
                Match::Alias => 6_000,
                Match::Other => 0,
            };
            let same_class = if class_of(f) == class { 2_000 } else { 0 };
            let list = match class {
                Class::Sans => SANS,
                Class::Serif => SERIF,
                Class::Mono => MONO,
            };
            let common = list.iter().position(|k| *k == key).map_or(0, |i| 500 - 50 * i as i32);
            let weight = -(i32::from(f.style.weight) - want_weight).abs();
            let slant = if f.style.italic == want.italic { 400 } else { 0 };
            (family + same_class + common + weight + slant, f)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.family.cmp(&b.1.family)));
    scored.into_iter().map(|(_, f)| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_fonts::embedding::FaceStyle;

    fn face(family: &str, weight: u16, italic: bool, serif: Option<bool>) -> FontFile {
        FontFile {
            path: PathBuf::from(format!("{family}-{weight}-{italic}.ttf")),
            family: family.into(),
            face: family.replace(' ', ""),
            style: FaceStyle { weight, italic, monospace: family.contains("Courier"), serif, cyrillic: !family.contains("Latin") },
        }
    }

    #[test]
    fn family_keys_drop_subset_tags_and_styles() {
        assert_eq!(family_key("ABCDEF+TimesNewRomanPS-BoldItalicMT"), "timesnewroman");
        assert_eq!(family_key("TimesNewRomanPSMT"), "timesnewroman");
        assert_eq!(family_key("Arial,Bold"), "arial");
        assert_eq!(family_key("ArialMT"), "arial");
        assert_eq!(family_key("Helvetica-Oblique"), "helvetica");
        assert_eq!(family_key("Times-Roman"), "times");
        assert_eq!(family_key("Calibri-Light"), "calibri");
        assert_eq!(family_key("Verdana"), "verdana");
    }

    #[test]
    fn ranking_prefers_the_family_then_its_stand_ins_then_the_class() {
        let fonts = vec![
            face("Arial", 400, false, Some(false)),
            face("Arial", 700, false, Some(false)),
            face("Times New Roman", 400, false, Some(true)),
            face("Times New Roman", 700, true, Some(true)),
            face("Georgia", 400, false, Some(true)),
            face("Courier New", 400, false, None),
            face("Latin Only Serif", 400, false, Some(true)),
        ];
        let first = |w: Wanted| rank(&w, &fonts).first().map(|f| f.display());
        let want =
            |base: &str, bold, italic, sample: &str| Wanted { base_font: base.into(), bold, italic, sample: sample.into(), ..Wanted::default() };
        // The same family, the closest weight and slant.
        assert_eq!(first(want("ABCDEF+TimesNewRomanPS-BoldItalicMT", true, true, "Привет")).as_deref(), Some("Times New Roman Bold Italic"));
        // A stand-in: Helvetica → Arial.
        assert_eq!(first(want("Helvetica-Bold", true, false, "Счёт")).as_deref(), Some("Arial Bold"));
        // An unknown serif face → the class's common face; a face without Cyrillic never.
        let book = Wanted { serif: Some(true), ..want("BookmanOldStyle", false, false, "Договор") };
        assert_eq!(first(book).as_deref(), Some("Times New Roman"));
        assert!(!rank(&want("LatinOnlySerif", false, false, "ж"), &fonts).iter().any(|f| f.family == "Latin Only Serif"));
        // Monospace.
        assert_eq!(first(want("Courier", false, false, "x")).as_deref(), Some("Courier New"));
        assert!(rank(&want("Anything", false, false, ""), &[]).is_empty());
    }
}
