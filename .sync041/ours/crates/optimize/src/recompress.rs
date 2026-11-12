//! Lossless stream recompression: every stream in a general-purpose encoding (Flate, LZW,
//! RunLength, ASCII85, ASCIIHex, or none) is decoded and stored again with Flate at the best
//! level; image samples also get a PNG predictor (the best filter for each row), which is what
//! makes screenshots, charts and 1-bit scans small. The decoded bytes are exactly the same, so
//! nothing a page draws changes. A stream is replaced only when the new one is smaller.
//!
//! JPEG images are transcoded losslessly instead ([`crate::jpeg::transcode`]): the same
//! coefficients with Huffman tables made for them, baseline or progressive, without Exif and
//! other metadata no reader draws; kept only when smaller and decoding to exactly the same pixels.
//!
//! Left alone: the other image codecs (JPEG 2000, JBIG2, CCITT: their data is already compact
//! and decoding it would lose nothing only for CCITT), XMP metadata (kept readable, PDF/A needs it),
//! cross-reference and object streams (the writer makes new ones), streams with external data,
//! indirect or abbreviated filter descriptions, `Crypt` filters, and in an encrypted document
//! embedded files (they may use their own crypt filter).

use pdfcraft_cos::{Dict, Document, Object, Stream, serialize};
use pdfcraft_filters::Params;

/// Streams that decode to more than this are left as they are (memory, time).
pub(crate) const MAX_DECODED: usize = 256 << 20;

/// Streams smaller than this don't gain anything worth the work.
const MIN_STORED: usize = 64;

/// The general-purpose filters, which decode to exactly the stored data.
const GENERAL: [&[u8]; 10] =
    [b"FlateDecode", b"Fl", b"LZWDecode", b"LZW", b"RunLengthDecode", b"RL", b"ASCII85Decode", b"A85", b"ASCIIHexDecode", b"AHx"];

/// What kind of stream this is, for the estimator's statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// Image samples (a PNG predictor is tried too).
    Image,
    /// Any other stream that is already compressed.
    Compressed,
    /// Any other stream stored without a filter.
    Plain,
    /// A JPEG (DCTDecode only), transcoded losslessly.
    Jpeg,
}

/// Whether `s` may be recompressed, and what kind it is.
pub(crate) fn eligible(s: &Stream, encrypted: bool) -> Option<Kind> {
    let d = &s.dict;
    let ty = d.name(b"Type");
    if matches!(ty, Some(b"Metadata" | b"XRef" | b"ObjStm")) || (encrypted && ty == Some(b"EmbeddedFile")) {
        return None;
    }
    if s.raw.len() < MIN_STORED
        || d.contains(b"F")
        || d.contains(b"FFilter")
        || d.contains(b"DP")
        || matches!(d.get(b"DecodeParms"), Some(Object::Ref(_)))
    {
        return None;
    }
    let names: Vec<&[u8]> = match d.get(b"Filter") {
        None => Vec::new(),
        Some(Object::Name(n)) => vec![n.as_slice()],
        Some(Object::Array(a)) => a.iter().map(Object::as_name).collect::<Option<Vec<_>>>()?,
        Some(_) => return None,
    };
    if let [b"DCTDecode" | b"DCT"] = names.as_slice() {
        return Some(Kind::Jpeg);
    }
    if !names.iter().all(|n| GENERAL.contains(n)) {
        return None;
    }
    Some(if d.name(b"Subtype") == Some(b"Image") {
        Kind::Image
    } else if names.is_empty() {
        Kind::Plain
    } else {
        Kind::Compressed
    })
}

