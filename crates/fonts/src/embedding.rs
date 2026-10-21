//! Unicode embedding of an explicitly supplied, embeddable TrueType face. Font files are
//! document inputs or the optional craft-fonts build input, never system-interface fallbacks.

use std::fmt::Write as _;

use pdfcraft_cos::{Dict, Document, Object, PdfString, Stream};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
    raw::TableProvider,
};

/// Compare PDF subset/PostScript names with the full face's family name.
pub fn face_key(name: &str) -> String {
    name.rsplit('+').next().unwrap_or(name).chars().filter(char::is_ascii_alphanumeric).flat_map(char::to_lowercase).collect()
}

/// Sintec.PDF: a PDF font's name as a family key: no subset tag (`ABCDEF+`), no style or foundry suffix
/// (`,Bold`, `-BoldItalicMT`, `PSMT`), lower-case letters and digits only.
/// `ABCDEF+TimesNewRomanPS-BoldItalicMT` → `timesnewroman`.
pub fn family_key(name: &str) -> String {
    let name = name.rsplit('+').next().unwrap_or(name);
    // The family is what precedes the style part (`-Bold`, `,Italic`).
    let family = name.split([',', '-']).next().unwrap_or(name);
    let mut key: String = family.chars().filter(char::is_ascii_alphanumeric).flat_map(char::to_lowercase).collect();
    for suffix in ["psmt", "mt", "ps", "regular", "roman", "bolditalic", "boldoblique", "bold", "italic", "oblique"] {
        // "TimesNewRoman" keeps its "roman"; a bare suffix only comes off a longer name.
        if suffix == "roman" && key.starts_with("timesnew") {
            continue;
        }
        if key.len() > suffix.len() + 2
            && let Some(stripped) = key.strip_suffix(suffix)
        {
            key = stripped.to_string();
        }
    }
    key
}

/// Full, approved faces of the source family. This never silently selects another family.
pub fn matching_faces(name: &str) -> Vec<&'static crate::CraftFont> {
    let key = face_key(name);
    crate::CRAFT_FONTS.iter().filter(|f| face_key(f.family) == key || face_key(&f.name()) == key).collect()
}

/// Whether a complete font program can show every non-newline character.
pub fn covers(bytes: &[u8], text: &str) -> bool {
    FontRef::new(bytes).is_ok_and(|f| text.chars().all(|c| matches!(c, '\n' | '\r') || f.charmap().map(c).is_some_and(|g| g.to_u32() != 0)))
}

/// Names come from the program, never a caller's guessed family.
pub fn names(bytes: &[u8]) -> Result<(String, String), String> {
    use skrifa::string::StringId;
    validate(bytes)?;
    let f = FontRef::new(bytes).map_err(|e| e.to_string())?;
    let name = |id| {
        f.localized_strings(id)
            .find(|s| s.language() == Some("en"))
            .or_else(|| f.localized_strings(id).next())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty() && s.len() <= 128)
    };
    let family = name(StringId::FAMILY_NAME).ok_or("the font has no family name")?;
    let postscript = name(StringId::POSTSCRIPT_NAME).unwrap_or_else(|| family.replace(' ', ""));
    Ok((family, postscript))
}

/// A selected external font is document input.
///
/// Sintec.PDF: an installed font may be embedded when its own embedding permission (OS/2
/// `fsType`, checked by [`validate`]) allows editable embedding, as office suites and Acrobat
/// embed them. Restricted and preview-and-print-only fonts are refused there; Adobe-authored
/// faces and the families the asset policy excludes are refused here. Upstream PdfCraft also
/// requires an open-licence declaration in the name table, which leaves out the system fonts
/// (Arial, Times New Roman, Calibri…) that edited documents need as substitutes.
pub fn validate_document_input(bytes: &[u8]) -> Result<(), String> {
    use skrifa::string::StringId;
    let (family, _) = names(bytes)?;
    let key = face_key(&family);
    if ["sourcehan", "sourceserif", "sourcesans", "notosanscjk", "notoserifcjk"].iter().any(|n| key.starts_with(n)) {
        return Err("this font family is excluded by the project's asset policy".into());
    }
    let f = FontRef::new(bytes).map_err(|e| e.to_string())?;
    let attribution: String =
        [StringId::COPYRIGHT_NOTICE, StringId::MANUFACTURER].into_iter().flat_map(|id| f.localized_strings(id)).map(|s| s.to_string()).collect();
    if attribution.to_ascii_lowercase().contains("adobe") {
        return Err("Adobe-authored font assets are excluded by the project's asset policy".into());
    }
    Ok(())
}

