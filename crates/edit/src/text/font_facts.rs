//! Sintec.PDF (tasks 001, 006, 027, 033): what a page's font is, which characters of new text
//! it can't show, and how a substitute face is fitted to it, so a substitute looks like the font
//! it stands in for in the inline editor's preview and in the rewrite alike.

use std::collections::{BTreeMap, BTreeSet};

use pdfcraft_fonts::pdf::Metrics;

use super::fallback::{source_encodes, source_program};
use super::lines::page_dict;
use super::{Dict, Document, EditError, TextLine};

/// Sintec.PDF: what a page's font resource is, for finding a substitute when the text being
/// edited needs one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontFacts {
    /// `/BaseFont` without the subset tag.
    pub base_font: String,
    pub bold: bool,
    pub italic: bool,
    /// The program is in the file (`FontFile`, `FontFile2` or `FontFile3`).
    pub embedded: bool,
    /// Only some glyphs are in the file (`ABCDEF+Name`).
    pub subset: bool,
    /// The descriptor's Serif flag, when it has flags; FixedPitch.
    pub serif: Option<bool>,
    pub monospace: bool,
    /// A full face of the same family is already loaded into the page (`PCFull…`).
    pub completed: bool,
    /// What the embedded program says it is (its name table and OS/2), which PDFs often hide
    /// behind a made-up `/BaseFont` such as `CIDFont+F1`: family, PostScript name, weight class,
    /// italic, and serif when its PANOSE says.
    pub program_family: Option<String>,
    pub program_postscript: Option<String>,
    pub program_weight: Option<u16>,
    pub program_italic: Option<bool>,
    pub program_serif: Option<bool>,
    /// Advance widths per 1000 em of the probe characters the font has (via ToUnicode and its
    /// widths): a fingerprint for fonts whose names say nothing.
    pub widths: Vec<(char, f32)>,
    /// The weight class (100–900) found for the font, and what said so (see [`StyleSource`]);
    /// [`StyleSource::Unknown`] when nothing was looked at (then `bold` and the program decide).
    pub weight: u16,
    pub weight_from: StyleSource,
    /// What says the font is italic or oblique; `None` when it is upright.
    pub italic_from: Option<StyleSource>,
    /// Letter heights and stem thickness per em: the descriptor's `/CapHeight` and `/XHeight`, else
    /// measured on the embedded program's outlines. Used to size a substitute face to the font.
    pub cap_height: Option<f64>,
    pub x_height: Option<f64>,
    pub stem: Option<f64>,
}

/// Sintec.PDF: where a detected weight or slant comes from, most reliable first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StyleSource {
    /// Not detected.
    #[default]
    Unknown,
    /// The embedded program's OS/2 table (weight class, italic bit).
    Program,
    /// The font descriptor's `/FontWeight`, `/ItalicAngle` or Italic flag.
    Descriptor,
    /// The style suffix of `/BaseFont` or of the program's PostScript name (`-Bold`, `,Italic`).
    Name,
    /// The descriptor's ForceBold flag.
    ForceBold,
    /// The thickness of the embedded glyphs' stems.
    Stems,
    /// The descriptor's `/StemV`.
    StemV,
    /// Nothing said otherwise: regular.
    Default,
}

impl FontFacts {
    /// The family to match: the program's own, else the `/BaseFont`.
    pub fn family(&self) -> &str {
        self.program_family.as_deref().unwrap_or(&self.base_font)
    }

    /// Bold (weight 600 or more): the detected weight, else the program's, else the PDF's flags
    /// and name.
    pub fn is_bold(&self) -> bool {
        if self.weight_from != StyleSource::Unknown {
            return self.weight >= 600;
        }
        self.program_weight.map_or(self.bold, |w| w >= 600)
    }

    pub fn is_italic(&self) -> bool {
        if self.weight_from != StyleSource::Unknown {
            return self.italic_from.is_some();
        }
        self.program_italic.unwrap_or(self.italic)
    }

    /// Serif: the program's PANOSE, else the descriptor's flag (which producers often get wrong).
    pub fn is_serif(&self) -> Option<bool> {
        self.program_serif.or(self.serif)
    }
}

