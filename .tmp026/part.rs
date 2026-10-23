/// How samples are resampled: Lanczos (3 lobes) for photos and scans, a tent (area-like, no
/// ringing) for graphics and soft masks, whose hard edges would get halos.
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

/// Carry out `plan` on image `s`. Returns the new image and its new soft mask, if the result is
/// worth it (see [`Compression`]).
pub(crate) fn encode(doc: &Document, s: &Stream, plan: &Plan, settings: &ImageSettings) -> Option<(Stream, Option<(ObjRef, Stream)>)> {
    let Plan { n, w, h, nw, nh, was_dct } = *plan;
    let d = &s.dict;
    let px = pixels(s, n)?;
    // Graphics are recognised on the original samples: resampling blends their colours. A JPEG
    // is a photo or scan (or was one when it was made lossy).
    let graphic = matches!(settings.compression, Compression::Targeted(_))
        && !was_dct
        && (crate::quality::palette(&px, n, 256).is_some() || crate::quality::flatness(&px, n, w as usize) >= GRAPHIC_FLATNESS);
    let filter = if graphic { Filter::Hard } else { Filter::Photo };
    let px = if plan.resized() { resample(&px, n, w, h, nw, nh, filter)? } else { px };
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
                    // No quality is good enough: keep a JPEG as it is, store the rest losslessly.
                    None if was_dct && !plan.resized() => return None,
                    None => lossless(&dict, &px, n, nw, nh)?,
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