/// Sintec.PDF: the family and PostScript names a font program's name table gives, read
/// leniently: font programs embedded in PDFs often fail [`validate`] (no cmap, stripped tables)
/// but still say what they are. `None` when there's no usable name.
pub fn program_names(bytes: &[u8]) -> Option<(String, String)> {
    use skrifa::string::StringId;
    let f = FontRef::new(bytes).ok()?;
    let name = |id| {
        f.localized_strings(id)
            .find(|s| s.language() == Some("en"))
            .or_else(|| f.localized_strings(id).next())
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty() && s.len() <= 128)
    };
    // The family as installed fonts are listed (name ID 1), so the two compare.
    let family = name(StringId::FAMILY_NAME).or_else(|| name(StringId::TYPOGRAPHIC_FAMILY_NAME))?;
    let postscript = name(StringId::POSTSCRIPT_NAME).unwrap_or_else(|| family.replace(' ', ""));
    Some((family, postscript))
}

/// Sintec.PDF: what a face looks like, for matching it to a PDF's font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceStyle {
    /// OS/2 weight class (400 regular, 700 bold).
    pub weight: u16,
    pub italic: bool,
    pub monospace: bool,
    /// From the PANOSE serif style when the face declares it.
    pub serif: Option<bool>,
    /// Whether it draws Russian (А–я, Ё, ё).
    pub cyrillic: bool,
}

/// The face's style, read from its OS/2, post and cmap tables (defaults where they're missing).
pub fn style_of(bytes: &[u8]) -> Result<FaceStyle, String> {
    let f = FontRef::new(bytes).map_err(|e| e.to_string())?;
    let os2 = f.os2().ok();
    let weight = os2.as_ref().map_or(400, |t| t.us_weight_class());
    // fsSelection bit 0 is ITALIC, bit 9 OBLIQUE; the head table's macStyle bit 1 is italic too.
    let italic = os2.as_ref().is_some_and(|t| t.fs_selection().bits() & (1 | 1 << 9) != 0) || f.head().is_ok_and(|h| h.mac_style().bits() & 2 != 0);
    let monospace = f.post().is_ok_and(|p| p.is_fixed_pitch() != 0);
    // PANOSE byte 0 is the family kind (2 = Latin text) and byte 1 the serif style: 11–13 are
    // sans serif, 2–10 serifs, 0–1 any/no fit.
    let serif = os2.as_ref().and_then(|t| {
        let p = t.panose_10();
        match (p.first().copied(), p.get(1).copied()) {
            (Some(2), Some(2..=10)) => Some(true),
            (Some(2), Some(11..=13)) => Some(false),
            _ => None,
        }
    });
    let cmap = f.charmap();
    let cyrillic = ['А', 'Я', 'а', 'я', 'Ё', 'ё', 'Ж', 'ж'].iter().all(|&c| cmap.map(c).is_some_and(|g| g.to_u32() != 0));
    Ok(FaceStyle { weight, italic, monospace, serif, cyrillic })
}

/// Sintec.PDF: the characters whose advance widths make up a face's width fingerprint: Latin and
/// Cyrillic letters, digits and common punctuation.
pub fn probe_chars() -> impl Iterator<Item = char> {
    ('A'..='Z').chain('a'..='z').chain('А'..='я').chain(['Ё', 'ё']).chain('0'..='9').chain(".,:;!?-()/%&@'\"".chars())
}

