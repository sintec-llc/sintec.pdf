//! Image resampling and recompression.

use std::collections::{HashMap, HashSet};

use pdfcraft_content::Matrix;
use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};

use crate::{Compression, ImageSettings, OptimizeError, Report, Settings, Stage};

/// Images with more pixels than this are left alone (memory).
const MAX_PIXELS: u64 = 80_000_000;

/// The resources of a page, inherited from the page tree if the page has none.
pub(crate) fn page_resources(doc: &Document, page: ObjRef) -> Option<Dict> {
    let mut node = Some(page);
    for _ in 0..64 {
        let d = doc.get(node?).as_dict()?.clone();
        if let Some(r) = d.get(b"Resources") {
            return doc.resolve(r).as_dict().cloned();
        }
        node = d.reference(b"Parent");
    }
    None
}

pub(crate) fn content_bytes(doc: &Document, contents: &Object) -> Vec<u8> {
    match &*doc.resolve(contents) {
        Object::Stream(s) => s.decoded().unwrap_or_default(),
        Object::Array(a) => a
            .iter()
            .flat_map(|c| match &*doc.resolve(c) {
                Object::Stream(s) => {
                    let mut v = s.decoded().unwrap_or_default();
                    v.push(b'\n');
                    v
                }
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Forms nested deeper than this aren't followed.
const MAX_FORM_DEPTH: usize = 12;
/// Walks of a form already walked once (other scale or caller) before the measurement gives up.
/// Each form's first walk is free: their number is bounded by the file's object count.
const MAX_REWALKS: usize = 100_000;

/// State of one [`effective_resolutions`] pass.
struct Walk<'a> {
    doc: &'a Document,
    /// The forms being walked right now, outermost first: a form drawing one of these is a cycle.
    path: Vec<ObjRef>,
    /// Forms with their own resources already walked, by the linear part of their matrix. Drawing
    /// one again at the same scale (only moved) can't change any image's resolution.
    walked: HashSet<(ObjRef, [u64; 4])>,
    /// Forms walked at least once.
    seen: HashSet<ObjRef>,
    rewalks: usize,
    /// False once a use may have been missed (depth or work limit).
    complete: bool,
    out: HashMap<ObjRef, f64>,
}

impl Walk<'_> {
    /// Walk one content stream, recording each image's displayed resolution (pixels per inch, the
    /// smaller of the two axes) and descending into form XObjects.
    fn content(&mut self, data: &[u8], resources: &Dict, ctm: Matrix) {
        let doc = self.doc;
        let xobjects = resources.get(b"XObject").map(|x| doc.resolve(x)).and_then(|x| x.as_dict().cloned()).unwrap_or_default();
        let mut stack = vec![ctm];
        for op in pdfcraft_content::parse(data).ops {
            let top = stack.last().copied().unwrap_or(ctm);
            match op.op.as_slice() {
                b"q" => stack.push(top),
                b"Q" if stack.len() > 1 => {
                    stack.pop();
                }
                b"cm" => {
                    if let Some(m) = Matrix::from_operands(&op.operands)
                        && let Some(t) = stack.last_mut()
                    {
                        *t = m.then(&top);
                    }
                }
                b"Do" => {
                    let Some(name) = op.name(0) else { continue };
                    let Some(r) = xobjects.get(name).and_then(Object::as_ref) else { continue };
                    let obj = doc.get(r);
                    let Object::Stream(s) = &*obj else { continue };
                    match s.dict.name(b"Subtype") {
                        Some(b"Image") => {
                            let (w, h) = (s.dict.int(b"Width").unwrap_or(0) as f64, s.dict.int(b"Height").unwrap_or(0) as f64);
                            let [a, b, c, d, ..] = top.0;
                            let (sx, sy) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
                            if w > 0.0 && h > 0.0 && sx > 1e-6 && sy > 1e-6 {
                                let ppi = (w / (sx / 72.0)).min(h / (sy / 72.0));
                                let e = self.out.entry(r).or_insert(f64::INFINITY);
                                *e = e.min(ppi);
                            }
                        }
                        Some(b"Form") => self.form(r, s, resources, top),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// Descend into form `r` drawn with `ctm` from a stream whose resources are `resources`.
    fn form(&mut self, r: ObjRef, s: &Stream, resources: &Dict, ctm: Matrix) {
        if self.path.contains(&r) {
            return;
        }
        if self.path.len() >= MAX_FORM_DEPTH {
            self.complete = false;
            return;
        }
        let doc = self.doc;
        let m = s.dict.get(b"Matrix").map(|m| doc.resolve(m)).and_then(|m| m.as_array().map(|a| Matrix::from_operands(a))).flatten();
        let inner = m.unwrap_or(Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])).then(&ctm);
        let own = s.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned());
        // A form without resources uses its caller's, so the same form can draw other images.
        if own.is_some() {
            let [a, b, c, d, ..] = inner.0;
            if !self.walked.insert((r, [a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits()])) {
                return;
            }
        }
        if !self.seen.insert(r) {
            if self.rewalks >= MAX_REWALKS {
                self.complete = false;
                return;
            }
            self.rewalks += 1;
        }
        self.path.push(r);
        self.content(&s.decoded().unwrap_or_default(), own.as_ref().unwrap_or(resources), inner);
        self.path.pop();
    }
}

/// The effective resolution (ppi) of every image drawn by `pages`: the smallest over its uses.
///
/// If forms nest too deeply or are drawn too many times to measure every use, every image is
/// given `INFINITY` (never resampled), since its largest use may be among those missed.
pub fn effective_resolutions(doc: &Document, pages: &[ObjRef]) -> HashMap<ObjRef, f64> {
    let mut walk = Walk { doc, path: Vec::new(), walked: HashSet::new(), seen: HashSet::new(), rewalks: 0, complete: true, out: HashMap::new() };
    for p in pages {
        let Some(d) = doc.get(*p).as_dict().cloned() else { continue };
        let Some(contents) = d.get(b"Contents") else { continue };
        let res = page_resources(doc, *p).unwrap_or_default();
        walk.content(&content_bytes(doc, contents), &res, Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    }
    if !walk.complete {
        walk.out.values_mut().for_each(|v| *v = f64::INFINITY);
    }
    walk.out
}

/// Colour components of a colour space PdfCraft resamples: gray (1) or RGB (3).
pub(crate) fn components(doc: &Document, cs: &Object) -> Option<usize> {
    let cs = doc.resolve(cs);
    match &*cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"G" => Some(1),
            b"DeviceRGB" | b"RGB" => Some(3),
            _ => None,
        },
        Object::Array(a) => match a.first()?.as_name()? {
            b"ICCBased" => match &*doc.resolve(a.get(1)?) {
                Object::Stream(icc) => match icc.dict.int(b"N")? {
                    1 => Some(1),
                    3 => Some(3),
                    _ => None,
                },
                _ => None,
            },
            b"CalGray" => Some(1),
            b"CalRGB" => Some(3),
            _ => None,
        },
        _ => None,
    }
}

fn is_dct(s: &Stream) -> Option<bool> {
    match s.dict.get(b"Filter") {
        None => Some(false),
        Some(Object::Name(n)) => match n.as_slice() {
            b"DCTDecode" | b"DCT" => Some(true),
            b"FlateDecode" | b"Fl" | b"LZWDecode" | b"LZW" | b"RunLengthDecode" | b"RL" | b"ASCII85Decode" | b"A85" | b"ASCIIHexDecode" | b"AHx" => {
                Some(false)
            }
            _ => None,
        },
        Some(Object::Array(a)) => {
            let names: Vec<&[u8]> = a.iter().filter_map(Object::as_name).collect();
            if names.len() != a.len() {
                return None;
            }
            match names.as_slice() {
                [b"DCTDecode" | b"DCT"] => Some(true),
                n if n.iter().all(|f| {
                    matches!(
                        *f,
                        b"FlateDecode"
                            | b"Fl"
                            | b"LZWDecode"
                            | b"LZW"
                            | b"RunLengthDecode"
                            | b"RL"
                            | b"ASCII85Decode"
                            | b"A85"
                            | b"ASCIIHexDecode"
                            | b"AHx"
                    )
                }) =>
                {
                    Some(false)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// 8-bit samples of an image (`n` components), decoded.
pub(crate) fn pixels(s: &Stream, n: usize) -> Option<Vec<u8>> {
    let (w, h) = (s.dict.int(b"Width")? as usize, s.dict.int(b"Height")? as usize);
    if is_dct(s)? {
        let img = image::load_from_memory_with_format(&s.raw, image::ImageFormat::Jpeg).ok()?;
        // Adobe CMYK/YCCK JPEGs aren't handled (the colour space check excludes them).
        let v = if n == 1 { img.into_luma8().into_raw() } else { img.into_rgb8().into_raw() };
        (v.len() == w * h * n).then_some(v)
    } else {
        let v = s.decoded().ok()?;
        (v.len() >= w * h * n).then(|| v[..w * h * n].to_vec())
    }
}

/// How samples are resampled: Lanczos (3 lobes) for photos and scans, a tent (area-like, no
/// ringing) for soft masks, whose hard edges would get halos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    Photo,
    Hard,
}

pub(crate) fn resample(px: &[u8], n: usize, w: u32, h: u32, nw: u32, nh: u32, filter: Filter) -> Option<Vec<u8>> {
    use image::imageops::{FilterType, resize};
    let f = match filter {
        Filter::Photo => FilterType::Lanczos3,
        Filter::Hard => FilterType::Triangle,
    };
    Some(match n {
        1 => resize(&image::GrayImage::from_raw(w, h, px.to_vec())?, nw, nh, f).into_raw(),
        _ => resize(&image::RgbImage::from_raw(w, h, px.to_vec())?, nw, nh, f).into_raw(),
    })
}

/// Whether [`pixels`] reads `s` as it is drawn: a codec it decodes and no decode array that
/// changes anything. `None` when the colour space isn't grey or RGB.
pub(crate) fn decodable(doc: &Document, s: &Stream) -> Option<bool> {
    let n = components(doc, s.dict.get(b"ColorSpace")?)?;
    Some(is_dct(s).is_some() && s.dict.get(b"Decode").is_none_or(|d| is_identity_decode(doc, d, n)))
}

/// Whether `decode` is the default decode array for `n` components ([0 1] each), which changes
/// nothing. Scanners write it on page images (#490).
fn is_identity_decode(doc: &Document, decode: &Object, n: usize) -> bool {
    let decode = doc.resolve(decode);
    let Some(a) = decode.as_array() else { return false };
    a.len() == 2 * n && a.chunks(2).all(|p| matches!(p, [lo, hi] if doc.resolve(lo).as_f64() == Some(0.0) && doc.resolve(hi).as_f64() == Some(1.0)))
}

/// The image dictionary `old` for new samples: size, 8 bits, no filter or decode array (only
/// images without one, or with the default one, get here).
fn new_dict(old: &Dict, w: u32, h: u32) -> Dict {
    let mut d = old.clone();
    for k in [&b"Filter"[..], b"DecodeParms", b"Decode", b"Length", b"DL"] {
        d.remove(k);
    }
    d.set(b"Width".to_vec(), Object::Int(i64::from(w)));
    d.set(b"Height".to_vec(), Object::Int(i64::from(h)));
    d.set(b"BitsPerComponent".to_vec(), Object::Int(8));
    d
}

/// A JPEG image stream from `data` under `old`'s dictionary.
fn jpeg_stream(old: &Dict, data: Vec<u8>, w: u32, h: u32) -> Stream {
    let mut d = new_dict(old, w, h);
    d.set(b"Filter".to_vec(), Object::name("DCTDecode"));
    Stream::from_raw(d, data)
}

/// A new image stream from 8-bit samples: the original dictionary with the new size and filter.
pub(crate) fn image_stream(old: &Dict, px: &[u8], n: usize, w: u32, h: u32, compression: Compression, was_dct: bool) -> Option<Stream> {
    let quality = match compression {
        Compression::Jpeg(q) => Some(q),
        Compression::Retain if was_dct => Some(80),
        _ => None,
    };
    if let Some(q) = quality {
        let sub = n == 3 && crate::quality::chroma_halves_well(px, w as usize, h as usize);
        Some(jpeg_stream(old, crate::jpeg::encode(px, n, w, h, q, sub)?, w, h))
    } else {
        Some(crate::recompress::flate_best(new_dict(old, w, h), px, Some(&crate::recompress::rows8(n, w))))
    }
}

/// `px` as an indexed image when it has few colours (at most 256, or 16 for grey, which would
/// gain nothing from 8-bit indices): 1, 2, 4 or 8 bits a pixel, the palette in `old`'s colour
/// space. Lossless.
fn palette_stream(old: &Dict, px: &[u8], n: usize, w: u32, h: u32) -> Option<Stream> {
    let (entries, idx) = crate::quality::palette(px, n, if n == 1 { 16 } else { 256 })?;
    let count = entries.len() / n.max(1);
    let bits: usize = match count {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    };
    let wu = usize::try_from(w).ok()?.max(1);
    let row = (wu * bits).div_ceil(8);
    let mut packed = Vec::with_capacity(row * (idx.len() / wu));
    for r in idx.chunks(wu) {
        let mut line = vec![0u8; row];
        for (x, i) in r.iter().enumerate() {
            let bit = x * bits;
            if let Some(b) = line.get_mut(bit / 8) {
                *b |= i << (8 - bits - bit % 8);
            }
        }
        packed.extend(line);
    }
    let mut d = new_dict(old, w, h);
    let base = old.get(b"ColorSpace")?.clone();
    let hival = i64::try_from(count.checked_sub(1)?).ok()?;
    d.set(
        b"ColorSpace".to_vec(),
        Object::Array(vec![Object::name("Indexed"), base, Object::Int(hival), Object::String(pdfcraft_cos::PdfString::literal(entries))]),
    );
    d.set(b"BitsPerComponent".to_vec(), Object::Int(bits as i64));
    let params = pdfcraft_filters::Params { predictor: 15, colors: 1, bits_per_component: bits as i64, columns: i64::from(w), ..Default::default() };
    Some(crate::recompress::flate_best(d, &packed, Some(&params)))
}

/// What [`encode`] would do to an image, decided without touching its pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
    /// Colour components (1 or 3).
    pub n: usize,
    pub w: u32,
    pub h: u32,
    /// The size after resampling (`w`, `h` when not resampled).
    pub nw: u32,
    pub nh: u32,
    pub was_dct: bool,
}

impl Plan {
    pub(crate) fn resized(&self) -> bool {
        (self.nw, self.nh) != (self.w, self.h)
    }
}

/// Whether and how `settings` change image `s` drawn at `ppi`: the checks that need no pixels.
pub(crate) fn plan(doc: &Document, s: &Stream, ppi: f64, settings: &ImageSettings, n: usize) -> Option<Plan> {
    let d = &s.dict;
    if d.get(b"Decode").is_some_and(|a| !is_identity_decode(doc, a, n))
        || d.contains(b"Mask")
        || d.name(b"ImageMask").is_some()
        || matches!(d.get(b"ImageMask"), Some(Object::Bool(true)))
    {
        return None;
    }
    if d.int(b"BitsPerComponent") != Some(8) {
        return None;
    }
    let was_dct = is_dct(s)?;
    let (w, h) = (u32::try_from(d.int(b"Width")?).ok()?, u32::try_from(d.int(b"Height")?).ok()?);
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    // Tiny images (icons, rules) gain nothing, and JPEG or resampling would blur them.
    let tiny = u64::from(w) * u64::from(h) < 64 * 64;
    let resize = settings.downsample && ppi.is_finite() && ppi > settings.above_ppi && settings.target_ppi > 0.0;
    let recompress = match settings.compression {
        Compression::Jpeg(_) | Compression::Targeted(_) => true,
        Compression::Flate => was_dct,
        // JPEGs are transcoded losslessly by the stream pass.
        Compression::Lossless => !was_dct,
        Compression::Retain => false,
    };
    let targeted = matches!(settings.compression, Compression::Targeted(_));
    if (!resize && (!recompress || tiny)) || (targeted && tiny) {
        return None;
    }
    let (nw, nh) = if resize {
        // `ppi > above_ppi >= 0` and `target_ppi > 0`, so `k` is finite and positive; the
        // casts saturate.
        let k = settings.target_ppi / ppi;
        (((f64::from(w) * k).round() as u32).clamp(1, w), ((f64::from(h) * k).round() as u32).clamp(1, h))
    } else {
        (w, h)
    };
    Some(Plan { n, w, h, nw, nh, was_dct })
}

/// A JPEG re-encoded without resampling must save at least this share of its bytes (fixed
/// quality settings): a second generation of JPEG loss for a few percent isn't worth it.
const REENCODE_MIN_SAVING: f64 = 0.1;

/// The share of pixels equal to their left neighbour from which an image counts as a graphic
/// (screenshot, chart, line art, text) and stays lossless.
const GRAPHIC_FLATNESS: f64 = 0.5;

/// The smaller of `px` stored with Flate and as an indexed palette (lossless both).
fn lossless(old: &Dict, px: &[u8], n: usize, w: u32, h: u32) -> Option<Stream> {
    let flate = image_stream(old, px, n, w, h, Compression::Flate, false)?;
    match palette_stream(old, px, n, w, h) {
        Some(p) if p.raw.len() < flate.raw.len() => Some(p),
        _ => Some(flate),
    }
}

/// Whether a JPEG image's dictionary asks for the usual YCbCr to RGB transform (or nothing
/// that says otherwise).
fn colour_transform(d: &Dict) -> bool {
    !matches!(d.get(b"DecodeParms"), Some(Object::Dict(p)) if p.int(b"ColorTransform") == Some(0))
}

/// Steps of [`requantized`], gentlest first: (multiplier of chroma AC steps, of luma steps for
/// the lowest AC coefficients, of luma steps above zigzag 10).
const REQUANTIZE: [(u16, u16, u16); 5] = [(2, 1, 1), (2, 1, 2), (2, 2, 2), (3, 2, 3), (3, 3, 3)];

/// A JPEG `raw` that isn't resampled, made smaller on its own coefficients, so it loses no
/// more than that (`px` are its decoded samples): its luma alone when it is grey to the eye,
/// then its quantization steps multiplied (up to [`Target::requantize`]) as far as the SSIM
/// target allows. Returns the JPEG and whether it became grey.
fn requantized(d: &Dict, raw: &[u8], px: &[u8], n: usize, w: u32, h: u32, target: &crate::Target) -> Option<(Vec<u8>, bool)> {
    let j = crate::jpeg::read(raw)?;
    let rgb = matches!(d.name(b"ColorSpace"), Some(b"DeviceRGB" | b"RGB"));
    let grey = target.grey && n == 3 && rgb && colour_transform(d) && crate::jpeg::is_ycbcr(&j) && crate::quality::is_grey(px);
    let j = if grey { crate::jpeg::luma_only(&j)? } else { j };
    let reference = crate::quality::luma(px, n);
    let min = f64::from(target.min_ssim) / 10_000.0;
    let (wu, hu) = (usize::try_from(w).ok()?, usize::try_from(h).ok()?);
    let check = |j: &crate::jpeg::Jpeg| -> Option<Vec<u8>> {
        let data = crate::jpeg::write(j)?;
        let l = crate::quality::decoded_luma(&data, if grey { 1 } else { n })?;
        (crate::quality::ssim(&reference, &l, wu, hu) >= min).then_some(data)
    };
    let mut best = if grey { Some(check(&j)?) } else { None };
    for (c, lo, hi) in REQUANTIZE {
        if c.max(lo).max(hi) > u16::from(target.requantize) {
            break;
        }
        let factor = |chroma: bool, k: usize| {
            if chroma {
                c
            } else if k < 10 {
                lo
            } else {
                hi
            }
        };
        match crate::jpeg::requantize(&j, &factor).as_ref().and_then(check) {
            Some(data) => best = Some(data),
            None => break,
        }
    }
    best.map(|data| (data, grey))
}

/// Carry out `plan` on image `s`. Returns the new image and its new soft mask, if the result is
/// worth it (see [`Compression`]).
pub(crate) fn encode(doc: &Document, s: &Stream, plan: &Plan, settings: &ImageSettings) -> Option<(Stream, Option<(ObjRef, Stream)>)> {
    let Plan { n, w, h, was_dct, .. } = *plan;
    let d = &s.dict;
    let px = pixels(s, n)?;
    // Graphics (recognised on the original samples; a JPEG is a photo or scan, or was one when
    // it was made lossy) keep their resolution: resampling would blur text and lines and blend
    // their few colours into many, which a palette can't hold.
    let graphic = matches!(settings.compression, Compression::Targeted(_))
        && !was_dct
        && (crate::quality::palette(&px, n, if n == 1 { 16 } else { 256 }).is_some()
            || crate::quality::flatness(&px, n, w as usize) >= GRAPHIC_FLATNESS);
    let plan = &Plan { nw: if graphic { w } else { plan.nw }, nh: if graphic { h } else { plan.nh }, ..*plan };
    let (nw, nh) = (plan.nw, plan.nh);
    let px = if plan.resized() { resample(&px, n, w, h, nw, nh, Filter::Photo)? } else { px };
    let new = match settings.compression {
        Compression::Lossless => {
            if plan.resized() {
                lossless(d, &px, n, nw, nh)?
            } else {
                // Only a palette is new here: plain Flate is the stream pass's job.
                let p = palette_stream(d, &px, n, nw, nh)?;
                let flate = image_stream(d, &px, n, nw, nh, Compression::Flate, false)?;
                if p.raw.len() >= flate.raw.len() {
                    return None;
                }
                p
            }
        }
        Compression::Targeted(target) if was_dct && !plan.resized() && crate::jpeg::read(&s.raw).is_some() => {
            let (data, grey) = requantized(d, &s.raw, &px, n, w, h, &target)?;
            let mut dict = d.clone();
            dict.remove(b"Length");
            if grey {
                dict.set(b"ColorSpace".to_vec(), Object::name("DeviceGray"));
                dict.remove(b"DecodeParms");
            }
            Stream::from_raw(dict, data)
        }
        Compression::Targeted(target) => {
            // A colour image that is grey to the eye is stored as grey (DeviceRGB only: other
            // RGB spaces would need their own grey axis).
            let rgb = matches!(d.name(b"ColorSpace"), Some(b"DeviceRGB" | b"RGB"));
            let (px, n, dict) = if target.grey && n == 3 && rgb && crate::quality::is_grey(&px) {
                let mut dict = d.clone();
                dict.set(b"ColorSpace".to_vec(), Object::name("DeviceGray"));
                (crate::quality::luma(&px, 3), 1, dict)
            } else {
                (px, n, d.clone())
            };
            if graphic {
                lossless(&dict, &px, n, nw, nh)?
            } else {
                let sub = n == 3 && crate::quality::chroma_halves_well(&px, nw as usize, nh as usize);
                match crate::quality::search(&px, n, nw, nh, &target, sub) {
                    Some((data, _, _)) => jpeg_stream(&dict, data, nw, nh),
                    // Not even the ceiling reaches the target: grain (sensor noise, paper) that
                    // SSIM counts as detail. A JPEG is kept as it is; anything else is stored at
                    // the ceiling quality, which looks the same.
                    None if was_dct && !plan.resized() => return None,
                    None => jpeg_stream(&dict, crate::jpeg::encode(&px, n, nw, nh, target.ceiling, sub)?, nw, nh),
                }
            }
        }
        _ => {
            let mut new = image_stream(d, &px, n, nw, nh, settings.compression, was_dct)?;
            if !was_dct && new.dict.name(b"Filter") == Some(b"DCTDecode") {
                let flate = image_stream(d, &px, n, nw, nh, Compression::Flate, false)?;
                if flate.raw.len() <= new.raw.len() {
                    new = flate;
                }
            }
            new
        }
    };
    // A soft mask must keep the image's dimensions: resample it with the image (lossless).
    let mut smask = None;
    if let Some(mr) = d.reference(b"SMask") {
        let mobj = doc.get(mr);
        let Object::Stream(m) = &*mobj else { return None };
        if plan.resized() {
            if m.dict.int(b"BitsPerComponent") != Some(8) || m.dict.contains(b"Matte") || m.dict.contains(b"Decode") {
                return None;
            }
            let (mw, mh) = (u32::try_from(m.dict.int(b"Width")?).ok()?, u32::try_from(m.dict.int(b"Height")?).ok()?);
            if mw == 0 || mh == 0 || u64::from(mw) * u64::from(mh) > MAX_PIXELS {
                return None;
            }
            let mpx = pixels(m, 1)?;
            let mpx = resample(&mpx, 1, mw, mh, nw, nh, Filter::Hard)?;
            smask = Some((mr, image_stream(&m.dict, &mpx, 1, nw, nh, Compression::Flate, false)?));
        }
    }
    let before = s.raw.len()
        + smask.as_ref().map_or(0, |(r, _)| match &*doc.get(*r) {
            Object::Stream(m) => m.raw.len(),
            _ => 0,
        });
    let after = new.raw.len() + smask.as_ref().map_or(0, |(_, m)| m.raw.len());
    let worth = if !plan.resized() && was_dct {
        match settings.compression {
            // Against the JPEG as the lossless transcode will store it.
            Compression::Targeted(t) => {
                let base = crate::jpeg::transcode_unchecked(&s.raw).map_or(before, |j| j.len());
                (after as f64) <= (base as f64) * (1.0 - f64::from(t.min_saving.min(100)) / 100.0)
            }
            _ => (after as f64) <= (before as f64) * (1.0 - REENCODE_MIN_SAVING),
        }
    } else {
        after < before
    };
    worth.then_some((new, smask))
}

/// One image: resample and/or recompress per `settings`; `ppi` is its effective resolution.
/// Returns the new image and its new soft mask, if the result is smaller.
fn process(doc: &Document, s: &Stream, ppi: f64, settings: &ImageSettings, n: usize) -> Option<(Stream, Option<(ObjRef, Stream)>)> {
    encode(doc, s, &plan(doc, s, ppi, settings, n)?, settings)
}

/// The images `pages` draw that the image settings may change, in object order, with their
/// effective resolution (soft masks are processed with their image, not on their own).
pub(crate) fn drawn(doc: &Document, pages: &[ObjRef]) -> Vec<(ObjRef, f64)> {
    let ppi = effective_resolutions(doc, pages);
    let masks: HashSet<ObjRef> = ppi
        .keys()
        .filter_map(|r| match &*doc.get(*r) {
            Object::Stream(s) => s.dict.reference(b"SMask"),
            _ => None,
        })
        .collect();
    let mut refs: Vec<(ObjRef, f64)> = ppi.into_iter().filter(|(r, _)| !masks.contains(r)).collect();
    refs.sort_by_key(|(r, _)| r.num);
    refs
}

pub(crate) fn run(
    doc: &mut Document,
    pages: &[ObjRef],
    settings: &Settings,
    report: &mut Report,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<(), OptimizeError> {
    let refs = drawn(doc, pages);
    let total = refs.len();
    for (done, (r, ppi)) in refs.into_iter().enumerate() {
        if !progress(Stage::Images { done, total }) {
            return Err(OptimizeError::Cancelled);
        }
        let obj = doc.get(r);
        let Object::Stream(s) = &*obj else { continue };
        report.images += 1;
        let Some(n) = s.dict.get(b"ColorSpace").and_then(|cs| components(doc, cs)) else { continue };
        let st = if n == 1 { &settings.gray } else { &settings.color };
        let Some((new, smask)) = process(doc, s, ppi, st, n) else { continue };
        let resized = new.dict.int(b"Width") != s.dict.int(b"Width") || new.dict.int(b"Height") != s.dict.int(b"Height");
        report.image_bytes_before += s.raw.len();
        report.image_bytes_after += new.raw.len();
        if resized {
            report.images_resampled += 1;
        } else {
            report.images_recompressed += 1;
        }
        doc.set(r, Object::Stream(new));
        if let Some((mr, m)) = smask {
            doc.set(mr, Object::Stream(m));
        }
    }
    if !progress(Stage::Images { done: total, total }) {
        return Err(OptimizeError::Cancelled);
    }
    Ok(())
}