/// The advance widths per 1000 em of the probe characters `m` shows, from its codes' Unicode text.
fn probe_widths(m: &Metrics) -> Vec<(char, f32)> {
    let probe: std::collections::HashSet<char> = pdfcraft_fonts::embedding::probe_chars().collect();
    let mut out: Vec<(char, f32)> = Vec::new();
    for code in 0u32..=0xFFFF {
        let Some(text) = m.text_of(code) else { continue };
        let mut it = text.chars();
        let (Some(c), None) = (it.next(), it.next()) else { continue };
        let w = m.width(code) * 1000.0;
        if probe.contains(&c) && w.is_finite() && w > 0.0 && !out.iter().any(|(o, _)| *o == c) {
            out.push((c, w as f32));
        }
    }
    out
}

/// [`FontFacts`] for the font resource `resource` of page `page`.
pub fn font_facts(doc: &Document, page: usize, resource: &str) -> Result<FontFacts, EditError> {
    let p = page_dict(doc, page)?;
    let resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let font = fonts.get(resource.as_bytes()).and_then(|f| doc.dict(f)).ok_or_else(|| EditError::Invalid(format!("no font resource {resource}")))?;
    let m = Metrics::from_dict(doc, &font);
    // A composite font's descriptor sits on its descendant.
    let descendant =
        font.get(b"DescendantFonts").map(|d| doc.resolve(d)).and_then(|d| d.as_array().and_then(|a| a.first().cloned())).and_then(|d| doc.dict(&d));
    let descriptor = font.get(b"FontDescriptor").or_else(|| descendant.as_ref().and_then(|d| d.get(b"FontDescriptor"))).and_then(|d| doc.dict(d));
    let embedded = descriptor.as_ref().is_some_and(|d| [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|k| d.get(k).is_some()));
    let flags = descriptor.as_ref().and_then(|d| d.get(b"Flags")).map(|f| doc.resolve(f)).and_then(|f| f.as_f64()).filter(|f| f.is_finite());
    // Flags (ISO 32000-2 table 121): bit 1 FixedPitch, bit 2 Serif.
    let flags = flags.map(|f| f.clamp(0.0, f64::from(u32::MAX)) as u32);
    let base_font = m.base_font.rsplit('+').next().unwrap_or_default().to_string();
    let program = pdfcraft_fonts::pdf::embedded_sfnt(doc, &font);
    let names = program.as_deref().and_then(pdfcraft_fonts::embedding::program_names);
    let style = program.as_deref().and_then(|b| pdfcraft_fonts::embedding::style_of(b).ok());
    let key = pdfcraft_fonts::embedding::family_key(names.as_ref().map_or(&base_font, |n| &n.0));
    let completed = fonts.iter().any(|(name, f)| {
        name.starts_with(b"PCFull")
            && doc.dict(f).is_some_and(|d| pdfcraft_fonts::embedding::family_key(&Metrics::from_dict(doc, &d).base_font) == key)
    });
    let number = |key: &[u8]| descriptor.as_ref().and_then(|d| d.get(key)).map(|v| doc.resolve(v)).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
    let shape = program.as_deref().and_then(pdfcraft_fonts::weight::face_metrics);
    // Per em: the descriptor's heights are in thousandths; nonsense values are ignored.
    let per_em = |v: Option<f64>| v.map(|v| v / 1000.0).filter(|v| (0.2..1.5).contains(v));
    let cap_height = per_em(number(b"CapHeight")).or(shape.and_then(|s| s.cap_height));
    let x_height = per_em(number(b"XHeight")).or(shape.and_then(|s| s.x_height));
    let (weight, weight_from) = detect_weight(
        shape.and_then(|s| s.os2_weight),
        number(b"FontWeight"),
        [m.base_font.as_str(), names.as_ref().map_or("", |n| n.1.as_str())],
        flags.is_some_and(|f| f & (1 << 18) != 0),
        shape.and_then(|s| s.stem_weight()),
        number(b"StemV").zip(x_height.or(cap_height.map(|c| c * 0.7))).and_then(|(v, x)| pdfcraft_fonts::weight::weight_from_stems(v / 1000.0, x)),
    );
    let italic_from = if let Some(s) = style.filter(|_| program.is_some() && shape.is_some_and(|s| s.os2_weight.is_some())) {
        s.italic.then_some(StyleSource::Program)
    } else if number(b"ItalicAngle").is_some_and(|a| a.abs() > 0.5) || flags.is_some_and(|f| f & (1 << 6) != 0) {
        Some(StyleSource::Descriptor)
    } else if pdfcraft_fonts::weight::italic_from_name(&m.base_font) || names.as_ref().is_some_and(|n| pdfcraft_fonts::weight::italic_from_name(&n.1))
    {
        Some(StyleSource::Name)
    } else {
        None
    };
    Ok(FontFacts {
        base_font,
        bold: m.bold,
        italic: m.italic,
        embedded,
        subset: m.subset,
        serif: flags.map(|f| f & 2 != 0),
        monospace: flags.is_some_and(|f| f & 1 != 0) || style.is_some_and(|s| s.monospace),
        completed,
        program_family: names.as_ref().map(|n| n.0.clone()),
        program_postscript: names.map(|n| n.1),
        program_weight: style.map(|s| s.weight),
        program_italic: style.map(|s| s.italic),
        program_serif: style.and_then(|s| s.serif),
        widths: probe_widths(&m),
        weight,
        weight_from,
        italic_from,
        cap_height,
        x_height,
        stem: shape.and_then(|s| s.stem),
    })
}

