//! Text files of any script the bundled monospaced font covers (Latin, Cyrillic, Greek…): the
//! font is embedded as a CIDFontType2 (Identity-H, glyph ids as codes) with a ToUnicode map, so
//! the text shows, searches and copies as written. Helvetica's WinAnsi encoding ([`super::from_text`])
//! can't show Cyrillic at all.
//!
//! Files also arrive in more than UTF-8: [`decode_text`] reads UTF-8 and UTF-16 (with a byte-order
//! mark) and falls back to Windows-1251, the legacy Russian Windows encoding.

use std::collections::BTreeMap;

use pdfcraft_cos::{Dict, Document, Object, Stream};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};

use super::{CreateError, add_page, set_title};

/// JetBrains Mono Regular (SIL OFL 1.1, assets/fonts/OFL-JetBrainsMono.txt): monospaced, like the
/// editors text files are written in, with Latin, Cyrillic and Greek. Embedding is allowed.
static MONO: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
const MONO_NAME: &str = "JetBrainsMono-Regular";

/// ISO A4 in points.
pub const A4: (f64, f64) = (595.28, 841.89);

/// Text set in the embedded monospaced font on pages of `page` size with 2 cm margins.
pub fn from_text_unicode(title: &str, text: &str, page: (f64, f64), font_size: f64) -> Result<Document, CreateError> {
    let font = FontRef::new(MONO).map_err(|e| CreateError::Invalid(format!("the text font is unreadable: {e}")))?;
    let charmap = font.charmap();
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let upem = f64::from(metrics.units_per_em.max(1));
    let glyph_metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let gid_of = |c: char| charmap.map(c).map(|g| g.to_u32());
    // The fallback for characters the font lacks.
    let missing = gid_of('?').unwrap_or(0);
    let advance_units = gid_of('M').and_then(|g| glyph_metrics.advance_width(g.into())).map_or(600.0, f64::from);
    let advance = advance_units * 1000.0 / upem; // per 1000 text units

    let size = if font_size.is_finite() && font_size > 0.0 { font_size.clamp(4.0, 72.0) } else { 10.0 };
    let (w, h) = page;
    let margin = 56.7; // 2 cm
    let line_h = size * 1.3;
    let cols = (((w - 2.0 * margin) / (advance / 1000.0 * size)).floor() as usize).max(8);
    let per_page = (((h - 2.0 * margin) / line_h).floor() as usize).max(1);

    // Lines, wrapped at `cols` characters (at the last space when there is one); form feeds start
    // a new page.
    let mut pages: Vec<Vec<Vec<char>>> = vec![Vec::new()];
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
    for (i, part) in normalized.split('\u{c}').enumerate() {
        if i > 0 && pages.last().is_some_and(|p| !p.is_empty()) {
            pages.push(Vec::new());
        }
        for line in part.split('\n') {
            let chars: Vec<char> = line.chars().filter(|c| !c.is_control()).collect();
            for piece in wrap_chars(&chars, cols) {
                if pages.last().is_some_and(|p| p.len() >= per_page) {
                    pages.push(Vec::new());
                }
                if let Some(p) = pages.last_mut() {
                    p.push(piece);
                }
            }
        }
    }

    let mut used: BTreeMap<u32, char> = BTreeMap::new();
    let mut doc = Document::new_empty();
    let mut contents = Vec::with_capacity(pages.len());
    for lines in &pages {
        let mut c = format!("BT /F1 {size} Tf {line_h:.3} TL {margin} {:.3} Td\n", h - margin - size).into_bytes();
        for line in lines {
            c.push(b'<');
            for &ch in line {
                let gid = gid_of(ch).filter(|&g| g != 0).unwrap_or(missing);
                used.entry(gid).or_insert(if gid == missing { '?' } else { ch });
                c.extend_from_slice(format!("{gid:04X}").as_bytes());
            }
            c.extend_from_slice(b"> Tj T*\n");
        }
        c.extend_from_slice(b"ET\n");
        contents.push(c);
    }

    // The font: Type0 / CIDFontType2 / FontFile2 (the whole font: it is small, and subsetting
    // would need a TrueType writer).
    let mut program_dict = Dict::new();
    program_dict.set(b"Length1".to_vec(), Object::Int(MONO.len() as i64));
    let program = doc.add(Object::Stream(Stream::flate(program_dict, MONO)));
    let scale = |v: f32| (f64::from(v) * 1000.0 / upem).round() as i64;
    let bbox = metrics.bounds.map_or([0, -300, 600, 1000], |b| [scale(b.x_min), scale(b.y_min), scale(b.x_max), scale(b.y_max)]);
    let mut desc = Dict::new();
    desc.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    desc.set(b"FontName".to_vec(), Object::name(MONO_NAME));
    // FixedPitch (1) + Nonsymbolic (32).
    desc.set(b"Flags".to_vec(), Object::Int(33));
    desc.set(b"FontBBox".to_vec(), Object::Array(bbox.into_iter().map(Object::Int).collect()));
    desc.set(b"ItalicAngle".to_vec(), Object::Int(0));
    desc.set(b"Ascent".to_vec(), Object::Int(scale(metrics.ascent)));
    desc.set(b"Descent".to_vec(), Object::Int(scale(metrics.descent)));
    desc.set(b"CapHeight".to_vec(), Object::Int(metrics.cap_height.map_or(700, scale)));
    desc.set(b"StemV".to_vec(), Object::Int(80));
    desc.set(b"FontFile2".to_vec(), Object::Ref(program));
    let desc = doc.add(Object::Dict(desc));
    let mut info = Dict::new();
    info.set(b"Registry".to_vec(), Object::String(pdfcraft_cos::PdfString::literal(&b"Adobe"[..])));
    info.set(b"Ordering".to_vec(), Object::String(pdfcraft_cos::PdfString::literal(&b"Identity"[..])));
    info.set(b"Supplement".to_vec(), Object::Int(0));
    let mut cid = Dict::new();
    cid.set(b"Type".to_vec(), Object::name("Font"));
    cid.set(b"Subtype".to_vec(), Object::name("CIDFontType2"));
    cid.set(b"BaseFont".to_vec(), Object::name(MONO_NAME));
    cid.set(b"CIDSystemInfo".to_vec(), Object::Dict(info));
    cid.set(b"FontDescriptor".to_vec(), Object::Ref(desc));
    cid.set(b"CIDToGIDMap".to_vec(), Object::name("Identity"));
    cid.set(b"DW".to_vec(), Object::Int(advance.round() as i64));
    let to_unicode = doc.add(Object::Stream(Stream::flate(Dict::new(), &to_unicode_cmap(&used))));
    let mut font_dict = Dict::new();
    font_dict.set(b"Type".to_vec(), Object::name("Font"));
    font_dict.set(b"Subtype".to_vec(), Object::name("Type0"));
    font_dict.set(b"BaseFont".to_vec(), Object::name(MONO_NAME));
    font_dict.set(b"Encoding".to_vec(), Object::name("Identity-H"));
    font_dict.set(b"DescendantFonts".to_vec(), Object::Array(vec![Object::Dict(cid)]));
    font_dict.set(b"ToUnicode".to_vec(), Object::Ref(to_unicode));
    let fr = doc.add(Object::Dict(font_dict));

    for c in contents {
        let mut fonts = Dict::new();
        fonts.set(b"F1".to_vec(), Object::Ref(fr));
        let mut res = Dict::new();
        res.set(b"Font".to_vec(), Object::Dict(fonts));
        add_page(&mut doc, w, h, res, Some(c))?;
    }
    set_title(&mut doc, title);
    Ok(doc)
}

