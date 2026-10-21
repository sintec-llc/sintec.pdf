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
    /// Advance widths per 1000 em of the probe characters the face has (its width fingerprint).
    pub widths: Vec<(char, u16)>,
    /// Shipped with the app (see [`bundled_directories`]), not installed on the machine.
    pub bundled: bool,
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

/// Where the app's own substitute fonts (Carlito, Caladea, Tinos, Cousine, PT Sans) live: `fonts`
/// beside the executable (Windows), `../Resources/fonts` (macOS bundle), `../share/pdfcraft/fonts`
/// (Linux packages) and the source tree's `assets/substitute-fonts` (`cargo xtask fonts`).
/// Folders that don't exist are skipped by [`discover`].
pub fn bundled_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("fonts"));
        dirs.push(exe_dir.join("../Resources/fonts"));
        dirs.push(exe_dir.join("../share/pdfcraft/fonts"));
    }
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/substitute-fonts"));
    dirs
}

/// The installed faces plus the bundled ones (flagged [`FontFile::bundled`]). A bundled face
/// whose file is also reachable through the installed folders is listed once, as installed.
pub fn discover_all() -> Vec<FontFile> {
    let mut files = discover(&installed_directories());
    let known: std::collections::HashSet<PathBuf> = files.iter().filter_map(|f| f.path.canonicalize().ok()).collect();
    let bundled = scan(&bundled_directories(), true).into_iter().filter(|f| f.path.canonicalize().map_or(true, |p| !known.contains(&p)));
    files.extend(bundled);
    sort_files(&mut files);
    files
}

/// How many bytes of font files one scan reads at most: a full Windows Fonts folder (≈ 1 GB
/// with the optional language packs) fits; each file is read, checked and dropped in turn.
const SCAN_BUDGET: u64 = 2 << 30;

/// Bounded, symlink-free scan. Only standalone, embeddable faces are listed.
/// The caller controls roots (automation resolves confinement before calling this).
pub fn discover(roots: &[PathBuf]) -> Vec<FontFile> {
    scan(roots, false)
}

fn scan(roots: &[PathBuf], bundled: bool) -> Vec<FontFile> {
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
                let widths = pdfcraft_fonts::embedding::probe_widths(&bytes);
                files.push(FontFile { path, family, face, style, widths, bundled });
            }
        }
    }
    sort_files(&mut files);
    files
}

fn sort_files(files: &mut [FontFile]) {
    files.sort_by(|a, b| {
        a.family.cmp(&b.family).then(a.style.weight.cmp(&b.style.weight)).then(a.style.italic.cmp(&b.style.italic)).then(a.path.cmp(&b.path))
    });
}

fn font_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
}

pub use pdfcraft_fonts::embedding::family_key;

/// Families that stand in for each other (metric-compatible or the usual substitutes), by key.
const ALIASES: &[&[&str]] = &[
    &["arial", "helvetica", "helveticaneue", "liberationsans", "arimo", "nimbussans", "nimbussansl", "freesans"],
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
const SANS: &[&str] = &["arial", "liberationsans", "dejavusans", "segoeui", "notosans", "ptsans"];
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
    /// The PDF font's advance widths per 1000 em of the probe characters it has (its width
    /// fingerprint), for fonts whose names say nothing.
    pub widths: Vec<(char, f32)>,
}

/// The fewest shared characters a width fingerprint needs to claim anything.
pub const FINGERPRINT_MIN_SHARED: usize = 8;
/// A fingerprint is close when the mean absolute width difference is below this (per 1000 em).
pub const FINGERPRINT_CLOSE: f32 = 15.0;

/// The mean absolute difference (per 1000 em) between the PDF font's widths and the face's over
/// the characters both have; `None` with fewer than [`FINGERPRINT_MIN_SHARED`] of them.
pub fn fingerprint_distance(want: &Wanted, f: &FontFile) -> Option<f32> {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for (c, w) in want.widths.iter().take(1024) {
        if !w.is_finite() {
            continue;
        }
        if let Some((_, fw)) = f.widths.iter().find(|(fc, _)| fc == c) {
            sum += (w - f32::from(*fw)).abs();
            n += 1;
        }
    }
    (n >= FINGERPRINT_MIN_SHARED).then(|| sum / n as f32).filter(|d| d.is_finite())
}

/// The distance when it is close enough to identify the family.
pub fn fingerprint(want: &Wanted, f: &FontFile) -> Option<f32> {
    fingerprint_distance(want, f).filter(|d| *d < FINGERPRINT_CLOSE)
}