/// Sintec.PDF: a font's weight class from the most reliable thing that says it: the program's OS/2
/// weight, the descriptor's `/FontWeight`, a style suffix in `names` (`/BaseFont`, the program's
/// PostScript name), ForceBold, the embedded stems' thickness, `/StemV`; regular (400) otherwise.
fn detect_weight(
    os2: Option<u16>,
    font_weight: Option<f64>,
    names: [&str; 2],
    force_bold: bool,
    stems: Option<u16>,
    stem_v: Option<u16>,
) -> (u16, StyleSource) {
    let class = |w: f64| (100.0..=1000.0).contains(&w).then(|| w.round() as u16);
    if let Some(w) = os2.filter(|w| (1..=1000).contains(w)) {
        return (w, StyleSource::Program);
    }
    if let Some(w) = font_weight.and_then(class) {
        return (w, StyleSource::Descriptor);
    }
    if let Some(w) = names.iter().find_map(|n| pdfcraft_fonts::weight::weight_from_name(n)) {
        return (w, StyleSource::Name);
    }
    if force_bold {
        return (700, StyleSource::ForceBold);
    }
    if let Some(w) = stems {
        return (w, StyleSource::Stems);
    }
    if let Some(w) = stem_v {
        return (w, StyleSource::StemV);
    }
    (400, StyleSource::Default)
}

/// Sintec.PDF: the first character of `text` (spaces and line breaks aside) that page `page`'s
/// font resource `resource` can't encode, so new text would need another font for it. `None`
/// when it can show all of it.
pub fn first_unencodable(doc: &Document, page: usize, resource: &str, text: &str) -> Result<Option<char>, EditError> {
    let p = page_dict(doc, page)?;
    let resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let font = fonts.get(resource.as_bytes()).and_then(|f| doc.dict(f)).ok_or_else(|| EditError::Invalid(format!("no font resource {resource}")))?;
    let m = Metrics::from_dict(doc, &font);
    Ok(text.chars().filter(|c| !c.is_whitespace()).find(|c| m.encode(&c.to_string()).is_none()))
}

/// Sintec.PDF (task 033): the characters of `text` (each once, in order) that a rewrite of a run
/// in page `page`'s font `resource` draws in a substitute face: those that neither the font, nor
/// another subset of it on the page, nor a full face of it built into the app, nor the document's
/// complete program of it can show. The steps are the rewrite's own (`rewrite_styled_block`),
/// so the inline editor previews exactly these characters in the substitute. At most 4096.
pub fn substitute_chars(doc: &Document, page: usize, resource: &str, text: &str) -> Result<Vec<char>, EditError> {
    let p = page_dict(doc, page)?;
    let resources = p.dict.get(b"Resources").and_then(|o| doc.dict(o)).unwrap_or_default();
    let fonts = resources.get(b"Font").and_then(|o| doc.dict(o)).unwrap_or_default();
    let font = fonts.get(resource.as_bytes()).and_then(|f| doc.dict(f)).ok_or_else(|| EditError::Invalid(format!("no font resource {resource}")))?;
    let own = Metrics::from_dict(doc, &font);
    let base_font = own.base_font.clone();
    let mut metrics = BTreeMap::new();
    for (name, object) in fonts.iter().take(256) {
        if let Some(f) = doc.resolve(object).as_dict() {
            metrics.insert(String::from_utf8_lossy(name).into_owned(), Metrics::from_dict(doc, f));
        }
    }
    metrics.insert(resource.to_owned(), own);
    let mut program: Option<Option<Vec<u8>>> = None;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for ch in text.chars().filter(|c| *c != '\n') {
        if seen.len() >= 4096 {
            break;
        }
        if !seen.insert(ch) {
            continue;
        }
        let s = ch.to_string();
        let shown = metrics.get(resource).is_some_and(|m| source_encodes(m, &s))
            || sibling_encoding(&metrics, &base_font, &s).is_some()
            || full_face_covering(&base_font, &s).is_some()
            || program
                .get_or_insert_with(|| source_program(doc, &fonts, resource))
                .as_deref()
                .is_some_and(|b| pdfcraft_fonts::embedding::covers(b, &s));
        if !shown {
            out.push(ch);
        }
    }
    Ok(out)
}