/// Colour components of an image colour space, whatever it is (for the row layout only).
fn colour_count(doc: &Document, cs: &Object) -> Option<i64> {
    let cs = doc.resolve(cs);
    match &*cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"G" | b"CalGray" => Some(1),
            b"DeviceRGB" | b"RGB" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            _ => None,
        },
        Object::Array(a) => match a.first()?.as_name()? {
            b"ICCBased" => match &*doc.resolve(a.get(1)?) {
                Object::Stream(icc) => icc.dict.int(b"N").filter(|n| (1..=32).contains(n)),
                _ => None,
            },
            b"CalGray" | b"Indexed" | b"I" | b"Separation" => Some(1),
            b"CalRGB" | b"Lab" => Some(3),
            b"DeviceN" => match &*doc.resolve(a.get(1)?) {
                Object::Array(names) => i64::try_from(names.len()).ok().filter(|n| (1..=32).contains(n)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// The predictor parameters matching an image's sample rows, if its layout is known.
fn image_params(doc: &Document, d: &Dict) -> Option<Params> {
    let w = d.int(b"Width").filter(|w| (1..=1 << 20).contains(w))?;
    let (colors, bpc) = if matches!(d.get(b"ImageMask"), Some(Object::Bool(true))) {
        (1, 1)
    } else {
        let bpc = d.int(b"BitsPerComponent").filter(|b| matches!(b, 1 | 2 | 4 | 8 | 16))?;
        (colour_count(doc, d.get(b"ColorSpace")?)?, bpc)
    };
    Some(Params { predictor: 15, colors, bits_per_component: bpc, columns: w, ..Params::default() })
}

/// Bytes a row of samples takes, for `p`.
fn row_bytes(p: &Params) -> Option<usize> {
    let bits = p.columns.checked_mul(p.colors)?.checked_mul(p.bits_per_component)?;
    usize::try_from(bits).ok().map(|b| b.div_ceil(8))
}

/// `s` recompressed losslessly, if that is smaller. `s` must be [`eligible`].
pub(crate) fn recompress(doc: &Document, s: &Stream) -> Option<Stream> {
    if eligible(s, false) == Some(Kind::Jpeg) {
        let data = crate::jpeg::transcode(&s.raw)?;
        let mut dict = s.dict.clone();
        dict.remove(b"Length");
        return Some(Stream::from_raw(dict, data));
    }
    let data = if s.dict.contains(b"Filter") { pdfcraft_filters::decode(&s.filters(), &s.raw, MAX_DECODED).ok()? } else { s.raw.to_vec() };
    let mut dict = s.dict.clone();
    for k in [&b"Length"[..], b"Filter", b"DecodeParms"] {
        dict.remove(k);
    }
    let params = if s.dict.name(b"Subtype") == Some(b"Image") { image_params(doc, &s.dict) } else { None };
    let best = flate_best(dict, &data, params.as_ref());
    (stored_len(&best)? < stored_len(s)?).then_some(best)
}

/// `data` stored with Flate at the best level under `dict`. When `rows` describes it as image
/// sample rows, a PNG predictor (the best filter for each row) is tried too and kept if smaller.
pub(crate) fn flate_best(mut dict: Dict, data: &[u8], rows: Option<&Params>) -> Stream {
    dict.remove(b"DecodeParms");
    let plain = Stream::flate_best(dict.clone(), data);
    let Some(p) = rows else { return plain };
    let Some(row) = row_bytes(p).filter(|r| *r > 0) else { return plain };
    if !data.len().is_multiple_of(row) {
        return plain;
    }
    let Ok(predicted) = pdfcraft_filters::apply_predictor(p, data) else { return plain };
    let packed = pdfcraft_filters::encode_flate_best(&predicted);
    if packed.len() >= plain.raw.len() {
        return plain;
    }
    let mut parms = Dict::new();
    parms.set(b"Predictor".to_vec(), Object::Int(15));
    parms.set(b"Colors".to_vec(), Object::Int(p.colors));
    parms.set(b"BitsPerComponent".to_vec(), Object::Int(p.bits_per_component));
    parms.set(b"Columns".to_vec(), Object::Int(p.columns));
    dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
    dict.set(b"DecodeParms".to_vec(), Object::Dict(parms));
    Stream::from_raw(dict, packed)
}

/// Predictor parameters for 8-bit samples with `n` components, `w` to a row.
pub(crate) fn rows8(n: usize, w: u32) -> Params {
    Params { predictor: 15, colors: n as i64, bits_per_component: 8, columns: i64::from(w), ..Params::default() }
}

/// Bytes the stream object takes in the file: its dictionary with `/Length` set as the writer
/// sets it, then its data.
pub(crate) fn stored_len(s: &Stream) -> Option<usize> {
    let mut dict = s.dict.clone();
    dict.set(b"Length".to_vec(), Object::Int(i64::try_from(s.raw.len()).ok()?));
    let mut out = Vec::new();
    serialize(&Object::Dict(dict), &mut out);
    out.len().checked_add(s.raw.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(doc: &mut Document, w: i64, h: i64, bpc: i64, cs: &str, data: &[u8], filter: bool) -> Stream {
        let _ = doc;
        let mut d = Dict::new();
        d.set(b"Subtype".to_vec(), Object::name("Image"));
        d.set(b"Width".to_vec(), Object::Int(w));
        d.set(b"Height".to_vec(), Object::Int(h));
        d.set(b"BitsPerComponent".to_vec(), Object::Int(bpc));
        d.set(b"ColorSpace".to_vec(), Object::name(cs));
        if filter { Stream::flate(d, data) } else { Stream::from_raw(d, data.to_vec()) }
    }

    #[test]
    fn an_image_gets_a_predictor_and_decodes_to_the_same_samples() {
        let mut doc = Document::new_empty();
        // A smooth RGB gradient: the PNG predictor turns it into near-constant rows.
        let (w, h) = (200i64, 120i64);
        let px: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x + y) as u8, (2 * x) as u8, (3 * y) as u8])).collect();
        let s = image(&mut doc, w, h, 8, "DeviceRGB", &px, true);
        assert_eq!(eligible(&s, false), Some(Kind::Image));
        let new = recompress(&doc, &s).expect("smaller");
        assert!(new.raw.len() < s.raw.len() / 2, "{} vs {}", new.raw.len(), s.raw.len());
        assert!(new.dict.get(b"DecodeParms").is_some());
        assert_eq!(new.decoded_strict().unwrap(), px);
    }

    #[test]
    fn a_one_bit_scan_stays_one_bit_and_lossless() {
        let mut doc = Document::new_empty();
        let (w, h) = (300i64, 200i64);
        let row = (w as usize).div_ceil(8);
        // Vertical stripes and a few dots, as on a scanned form.
        let px: Vec<u8> = (0..h as usize).flat_map(|y| (0..row).map(move |x| if (x + y / 7) % 5 == 0 { 0x0F } else { 0xFF })).collect();
        let s = image(&mut doc, w, h, 1, "DeviceGray", &px, false);
        let new = recompress(&doc, &s).expect("smaller");
        assert_eq!(new.dict.int(b"BitsPerComponent"), Some(1));
        assert_eq!(new.decoded_strict().unwrap(), px);
    }

    #[test]
    fn content_is_recompressed_at_the_best_level_and_kept_only_when_smaller() {
        let doc = Document::new_empty();
        let text: Vec<u8> = (0..400).flat_map(|i| format!("BT /F1 12 Tf 72 {} Td (Line {i} of the report) Tj ET\n", 700 - i).into_bytes()).collect();
        let plain = Stream::from_raw(Dict::new(), text.clone());
        assert_eq!(eligible(&plain, false), Some(Kind::Plain));
        let new = recompress(&doc, &plain).unwrap();
        assert_eq!(new.decoded_strict().unwrap(), text);
        // Already at the best level: nothing to gain, so it stays.
        assert!(recompress(&doc, &new).is_none());
    }

    #[test]
    fn what_is_not_eligible() {
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::name("DCTDecode"));
        assert_eq!(eligible(&Stream::from_raw(d, vec![0; 100]), false), Some(Kind::Jpeg), "JPEG is transcoded");
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::name("JPXDecode"));
        assert_eq!(eligible(&Stream::from_raw(d, vec![0; 100]), false), None, "image codecs");
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"), Object::name("DCTDecode")]));
        assert_eq!(eligible(&Stream::from_raw(d.clone(), vec![0; 100]), false), None, "JPEG under another filter");
        // A JPEG that isn't one is left as it is.
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::name("DCTDecode"));
        assert!(recompress(&Document::new_empty(), &Stream::from_raw(d, [0xFF, 0xD8, 0xFF, 0xC0, 0, 2].repeat(20))).is_none());
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("Metadata"));
        assert_eq!(eligible(&Stream::from_raw(d, vec![b'a'; 100]), false), None, "XMP stays readable");
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"), Object::name("Crypt")]));
        assert_eq!(eligible(&Stream::from_raw(d, vec![0; 100]), false), None, "Crypt");
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("EmbeddedFile"));
        assert_eq!(eligible(&Stream::from_raw(d.clone(), vec![b'a'; 100]), true), None, "encrypted attachment");
        assert!(eligible(&Stream::from_raw(d, vec![b'a'; 100]), false).is_some());
        assert_eq!(eligible(&Stream::from_raw(Dict::new(), vec![b'a'; 10]), false), None, "tiny");
    }

    #[test]
    fn corrupt_data_is_left_alone() {
        let doc = Document::new_empty();
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let s = Stream::from_raw(d, [0x78, 0x9c, 1, 2, 3, 4, 5, 6, 7, 8].repeat(10));
        assert!(eligible(&s, false).is_some());
        assert!(recompress(&doc, &s).is_none());
    }

    #[test]
    fn odd_image_geometry_falls_back_to_plain_flate() {
        let mut doc = Document::new_empty();
        // Width and data that don't line up in rows: no predictor, still lossless.
        let px = vec![7u8; 1001];
        let s = image(&mut doc, 10, 10, 8, "DeviceRGB", &px, false);
        let new = recompress(&doc, &s).unwrap();
        assert!(new.dict.get(b"DecodeParms").is_none());
        assert_eq!(new.decoded_strict().unwrap(), px);
        // An absurd width doesn't overflow.
        let s = image(&mut doc, i64::MAX, 1, 16, "DeviceCMYK", &px, false);
        assert!(recompress(&doc, &s).is_some());
    }
}