/// Sintec.PDF: a face's advance widths per 1000 em for the [`probe_chars`] it has. Empty when
/// the face can't be read or has no usable units per em.
pub fn probe_widths(bytes: &[u8]) -> Vec<(char, u16)> {
    let Ok(f) = FontRef::new(bytes) else { return Vec::new() };
    let upem = f.metrics(Size::unscaled(), LocationRef::default()).units_per_em;
    if upem == 0 {
        return Vec::new();
    }
    let (cmap, gm) = (f.charmap(), f.glyph_metrics(Size::unscaled(), LocationRef::default()));
    probe_chars()
        .filter_map(|c| {
            let gid = cmap.map(c).filter(|g| g.to_u32() != 0)?;
            let w = gm.advance_width(gid).filter(|w| w.is_finite())? * 1000.0 / f32::from(upem);
            Some((c, w.round().clamp(0.0, 4000.0) as u16))
        })
        .collect()
}

/// Validate before allocating any PDF objects, so an invalid font cannot partly mutate a doc.
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 16 << 20 {
        return Err("the selected font exceeds 16 MiB".into());
    }
    let font = FontRef::new(bytes).map_err(|_| "select a standalone TrueType/OpenType font (not a font collection)")?;
    if font.fvar().is_ok() {
        return Err("select a static instance of this variable font before embedding".into());
    }
    if (font.glyf().is_err() && font.cff().is_err()) || font.head().is_err() || font.hhea().is_err() || font.hmtx().is_err() {
        return Err("the selected font has no usable TrueType/CFF outlines and metrics".into());
    }
    if font.os2().is_ok_and(|t| t.fs_type() & (0x0002 | 0x0004 | 0x0200) != 0) {
        return Err("this font's embedding permissions prohibit editable outline embedding".into());
    }
    let m = font.metrics(Size::unscaled(), LocationRef::default());
    if m.units_per_em == 0 || font.charmap().mappings().next().is_none() {
        return Err("the selected font has no Unicode mapping or valid units per em".into());
    }
    if font.charmap().mappings().take(65_535).count() >= 65_535 {
        return Err("the selected font has too many Unicode mappings for one PDF font".into());
    }
    if font.charmap().mappings().any(|(_, g)| g.to_u32() >= u32::from(m.glyph_count)) {
        return Err("the selected font maps Unicode to a glyph outside its program".into());
    }
    Ok(())
}

fn stream(doc: &mut Document, bytes: Vec<u8>) -> Object {
    Object::Ref(doc.add(Object::Stream(Stream::from_raw(Dict::new(), bytes))))
}

/// Embed the complete font once and provide separate CIDs for Unicode aliases. /ToUnicode,
/// /CIDToGIDMap and widths agree, including supplementary characters, so reopening needs no
/// installed font. Keeping the full program also lets subsequent edits add other glyphs.
pub fn embed(doc: &mut Document, fonts: &mut Dict, bytes: &[u8], family: &str) -> Result<String, String> {
    validate(bytes)?;
    let font = FontRef::new(bytes).map_err(|e| e.to_string())?;
    let mappings: Vec<_> = font
        .charmap()
        .mappings()
        .filter(|(_, gid)| gid.to_u32() > 0)
        .filter_map(|(cp, gid)| char::from_u32(cp).map(|c| (c.to_string(), gid)))
        .collect();
    embed_mapping(doc, fonts, bytes, family, &mappings)
}