impl Wanted {
    /// What to look for to stand in for a PDF font: its program's own family, weight and slant
    /// when it carries them (PDFs often name their fonts `CIDFont+F1`), else what the PDF says;
    /// `bold` and `italic` ask for that style of it instead.
    pub fn from_facts(facts: &pdfcraft_edit::text::FontFacts, bold: Option<bool>, italic: Option<bool>, sample: &str) -> Wanted {
        Wanted {
            base_font: facts.family().to_string(),
            bold: bold.unwrap_or_else(|| facts.is_bold()),
            italic: italic.unwrap_or_else(|| facts.is_italic()),
            serif: facts.is_serif(),
            monospace: facts.monospace,
            sample: sample.to_string(),
            widths: facts.widths.clone(),
        }
    }

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
    /// Not the same name, but the glyph widths are those of this face.
    Fingerprint,
    Exact,
}

/// How well `f` matches `want`'s family.
pub fn family_match(want: &Wanted, f: &FontFile) -> Match {
    let (a, b) = (family_key(&want.base_font), family_key(&f.family));
    if a.is_empty() {
        if fingerprint(want, f).is_some() { Match::Fingerprint } else { Match::Other }
    } else if a == b || family_key(&f.face) == a {
        Match::Exact
    } else if fingerprint(want, f).is_some() {
        Match::Fingerprint
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
                Match::Fingerprint => 8_000,
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
            // At an otherwise equal score the installed face wins over the bundled one.
            let installed_first = i32::from(!f.bundled);
            let slant = if f.style.italic == want.italic { 400 } else { 0 };
            (family + same_class + common + weight + slant + installed_first, f)
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
            widths: Vec::new(),
            bundled: false,
        }
    }

    fn bundled(family: &str, weight: u16, italic: bool, serif: Option<bool>) -> FontFile {
        FontFile { bundled: true, path: PathBuf::from(format!("bundled/{family}-{weight}-{italic}.ttf")), ..face(family, weight, italic, serif) }
    }

    fn known(offset: f32, n: usize) -> Vec<(char, f32)> {
        "ABCDEFGHIJKLMNOP".chars().take(n).enumerate().map(|(i, c)| (c, 500.0 + 10.0 * i as f32 + offset)).collect()
    }

    fn known_face(family: &str) -> FontFile {
        let mut f = face(family, 400, false, Some(false));
        f.widths = known(0.0, 16).into_iter().map(|(c, w)| (c, w as u16)).collect();
        f
    }

    #[test]
    fn an_unnamed_font_is_identified_by_its_widths() {
        let fonts = vec![face("Arial", 400, false, Some(false)), known_face("Mystery Sans"), face("Liberation Sans", 400, false, Some(false))];
        let want = Wanted { base_font: "CIDFont+F1".into(), widths: known(3.0, 16), ..Wanted::default() };
        assert_eq!(rank(&want, &fonts).first().map(|f| f.family.as_str()), Some("Mystery Sans"));
        assert_eq!(family_match(&want, &fonts[1]), Match::Fingerprint);
        assert!((fingerprint_distance(&want, &fonts[1]).unwrap_or(-1.0) - 3.0).abs() < 0.01);
        // Fewer than eight shared characters: no claim.
        let few = Wanted { widths: known(0.0, 7), ..want.clone() };
        assert_eq!(fingerprint_distance(&few, &fonts[1]), None);
        assert_eq!(family_match(&few, &fonts[1]), Match::Other);
        // A mean difference of 15 or more: no claim.
        let far = Wanted { widths: known(15.0, 16), ..want };
        assert!(fingerprint_distance(&far, &fonts[1]).is_some());
        assert_eq!(family_match(&far, &fonts[1]), Match::Other);
    }

    #[test]
    fn wanted_prefers_the_programs_family_and_weight_over_the_pdfs_flags() {
        let facts = pdfcraft_edit::text::FontFacts {
            base_font: "F1".into(),
            bold: false,
            italic: false,
            serif: Some(true),
            program_family: Some("Rect Font".into()),
            program_weight: Some(700),
            program_italic: Some(true),
            program_serif: Some(false),
            ..Default::default()
        };
        let w = Wanted::from_facts(&facts, None, None, "Ж");
        assert_eq!((w.base_font.as_str(), w.bold, w.italic, w.serif, w.sample.as_str()), ("Rect Font", true, true, Some(false), "Ж"));
        // An explicit style request still wins; without a program the PDF's own facts are used.
        let w = Wanted::from_facts(&facts, Some(false), Some(false), "");
        assert!(!w.bold && !w.italic);
        let plain = pdfcraft_edit::text::FontFacts { base_font: "Arial".into(), bold: true, ..Default::default() };
        let w = Wanted::from_facts(&plain, None, None, "");
        assert!(w.bold && w.base_font == "Arial");
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

    #[test]
    fn bundled_faces_stand_in_as_aliases_for_their_originals() {
        for (original, substitute) in [("Calibri", "Carlito"), ("Cambria", "Caladea"), ("Times New Roman", "Tinos"), ("Courier New", "Cousine")] {
            let want = Wanted { base_font: format!("ABCDEF+{}", original.replace(' ', "")), ..Wanted::default() };
            assert_eq!(family_match(&want, &bundled(substitute, 400, false, Some(true))), Match::Alias, "{original}");
        }
        // PT Sans is only a sans class fallback, never an alias.
        let arial = Wanted { base_font: "Arial".into(), ..Wanted::default() };
        assert_eq!(family_match(&arial, &bundled("PT Sans", 400, false, Some(false))), Match::Other);
    }

    #[test]
    fn installed_faces_beat_bundled_ones() {
        let fonts = vec![
            bundled("Carlito", 400, false, Some(false)),
            bundled("Tinos", 400, false, Some(true)),
            face("Calibri", 400, false, Some(false)),
            face("Carlito", 400, false, Some(false)),
        ];
        let want = |base: &str| Wanted { base_font: base.into(), ..Wanted::default() };
        // The exact installed face first; then, between equal stand-ins, the installed one.
        let ranked = rank(&want("Calibri"), &fonts);
        assert_eq!((ranked[0].family.as_str(), ranked[0].bundled), ("Calibri", false));
        assert_eq!((ranked[1].family.as_str(), ranked[1].bundled), ("Carlito", false));
        assert!(ranked[2].bundled);
        // Without the original installed, the bundled stand-in beats the unrelated serif.
        let only = vec![bundled("Tinos", 400, false, Some(true)), face("Georgia", 400, false, Some(true))];
        assert_eq!(rank(&want("Times New Roman"), &only)[0].family, "Tinos");
        // And with an installed twin of the same family, the installed one wins.
        let twins = vec![bundled("Tinos", 400, false, Some(true)), face("Tinos", 400, false, Some(true))];
        assert!(!rank(&want("TimesNewRoman"), &twins)[0].bundled);
    }

    #[test]
    fn pt_sans_is_a_cyrillic_sans_fallback_not_an_alias() {
        let fonts = vec![bundled("PT Sans", 400, false, Some(false)), bundled("Tinos", 400, false, Some(true))];
        let want = Wanted { base_font: "Verdana".into(), serif: Some(false), sample: "Привет".into(), ..Wanted::default() };
        assert_eq!(rank(&want, &fonts)[0].family, "PT Sans");
    }

    #[test]
    fn bundled_directories_are_found_and_missing_ones_skipped() {
        let dirs = bundled_directories();
        assert!(dirs.iter().any(|d| d.ends_with("substitute-fonts")) && dirs.iter().any(|d| d.ends_with("fonts")));
        assert!(scan(&[PathBuf::from("no/such/dir")], true).is_empty());
        let dir = std::env::temp_dir().join(format!("pdfcraft-bundled-fonts-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("Corrupt.ttf"), b"not a font");
        assert!(scan(std::slice::from_ref(&dir), true).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With the fonts fetched (`cargo xtask fonts`), every bundled face is listed and flagged; all but Caladea (Latin only) have Cyrillic.
    #[test]
    fn fetched_substitute_fonts_are_discovered_with_cyrillic() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/substitute-fonts");
        if !dir.is_dir() {
            return;
        }
        let found = scan(&[dir], true);
        if found.is_empty() {
            return;
        }
        assert!(
            found.iter().all(|f| f.bundled && (f.style.cyrillic || f.family == "Caladea")),
            "{:?}",
            found.iter().filter(|f| !f.style.cyrillic).map(|f| &f.face).collect::<Vec<_>>()
        );
        for family in ["Carlito", "Caladea", "Tinos", "Cousine", "PT Sans"] {
            assert!(found.iter().any(|f| f.family == family), "{family}");
        }
    }
}