/// Break `chars` into lines of at most `cols`, at the last space when one is in reach.
fn wrap_chars(chars: &[char], cols: usize) -> Vec<Vec<char>> {
    if chars.is_empty() {
        return vec![Vec::new()];
    }
    let cols = cols.max(1);
    let mut out = Vec::new();
    let mut rest = chars;
    while rest.len() > cols {
        // A space in the second half of the line: break after it.
        let cut = rest.get(..=cols).and_then(|head| head.iter().rposition(|c| *c == ' ')).filter(|&i| i >= cols / 2).map_or(cols, |i| i + 1);
        let (line, tail) = rest.split_at(cut.min(rest.len()));
        out.push(line.iter().copied().collect::<String>().trim_end().chars().collect());
        rest = tail;
    }
    out.push(rest.to_vec());
    out
}

/// A ToUnicode CMap for the glyph ids used (2-byte codes = glyph ids).
fn to_unicode_cmap(used: &BTreeMap<u32, char>) -> Vec<u8> {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Sintec-Text-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&u32, &char)> = used.iter().collect();
    // At most 100 entries per block (ISO 32000-2 §9.10.3).
    for chunk in entries.chunks(100) {
        s.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, ch) in chunk {
            let mut units = [0u16; 2];
            let hex: String = ch.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect();
            s.push_str(&format!("<{gid:04X}> <{hex}>\n"));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s.into_bytes()
}

/// Windows-1251 bytes 0x80–0xFF as Unicode (from the WHATWG encoding index).
const CP1251_HIGH: [char; 128] = [
    'Ђ', 'Ѓ', '‚', 'ѓ', '„', '…', '†', '‡', '€', '‰', 'Љ', '‹', 'Њ', 'Ќ', 'Ћ', 'Џ', //
    'ђ', '‘', '’', '“', '”', '•', '–', '—', '\u{98}', '™', 'љ', '›', 'њ', 'ќ', 'ћ', 'џ', //
    '\u{a0}', 'Ў', 'ў', 'Ј', '¤', 'Ґ', '¦', '§', 'Ё', '©', 'Є', '«', '¬', '\u{ad}', '®', 'Ї', //
    '°', '±', 'І', 'і', 'ґ', 'µ', '¶', '·', 'ё', '№', 'є', '»', 'ј', 'Ѕ', 'ѕ', 'ї', //
    'А', 'Б', 'В', 'Г', 'Д', 'Е', 'Ж', 'З', 'И', 'Й', 'К', 'Л', 'М', 'Н', 'О', 'П', //
    'Р', 'С', 'Т', 'У', 'Ф', 'Х', 'Ц', 'Ч', 'Ш', 'Щ', 'Ъ', 'Ы', 'Ь', 'Э', 'Ю', 'Я', //
    'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'й', 'к', 'л', 'м', 'н', 'о', 'п', //
    'р', 'с', 'т', 'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ъ', 'ы', 'ь', 'э', 'ю', 'я',
];

/// Text file bytes as a string: UTF-8 (BOM optional), UTF-16 with a BOM, else Windows-1251.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], le: bool| -> String {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&p| if le { u16::from_le_bytes(p) } else { u16::from_be_bytes(p) }).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes
            .iter()
            .map(|&b| if b < 0x80 { char::from(b) } else { CP1251_HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('\u{fffd}') })
            .collect(),
    }
}