/// The page font (a resource name of `metrics`) of the same face as `base_font` that encodes
/// `s`: another subset of the same font.
pub(super) fn sibling_encoding<'a>(metrics: &'a BTreeMap<String, Metrics>, base_font: &str, s: &str) -> Option<&'a String> {
    let key = pdfcraft_fonts::embedding::face_key(base_font);
    metrics.iter().find(|(_, m)| pdfcraft_fonts::embedding::face_key(&m.base_font) == key && source_encodes(m, s)).map(|(name, _)| name)
}

/// The full face of the font `base_font` that the app has built in (craft-fonts) and that shows `s`.
pub(super) fn full_face_covering(base_font: &str, s: &str) -> Option<&'static pdfcraft_fonts::CraftFont> {
    pdfcraft_fonts::embedding::matching_faces(base_font).into_iter().find(|f| pdfcraft_fonts::embedding::covers(f.bytes, s))
}

/// Sintec.PDF: how glyphs of a substitute face are fitted to the source font they stand in for:
/// a size factor that gives them its cap height, a horizontal scale that gives them its advance
/// widths, and a stroke (per em) that thickens a regular face standing in for a bold font when no
/// bold face is loaded.
///
/// Sintec.PDF (task 033): one rule, [`substitute_fit`], for the rewrite and for the inline
/// editor's preview of the same substitute, so the letters typed look as they will be drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// Font size factor (letter height).
    pub size: f64,
    /// Horizontal scale on top of the size factor (advance widths).
    pub scale: f64,
    /// Outline stroke per em (synthetic bold); 0 for none.
    pub stroke: f64,
}

impl Fit {
    pub const NONE: Fit = Fit { size: 1.0, scale: 1.0, stroke: 0.0 };

    /// How much wider each substitute glyph advances than its face's own advance at the same
    /// font size: `size × scale`.
    pub fn advance(&self) -> f64 {
        self.size * self.scale
    }
}

/// Sintec.PDF: the [`Fit`] of the substitute face `face` (a TrueType/OpenType program) to the
/// source font `facts` describes: its letter height, advance widths and weight. [`Fit::NONE`]
/// when the face can't be measured.
pub fn substitute_fit(facts: &FontFacts, face: &[u8]) -> Fit {
    let Some(shape) = pdfcraft_fonts::weight::face_metrics(face) else { return Fit::NONE };
    // Same letter height: capitals, else lowercase. Within 2 % is the same (no rescaling).
    let size = match (facts.cap_height.zip(shape.cap_height), facts.x_height.zip(shape.x_height)) {
        (Some((a, b)), _) | (None, Some((a, b))) if b > 0.0 => (a / b).clamp(0.85, 1.2),
        _ => 1.0,
    };
    let size = if (size - 1.0).abs() < 0.02 { 1.0 } else { size };
    // Same advance widths: the median ratio over the letters both have (per 1000 em: the source
    // font's PDF widths, the face's own advances).
    let face_widths = pdfcraft_fonts::embedding::probe_widths(face);
    let mut ratios: Vec<f64> = facts
        .widths
        .iter()
        .filter_map(|(c, src)| {
            let sub = face_widths.iter().find(|(f, _)| f == c).map(|(_, w)| f64::from(*w))?;
            Some(f64::from(*src) / (sub * size))
        })
        .filter(|r| r.is_finite() && *r > 0.0)
        .collect();
    ratios.sort_by(f64::total_cmp);
    let scale = if ratios.len() >= 4 { ratios.get(ratios.len() / 2).copied().unwrap_or(1.0).clamp(0.85, 1.15) } else { 1.0 };
    let scale = if (scale - 1.0).abs() < 0.02 { 1.0 } else { scale };
    // A regular face for a bold font: stroke the difference in stem thickness.
    let stroke = if facts.is_bold() && shape.os2_weight.is_some_and(|w| w < 600) {
        facts.stem.zip(shape.stem).map_or(0.03, |(a, b)| (a - b * size).clamp(0.015, 0.05))
    } else {
        0.0
    };
    Fit { size, scale, stroke }
}