/// Explicit glyph mapping for shaped text. A CID can represent a contextual glyph or a
/// ligature, while its ToUnicode retains the logical cluster. The full source program stays exact.
pub fn embed_mapping(
    doc: &mut Document,
    fonts: &mut Dict,
    bytes: &[u8],
    family: &str,
    mappings: &[(String, skrifa::GlyphId)],
) -> Result<String, String> {
    validate(bytes)?;
    if mappings.len() > 65_534 || mappings.iter().any(|(s, _)| s.len() > 4096) {
        return Err("shaped font mapping exceeds its budget".into());
    }
    let font = FontRef::new(bytes).map_err(|e| e.to_string())?;
    let cff = font.glyf().is_err();
    let loc = LocationRef::default();
    let m = font.metrics(Size::unscaled(), loc);
    let scale = 1000.0 / f64::from(m.units_per_em);
    let advances = font.glyph_metrics(Size::unscaled(), loc);
    let mut cid_encoding = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapName /PCGlyphs def\n/CMapType 1 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let mut cff_widths = std::collections::BTreeMap::new();
    let mut gids = vec![0, 0];
    let mut widths = Vec::with_capacity(mappings.len());
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapName /PCUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for (chunk_index, chunk) in mappings.chunks(100).enumerate() {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        let _ = writeln!(cid_encoding, "{} begincidchar", chunk.len());
        for (offset, (unicode, gid)) in chunk.iter().enumerate() {
            let cid = u16::try_from(chunk_index.saturating_mul(100).saturating_add(offset).saturating_add(1)).map_err(|_| "font mapping limit")?;
            let glyph = u16::try_from(gid.to_u32()).map_err(|_| "font glyph limit")?;
            if glyph == 0 || u32::from(glyph) >= u32::from(m.glyph_count) {
                return Err("font contains an unavailable shaped glyph".into());
            }
            let width = advances.advance_width(*gid).unwrap_or(0.0) as f64 * scale;
            if !width.is_finite() || !(0.0..=16_000.0).contains(&width) {
                return Err("font contains an invalid advance".into());
            }
            gids.extend_from_slice(&glyph.to_be_bytes());
            widths.push(Object::Real(width));
            cff_widths.insert(glyph, width);
            let _ = writeln!(cid_encoding, "<{cid:04X}> {glyph}");
            let hex: String = unicode.encode_utf16().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(cmap, "<{cid:04X}> <{hex}>");
        }
        cmap.push_str("endbfchar\n");
        cid_encoding.push_str("endcidchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cid_encoding.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let name = family.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect::<String>();
    if name.is_empty() || name.len() > 128 {
        return Err("the font family name is invalid".into());
    }
    let mut descriptor = Dict::new();
    descriptor.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    descriptor.set(b"FontName".to_vec(), Object::name(&name));
    descriptor.set(b"FontFamily".to_vec(), Object::String(PdfString::literal(family.as_bytes().to_vec())));
    descriptor.set(b"Flags".to_vec(), Object::Int(4));
    descriptor.set(b"ItalicAngle".to_vec(), Object::Real(f64::from(m.italic_angle)));
    descriptor.set(b"Ascent".to_vec(), Object::Real(f64::from(m.ascent) * scale));
    descriptor.set(b"Descent".to_vec(), Object::Real(f64::from(m.descent) * scale));
    descriptor.set(b"CapHeight".to_vec(), Object::Real(f64::from(m.cap_height.unwrap_or(m.ascent)) * scale));
    descriptor.set(b"StemV".to_vec(), Object::Int(80));
    let bounds = m.bounds.ok_or("font has no outline bounds")?;
    descriptor.set(
        b"FontBBox".to_vec(),
        Object::Array([bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max].map(|v| Object::Real(f64::from(v) * scale)).to_vec()),
    );
    // Reuse a full program registered on another page; source subset programs stay untouched.
    let existing = doc.scan_objects().take(8192).find_map(|(r, o)| match o.as_ref() {
        Object::Stream(s) if s.dict.get(b"PCFullFont").is_some() && s.raw.as_ref() == bytes => Some(Object::Ref(r)),
        _ => None,
    });
    let program = existing.unwrap_or_else(|| {
        let mut d = Dict::new();
        d.set(b"Length1".to_vec(), Object::Int(bytes.len() as i64));
        if cff {
            d.set(b"Subtype".to_vec(), Object::name("OpenType"));
        }
        d.set(b"PCFullFont".to_vec(), Object::Bool(true));
        Object::Ref(doc.add(Object::Stream(Stream::from_raw(d, bytes.to_vec()))))
    });
    descriptor.set(if cff { b"FontFile3".to_vec() } else { b"FontFile2".to_vec() }, program);
    let desc = Object::Ref(doc.add(Object::Dict(descriptor)));
    let mut system = Dict::new();
    system.set(b"Registry".to_vec(), Object::String(PdfString::literal(b"Adobe".to_vec())));
    system.set(b"Ordering".to_vec(), Object::String(PdfString::literal(b"Identity".to_vec())));
    system.set(b"Supplement".to_vec(), Object::Int(0));
    let mut cid = Dict::new();
    cid.set(b"Type".to_vec(), Object::name("Font"));
    cid.set(b"Subtype".to_vec(), Object::name(if cff { "CIDFontType0" } else { "CIDFontType2" }));
    cid.set(b"BaseFont".to_vec(), Object::name(&name));
    cid.set(b"CIDSystemInfo".to_vec(), Object::Dict(system));
    cid.set(b"FontDescriptor".to_vec(), desc);
    if cff {
        cid.set(
            b"W".to_vec(),
            Object::Array(cff_widths.into_iter().flat_map(|(gid, w)| [Object::Int(i64::from(gid)), Object::Array(vec![Object::Real(w)])]).collect()),
        );
    } else {
        cid.set(b"CIDToGIDMap".to_vec(), stream(doc, gids));
        cid.set(b"W".to_vec(), Object::Array(vec![Object::Int(1), Object::Array(widths)]));
    }
    let descendant = Object::Ref(doc.add(Object::Dict(cid)));
    let mut top = Dict::new();
    top.set(b"Type".to_vec(), Object::name("Font"));
    top.set(b"Subtype".to_vec(), Object::name("Type0"));
    top.set(b"BaseFont".to_vec(), Object::name(&name));
    top.set(b"Encoding".to_vec(), if cff { stream(doc, cid_encoding.into_bytes()) } else { Object::name("Identity-H") });
    top.set(b"DescendantFonts".to_vec(), Object::Array(vec![descendant]));
    top.set(b"ToUnicode".to_vec(), stream(doc, cmap.into_bytes()));
    let mut index = 0usize;
    while fonts.contains(format!("PCFull{index}").as_bytes()) {
        index = index.saturating_add(1);
    }
    let resource = format!("PCFull{index}");
    fonts.set(resource.as_bytes().to_vec(), Object::Ref(doc.add(Object::Dict(top))));
    Ok(resource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::Metrics;

    #[test]
    fn full_unicode_embedding_keeps_outlines_advances_and_roundtrip_codes() {
        let bytes = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let mut doc = Document::new_empty();
        let mut fonts = Dict::new();
        let name = embed(&mut doc, &mut fonts, bytes, "Inter").unwrap();
        let font = doc.dict(fonts.get(name.as_bytes()).unwrap()).unwrap();
        let m = Metrics::from_dict(&doc, &font);
        let text = "Aé ΩЖ €";
        let codes = m.encode(text).unwrap();
        assert_eq!(m.decode(&codes), text);
        assert_eq!(crate::pdf::embedded_sfnt(&doc, &font).unwrap(), bytes);
        let sfnt = FontRef::new(bytes).unwrap();
        for (ch, (code, _)) in text.chars().zip(m.codes(&codes)) {
            let gid = sfnt.charmap().map(ch).unwrap();
            let w = sfnt.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid).unwrap() as f64
                / f64::from(sfnt.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
            assert!((m.width(code) - w).abs() < 1e-8);
        }
        let before = doc.object_numbers().len();
        assert!(embed(&mut doc, &mut fonts, b"broken font", "Broken").is_err());
        assert!(embed_mapping(&mut doc, &mut fonts, bytes, "Inter", &[("X".into(), skrifa::GlyphId::new(65_535))]).is_err());
        assert_eq!(doc.object_numbers().len(), before);
        let programs = || doc.scan_objects().filter(|(_, o)| matches!(o.as_ref(), Object::Stream(s) if s.dict.get(b"PCFullFont").is_some())).count();
        assert_eq!(programs(), 1);
        let mut second = Dict::new();
        embed(&mut doc, &mut second, bytes, "Inter").unwrap();
        assert_eq!(doc.scan_objects().filter(|(_, o)| matches!(o.as_ref(), Object::Stream(s) if s.dict.get(b"PCFullFont").is_some())).count(), 1);
        assert_eq!(face_key("ABCDEF+Droid-Sans-Fallback"), face_key("Droid Sans Fallback"));
    }
}