/// The [`substitute_fit`] of the loaded substitute `name` (its embedded program) to page `page`'s
/// font `source` (a resource name). [`Fit::NONE`] when either can't be read.
pub(super) fn loaded_fit(doc: &Document, page: usize, fonts: &Dict, source: &str, name: &str) -> Fit {
    let Ok(facts) = font_facts(doc, page, source) else { return Fit::NONE };
    match fonts.get(name.as_bytes()).and_then(|f| doc.dict(f)).and_then(|f| pdfcraft_fonts::pdf::embedded_sfnt(doc, &f)) {
        Some(face) => substitute_fit(&facts, &face),
        None => Fit::NONE,
    }
}

/// Sintec.PDF: the full face loaded into the page (`PCFull…`, see `register_text_font`) that
/// best stands in for `source` and can show `s`: the same family first, then the same weight and
/// slant. `None` when no loaded face can show it.
pub(super) fn registered_substitute(metrics: &BTreeMap<String, Metrics>, source: &SourceFace, s: &str) -> Option<String> {
    let family = &source.family;
    metrics
        .iter()
        .filter(|(name, m)| name.starts_with("PCFull") && source_encodes(m, s))
        .max_by_key(|(_, m)| {
            let same_family = pdfcraft_fonts::embedding::family_key(&m.base_font) == *family;
            let bold = pdfcraft_fonts::weight::weight_from_name(&m.base_font).is_some_and(|w| w >= 600);
            let italic = pdfcraft_fonts::weight::italic_from_name(&m.base_font);
            // The map is ordered by name, so ties resolve the same way every time.
            (same_family, bold == source.bold, italic == source.italic)
        })
        .map(|(name, _)| name.clone())
}

/// Sintec.PDF: a source font as it really is: its program's family key, weight and slant
/// (`CIDFont+F1` may be Calibri Bold), else what its PDF dictionary says.
pub(super) struct SourceFace {
    family: String,
    bold: bool,
    italic: bool,
}

pub(super) fn source_face(doc: &Document, page: usize, line: &TextLine) -> SourceFace {
    match font_facts(doc, page, &line.font) {
        Ok(f) => SourceFace {
            family: pdfcraft_fonts::embedding::family_key(f.family()),
            bold: f.is_bold() || line.bold,
            italic: f.is_italic() || line.italic,
        },
        Err(_) => SourceFace { family: pdfcraft_fonts::embedding::family_key(&line.base_font), bold: line.bold, italic: line.italic },
    }
}

#[cfg(test)]
#[path = "../../../engine/tests/it/support/font_fixture.rs"]
mod font_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use Fit as SubstituteFit;
    use pdfcraft_cos::{Object, Stream};

    /// Sintec.PDF (task 027): the weight comes from the most reliable thing that says it.
    #[test]
    fn weight_is_detected_from_the_most_reliable_source() {
        use StyleSource as S;
        assert_eq!(detect_weight(Some(700), Some(400.0), ["Arial", ""], false, None, None), (700, S::Program));
        assert_eq!(detect_weight(None, Some(600.0), ["Arial-Bold", ""], false, None, None), (600, S::Descriptor));
        assert_eq!(detect_weight(None, None, ["ABCDEF+Arial,Bold", ""], false, Some(400), None), (700, S::Name));
        assert_eq!(detect_weight(None, None, ["CIDFont+F1", "Calibri-Bold"], false, None, None), (700, S::Name));
        assert_eq!(detect_weight(None, None, ["CIDFont+F1", ""], true, Some(400), None), (700, S::ForceBold));
        assert_eq!(detect_weight(None, None, ["CIDFont+F1", ""], false, Some(700), Some(400)), (700, S::Stems));
        assert_eq!(detect_weight(None, None, ["F1", ""], false, None, Some(700)), (700, S::StemV));
        assert_eq!(detect_weight(None, Some(f64::NAN), ["F1", ""], false, None, None), (400, S::Default));
        assert_eq!(detect_weight(Some(0), Some(5000.0), ["F1", ""], false, None, None), (400, S::Default));
    }

    /// Detected facts decide bold and italic once detection ran; before, the program and the PDF's
    /// flags do, as they did.
    #[test]
    fn bold_and_italic_follow_the_detected_style() {
        let detected = FontFacts { weight: 600, weight_from: StyleSource::Stems, italic_from: Some(StyleSource::Descriptor), ..Default::default() };
        assert!(detected.is_bold() && detected.is_italic());
        let light = FontFacts { weight: 300, weight_from: StyleSource::Name, bold: true, ..Default::default() };
        assert!(!light.is_bold() && !light.is_italic());
        let legacy = FontFacts { bold: true, italic: true, ..Default::default() };
        assert!(legacy.is_bold() && legacy.is_italic());
    }

    /// A one-page document with composite font `F1` (`/BaseFont /CIDFont+F1`) embedding `program`.
    fn cid_document(program: Vec<u8>) -> Document {
        let mut d = Document::new_empty();
        let pages = d.add(Object::Dict(Dict::new()));
        let file = d.add(Object::Stream(Stream::flate(Dict::new(), &program)));
        let mut desc = Dict::new();
        desc.set(b"Type".to_vec(), Object::name("FontDescriptor"));
        desc.set(b"FontName".to_vec(), Object::name("CIDFont+F1"));
        desc.set(b"Flags".to_vec(), Object::Int(4));
        desc.set(b"FontFile2".to_vec(), Object::Ref(file));
        let desc = d.add(Object::Dict(desc));
        let mut cid = Dict::new();
        cid.set(b"Type".to_vec(), Object::name("Font"));
        cid.set(b"Subtype".to_vec(), Object::name("CIDFontType2"));
        cid.set(b"BaseFont".to_vec(), Object::name("CIDFont+F1"));
        cid.set(b"FontDescriptor".to_vec(), Object::Ref(desc));
        cid.set(b"CIDToGIDMap".to_vec(), Object::name("Identity"));
        let cid = d.add(Object::Dict(cid));
        let mut font = Dict::new();
        font.set(b"Type".to_vec(), Object::name("Font"));
        font.set(b"Subtype".to_vec(), Object::name("Type0"));
        font.set(b"BaseFont".to_vec(), Object::name("CIDFont+F1"));
        font.set(b"Encoding".to_vec(), Object::name("Identity-H"));
        font.set(b"DescendantFonts".to_vec(), Object::Array(vec![Object::Ref(cid)]));
        // Glyph 1 is the space and glyph 2 the letter A.
        let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap
1 begincodespacerange <0000> <FFFF> endcodespacerange
2 beginbfchar <0001> <0020> <0002> <0041> endbfchar
endcmap end end";
        let to_unicode = d.add(Object::Stream(Stream::flate(Dict::new(), cmap)));
        font.set(b"ToUnicode".to_vec(), Object::Ref(to_unicode));
        let mut fonts = Dict::new();
        fonts.set(b"F1".to_vec(), Object::Dict(font));
        let mut res = Dict::new();
        res.set(b"Font".to_vec(), Object::Dict(fonts));
        let mut p = Dict::new();
        p.set(b"Type".to_vec(), Object::name("Page"));
        p.set(b"Parent".to_vec(), Object::Ref(pages));
        p.set(b"Resources".to_vec(), Object::Dict(res));
        p.set(b"MediaBox".to_vec(), Object::Array([0, 0, 300, 240].map(Object::Int).to_vec()));
        let page = d.add(Object::Dict(p));
        let _ = d.update_dict(pages, |p| {
            p.set(b"Type".to_vec(), Object::name("Pages"));
            p.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(page)]));
            p.set(b"Count".to_vec(), Object::Int(1));
        });
        let mut root = Dict::new();
        root.set(b"Type".to_vec(), Object::name("Catalog"));
        root.set(b"Pages".to_vec(), Object::Ref(pages));
        let root = d.add(Object::Dict(root));
        d.trailer_mut().set(b"Root".to_vec(), Object::Ref(root));
        d
    }

    #[test]
    fn font_facts_come_from_the_embedded_program_not_the_made_up_name() {
        let doc = cid_document(font_fixture::cff_font_with_weight(700));
        let facts = font_facts(&doc, 0, "F1").expect("facts");
        assert_eq!(facts.base_font, "F1", "the PDF's own name is only the fallback");
        assert!(facts.embedded);
        assert_eq!(facts.family(), "Rect Font");
        assert_eq!(facts.program_weight, Some(700));
        assert!(facts.is_bold(), "the program's weight wins over the descriptor's flags");
        assert!(font_facts(&doc, 0, "Nope").is_err());
    }

    #[test]
    fn first_unencodable_names_the_first_character_the_font_cannot_show() {
        let doc = cid_document(font_fixture::cff_font_with_weight(400));
        // The font shows only A and the space; whitespace is never reported.
        assert_eq!(first_unencodable(&doc, 0, "F1", "A A").expect("ok"), None);
        assert_eq!(first_unencodable(&doc, 0, "F1", "A q Z").expect("ok"), Some('q'));
        assert!(first_unencodable(&doc, 0, "Nope", "A").is_err());
    }

    /// Sintec.PDF (task 033): the characters a rewrite draws in a substitute are those the font,
    /// its other subsets and its own program can't show; each once, line breaks aside.
    #[test]
    fn substitute_chars_are_the_ones_the_font_cannot_show() {
        // A subset: its program has only A and the space.
        let doc = cid_document(font_fixture::cff_font_styled("CIDFont+F1", "Regular", 400, 700, &[' ', 'A']));
        assert_eq!(substitute_chars(&doc, 0, "F1", "A ЖЖ\nqA").expect("ok"), vec!['Ж', 'q']);
        assert_eq!(substitute_chars(&doc, 0, "F1", "A A").expect("ok"), Vec::<char>::new());
        assert!(substitute_chars(&doc, 0, "Nope", "Ж").is_err());
    }

    /// Sintec.PDF (task 033): one fitting rule for the preview and the rewrite: the stand-in gets
    /// the source font's letter height, advance widths and weight, within bounds.
    #[test]
    fn substitute_fit_matches_letter_height_widths_and_weight() {
        let chars = [' ', 'A', 'B', 'C', 'D', 'H', 'Ж'];
        // Rectangles 700 units tall and 600 wide, regular.
        let face = font_fixture::cff_font_styled("Stand-Regular", "Regular", 400, 700, &chars);
        let shape = pdfcraft_fonts::weight::face_metrics(&face).expect("measurable");
        let widths: Vec<(char, f32)> = ['A', 'B', 'C', 'D'].iter().map(|c| (*c, 630.0)).collect();
        let facts = FontFacts { cap_height: Some(0.65), weight: 700, weight_from: StyleSource::Name, widths: widths.clone(), ..Default::default() };
        let fit = substitute_fit(&facts, &face);
        match shape.cap_height {
            Some(cap) => assert!((fit.size - (0.65 / cap).clamp(0.85, 1.2)).abs() < 1e-9, "{fit:?} for {shape:?}"),
            None => assert_eq!(fit.size, 1.0),
        }
        assert!((fit.scale - (630.0 / (600.0 * fit.size)).clamp(0.85, 1.15)).abs() < 1e-6, "{fit:?}");
        assert!((fit.advance() - fit.size * fit.scale).abs() < 1e-12);
        assert!(fit.stroke > 0.0, "a regular face standing in for a bold font is stroked");
        // The same font, regular and as wide: nothing to fit.
        let same = FontFacts {
            cap_height: shape.cap_height,
            widths: chars.iter().filter(|c| **c != ' ').map(|c| (*c, 600.0)).collect(),
            ..Default::default()
        };
        assert_eq!(substitute_fit(&same, &face), SubstituteFit::NONE);
        // Fewer than four letters in common say nothing about widths.
        let few = FontFacts { widths: widths.into_iter().take(3).collect(), ..Default::default() };
        assert_eq!(substitute_fit(&few, &face).scale, 1.0);
        // Not a font program: no fit, no panic.
        assert_eq!(substitute_fit(&facts, b"not a font"), SubstituteFit::NONE);
        assert_eq!(substitute_fit(&facts, &[]), SubstituteFit::NONE);
    }
}
