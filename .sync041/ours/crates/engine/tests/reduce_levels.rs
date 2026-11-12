//! Reduce File Size's levels on documents shaped like real ones: photos, scans, text with
//! duplicated fonts and thumbnails, uncompressed streams. Each level must make the file smaller
//! than the one below it, keep what it promises to keep, and the estimate shown before must be
//! close to the real result.
//!
//! `cargo test -p pdfcraft-engine --test reduce_levels -- --nocapture` prints the measurements.

use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, SaveOptions, Stream, serialize, write_full};
use pdfcraft_engine::Session;
use pdfcraft_engine::optimize::Budget;
use pdfcraft_engine::optimizer::Level;

/// The estimate must be within this share of the real result (or 2 KB, for tiny files) when the
/// document fits the default budget, and within the looser share when only a sample is tried.
const TOLERANCE: f64 = 0.05;
const SAMPLED_TOLERANCE: f64 = 0.25;

/// A deterministic noise source (xorshift).
struct Noise(u64);
impl Noise {
    fn next(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 24) as u8
    }
}

/// Photo-like samples: smooth gradients and shapes with sensor noise.
fn photo(w: u32, h: u32, n: usize, seed: u64) -> Vec<u8> {
    let mut noise = Noise(seed | 1);
    let mut px = Vec::with_capacity((w * h) as usize * n);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let blob = (-((fx - 0.4).powi(2) + (fy - 0.5).powi(2)) * 12.0).exp();
            for c in 0..n {
                let base = 40.0 + 150.0 * blob + 50.0 * ((fx * 6.0 + c as f32).sin() * (fy * 4.0).cos());
                let v = base + (noise.next() % 12) as f32 - 6.0;
                px.push(v.clamp(0.0, 255.0) as u8);
            }
        }
    }
    px
}

/// A scanned page: white paper with a little grain, lines of dark "words".
fn scan(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut noise = Noise(seed | 1);
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let line = (y / 40) % 2 == 0 && y > 200 && y < h - 200;
        for x in 0..w {
            let word = line && (x / 90) % 5 != 0 && x > 150 && x < w - 150 && (y % 40) > 8 && (y % 40) < 30 && (x % 9) < 6;
            let v: i32 = if word { 30 } else { 235 };
            px.push((v + (noise.next() % 16) as i32 - 8).clamp(0, 255) as u8);
        }
    }
    px
}

fn jpeg(px: &[u8], w: u32, h: u32, n: usize, q: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let ty = if n == 1 { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q).encode(px, w, h, ty).unwrap();
    out
}

enum Store {
    Jpeg(u8),
    Flate,
    /// Uncompressed, as some scanners and converters write it.
    Raw,
}

fn add_image(doc: &mut Document, px: &[u8], w: u32, h: u32, n: usize, bpc: i64, store: Store) -> ObjRef {
    let mut d = Dict::new();
    d.set(b"Type".to_vec(), Object::name("XObject"));
    d.set(b"Subtype".to_vec(), Object::name("Image"));
    d.set(b"Width".to_vec(), Object::Int(w as i64));
    d.set(b"Height".to_vec(), Object::Int(h as i64));
    d.set(b"BitsPerComponent".to_vec(), Object::Int(bpc));
    d.set(b"ColorSpace".to_vec(), Object::name(if n == 1 { "DeviceGray" } else { "DeviceRGB" }));
    let s = match store {
        Store::Jpeg(q) => {
            d.set(b"Filter".to_vec(), Object::name("DCTDecode"));
            Stream::from_raw(d, jpeg(px, w, h, n, q))
        }
        Store::Flate => Stream::flate(d, px),
        Store::Raw => Stream::from_raw(d, px.to_vec()),
    };
    doc.add(Object::Stream(s))
}

/// A page drawing `images` (name, object, placement `[w h x y]` in points) and `text`, with
/// `fonts` (name, font object) in its resources. Content is stored without a filter.
fn add_page(doc: &mut Document, images: &[(&str, ObjRef, [f64; 4])], text: &str, fonts: &[(&str, ObjRef)], thumb: Option<ObjRef>) -> ObjRef {
    let pages = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|d| d.reference(b"Pages"))).unwrap();
    let mut x = Dict::new();
    let mut content = String::new();
    for (name, r, [w, h, px, py]) in images {
        x.set(name.as_bytes().to_vec(), Object::Ref(*r));
        content.push_str(&format!("q {w} 0 0 {h} {px} {py} cm /{name} Do Q\n"));
    }
    content.push_str(text);
    let mut f = Dict::new();
    for (name, r) in fonts {
        f.set(name.as_bytes().to_vec(), Object::Ref(*r));
    }
    let mut res = Dict::new();
    res.set(b"XObject".to_vec(), Object::Dict(x));
    res.set(b"Font".to_vec(), Object::Dict(f));
    let c = doc.add(Object::Stream(Stream::from_raw(Dict::new(), content.into_bytes())));
    let mut p = Dict::new();
    p.set(b"Type".to_vec(), Object::name("Page"));
    p.set(b"Parent".to_vec(), Object::Ref(pages));
    p.set(b"MediaBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
    p.set(b"Resources".to_vec(), Object::Dict(res));
    p.set(b"Contents".to_vec(), Object::Ref(c));
    if let Some(t) = thumb {
        p.set(b"Thumb".to_vec(), Object::Ref(t));
    }
    let r = doc.add(Object::Dict(p));
    doc.update_dict(pages, |d| {
        let mut kids = d.get(b"Kids").and_then(|k| k.as_array().cloned()).unwrap_or_default();
        kids.push(Object::Ref(r));
        d.set(b"Count".to_vec(), Object::Int(kids.len() as i64));
        d.set(b"Kids".to_vec(), Object::Array(kids));
    })
    .unwrap();
    r
}

/// An embedded TrueType-like font program (its bytes only need to be stable, not drawable:
/// the text uses the standard Helvetica metrics) and its font dictionary.
fn add_font(doc: &mut Document, seed: u64) -> ObjRef {
    let mut noise = Noise(seed | 1);
    // Half table-like structure, half glyph noise: compresses about as well as real fonts.
    let program: Vec<u8> = (0..60_000).map(|i| if i % 4 < 2 { (i / 7) as u8 } else { noise.next() }).collect();
    let mut fd = Dict::new();
    fd.set(b"Length1".to_vec(), Object::Int(program.len() as i64));
    let file = doc.add(Object::Stream(Stream::flate(fd, &program)));
    let mut desc = Dict::new();
    desc.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    desc.set(b"FontName".to_vec(), Object::name("Helvetica"));
    desc.set(b"Flags".to_vec(), Object::Int(32));
    desc.set(b"FontFile2".to_vec(), Object::Ref(file));
    let desc = doc.add(Object::Dict(desc));
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("TrueType"));
    font.set(b"BaseFont".to_vec(), Object::name("Helvetica"));
    font.set(b"FontDescriptor".to_vec(), Object::Ref(desc));
    doc.add(Object::Dict(font))
}

fn text_block(page: usize, lines: usize) -> String {
    let mut s = String::from("BT /F1 10 Tf 72 740 Td 12 TL\n");
    for i in 0..lines {
        s.push_str(&format!("(Page {page}, line {i}: the quarterly figures are summarised in the table below.) '\n"));
    }
    s.push_str("ET\n");
    s
}

fn bytes(doc: &Document) -> Vec<u8> {
    write_full(doc, &SaveOptions { object_streams: false, ..SaveOptions::default() }).unwrap()
}

/// Three photos at 300 ppi: two high-quality JPEGs and one stored losslessly.
fn photos() -> Vec<u8> {
    let mut doc = Document::new_empty();
    for (i, store) in [Store::Jpeg(92), Store::Jpeg(92), Store::Flate].into_iter().enumerate() {
        let (w, h) = (1500, 1000);
        let img = add_image(&mut doc, &photo(w, h, 3, 7 + i as u64), w, h, 3, 8, store);
        add_page(&mut doc, &[("Im1", img, [360.0, 240.0, 126.0, 400.0])], "", &[], None);
    }
    bytes(&doc)
}

/// Two grey scans at 300 ppi stored with Flate, and a 1-bit scan stored uncompressed.
fn scans() -> Vec<u8> {
    let mut doc = Document::new_empty();
    let (w, h) = (1275, 1650);
    for i in 0..2 {
        let img = add_image(&mut doc, &scan(w, h, 3 + i), w, h, 1, 8, Store::Flate);
        add_page(&mut doc, &[("Im1", img, [306.0, 396.0, 0.0, 0.0])], "", &[], None);
    }
    let row = (w as usize).div_ceil(8);
    let bits: Vec<u8> = scan(w, h, 11)
        .chunks(w as usize)
        .flat_map(|r| (0..row).map(move |b| (0..8).fold(0u8, |acc, k| acc << 1 | u8::from(r.get(b * 8 + k).is_none_or(|v| *v > 128)))))
        .collect();
    let img = add_image(&mut doc, &bits, w, h, 1, 1, Store::Raw);
    add_page(&mut doc, &[("Im1", img, [306.0, 396.0, 0.0, 0.0])], "", &[], None);
    bytes(&doc)
}

/// Twelve pages of text, uncompressed, each with its own copy of the same embedded font and a
/// thumbnail.
fn text() -> Vec<u8> {
    let mut doc = Document::new_empty();
    for p in 0..12 {
        let font = add_font(&mut doc, 99);
        let tw = 76u32;
        let thumb = add_image(&mut doc, &photo(tw, 98, 3, p as u64), tw, 98, 3, 8, Store::Raw);
        add_page(&mut doc, &[], &text_block(p, 55), &[("F1", font)], Some(thumb));
    }
    bytes(&doc)
}

/// A report: text pages, a chart stored losslessly, a photo used on two pages (as two copies).
fn mixed() -> Vec<u8> {
    let mut doc = Document::new_empty();
    let font = add_font(&mut doc, 5);
    let (w, h) = (1200, 800);
    let px = photo(w, h, 3, 21);
    let a = add_image(&mut doc, &px, w, h, 3, 8, Store::Jpeg(90));
    let b = add_image(&mut doc, &px, w, h, 3, 8, Store::Jpeg(90));
    // A flat-colour bar chart: lossless stays smaller than JPEG.
    let (cw, ch) = (900u32, 600u32);
    let chart: Vec<u8> = (0..ch)
        .flat_map(|y| (0..cw).flat_map(move |x| if (x / 100) % 2 == 0 && y > 600 - (x / 100 + 1) * 50 { [40, 90, 200] } else { [255, 255, 255] }))
        .collect();
    let c = add_image(&mut doc, &chart, cw, ch, 3, 8, Store::Raw);
    add_page(&mut doc, &[("P", a, [288.0, 192.0, 72.0, 500.0])], &text_block(0, 20), &[("F1", font)], None);
    add_page(&mut doc, &[("C", c, [432.0, 288.0, 90.0, 300.0])], &text_block(1, 20), &[("F1", font)], None);
    add_page(&mut doc, &[("P", b, [288.0, 192.0, 72.0, 500.0])], &text_block(2, 20), &[("F1", font)], None);
    bytes(&doc)
}

struct Measured {
    original: usize,
    real: Vec<(Level, usize)>,
    estimated: Vec<(Level, usize)>,
    sampled: Vec<(Level, usize)>,
}

fn measure(name: &str, pdf: Vec<u8>) -> (Session, pdfcraft_engine::DocId, Measured) {
    let mut s = Session::new().with_clock(|| 1_700_000_000);
    let id = s.open(format!("{name}.pdf"), None, Arc::new(pdf), None).unwrap();
    let original = s.get(id).unwrap().bytes.len();
    let real: Vec<(Level, usize)> = Level::ALL.iter().map(|l| (*l, s.reduced_bytes(id, *l).unwrap().0.len())).collect();
    let mut estimated = Vec::new();
    s.estimate_job(id).unwrap().run(|| false, |l, b| estimated.push((l, b))).unwrap();
    // A budget too small for anything but one trial per kind: the model does the rest.
    let mut sampled = Vec::new();
    s.estimate_job(id).unwrap().with_budget(Budget { image_pixels: 0, stream_bytes: 0 }).run(|| false, |l, b| sampled.push((l, b))).unwrap();
    let m = Measured { original, real, estimated, sampled };
    // The Reduce File Size this replaces: Acrobat's defaults, no lossless recompression.
    let old_settings = pdfcraft_engine::optimize::Settings { recompress_streams: false, ..Default::default() };
    let old = s.optimized_bytes(id, &old_settings, &[]).unwrap().0.len();
    println!("{name}: original {}, previous Reduce File Size {old} ({:.1}% saved)", m.original, 100.0 * (1.0 - old as f64 / m.original as f64));
    for ((l, r), ((_, e), (_, sm))) in m.real.iter().zip(m.estimated.iter().zip(&m.sampled)) {
        let pct = |x: usize| 100.0 * (1.0 - x as f64 / m.original as f64);
        let err = |x: usize| 100.0 * (x as f64 - *r as f64) / *r as f64;
        println!(
            "  {:<6} real {r:>9} ({:>5.1}% saved)  estimate {e:>9} ({:>+5.1}%)  sampled estimate {sm:>9} ({:>+5.1}%)",
            l.name(),
            pct(*r),
            err(*e),
            err(*sm)
        );
    }
    (s, id, m)
}

fn size(v: &[(Level, usize)], l: Level) -> usize {
    v.iter().find(|(x, _)| *x == l).map(|(_, b)| *b).unwrap()
}

fn assert_close(what: &str, estimate: usize, real: usize, tolerance: f64) {
    let off = (estimate as f64 - real as f64).abs();
    assert!(off <= (real as f64 * tolerance).max(2048.0), "{what}: estimate {estimate} vs real {real} (off by {:.1}%)", 100.0 * off / real as f64);
}

fn check_estimates(name: &str, m: &Measured) {
    for l in Level::ALL {
        assert_close(&format!("{name} {}", l.name()), size(&m.estimated, l), size(&m.real, l), TOLERANCE);
        assert_close(&format!("{name} {} (sampled)", l.name()), size(&m.sampled, l), size(&m.real, l), SAMPLED_TOLERANCE);
    }
}

/// Every object of a PDF as text: dictionaries serialized, streams decoded.
fn all_text(bytes: &Arc<Vec<u8>>) -> String {
    let doc = Document::open(bytes.clone()).unwrap();
    let mut out = Vec::new();
    for n in doc.object_numbers() {
        match &*doc.get(ObjRef::new(n, doc.generation(n))) {
            Object::Stream(s) => {
                serialize(&Object::Dict(s.dict.clone()), &mut out);
                out.extend(s.decoded().unwrap_or_default());
            }
            o => serialize(o, &mut out),
        }
        out.push(b'\n');
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn reopen(bytes: Arc<Vec<u8>>) -> (Session, pdfcraft_engine::DocId) {
    let mut s = Session::new();
    let id = s.open("r.pdf", None, bytes, None).unwrap();
    (s, id)
}

#[test]
fn photos_get_smaller_at_each_level_and_the_estimate_is_close() {
    let (_, _, m) = measure("photos", photos());
    let (light, medium, strong) = (size(&m.real, Level::Light), size(&m.real, Level::Medium), size(&m.real, Level::Strong));
    assert!(strong < medium && medium < light && light < m.original, "{:?} from {}", m.real, m.original);
    assert!(medium * 3 < m.original, "300 ppi photos at 150 ppi: {medium} from {}", m.original);
    check_estimates("photos", &m);
}

#[test]
fn scans_get_smaller_at_each_level_and_the_one_bit_scan_stays_lossless() {
    let (s, id, m) = measure("scans", scans());
    let (light, medium, strong) = (size(&m.real, Level::Light), size(&m.real, Level::Medium), size(&m.real, Level::Strong));
    assert!(strong < medium && medium < light && light < m.original, "{:?} from {}", m.real, m.original);
    check_estimates("scans", &m);
    // The 1-bit scan is never turned into JPEG: same size, one bit, the same samples.
    let (bytes, _) = s.reduced_bytes(id, Level::Strong).unwrap();
    let doc = Document::open(bytes.clone()).unwrap();
    let bits: Vec<Stream> = doc
        .object_numbers()
        .into_iter()
        .filter_map(|n| match &*doc.get(ObjRef::new(n, doc.generation(n))) {
            Object::Stream(s) if s.dict.int(b"BitsPerComponent") == Some(1) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(bits.len(), 1);
    assert_eq!((bits[0].dict.int(b"Width"), bits[0].dict.name(b"Filter")), (Some(1275), Some(&b"FlateDecode"[..])));
    assert_eq!(bits[0].decoded_strict().unwrap().len(), 1275usize.div_ceil(8) * 1650);
}

#[test]
fn text_is_kept_while_duplicate_fonts_thumbnails_and_plain_streams_go() {
    let (s, id, m) = measure("text", text());
    let light = size(&m.real, Level::Light);
    assert!(light * 4 < m.original, "duplicate fonts merged, content compressed: {light} from {}", m.original);
    for l in Level::ALL {
        assert!(size(&m.real, l) < m.original);
    }
    check_estimates("text", &m);
    for l in Level::ALL {
        let (bytes, merged) = s.reduced_bytes(id, l).unwrap();
        assert!(merged >= 11 * 3, "the 12 copies of the font become one: {merged}");
        let text = all_text(&bytes);
        for p in 0..12 {
            assert!(text.contains(&format!("(Page {p}, line 54: the quarterly")), "{}: page {p}'s text", l.name());
        }
        let (r, rid) = reopen(bytes);
        assert_eq!(r.get(rid).unwrap().info.pages.len(), 12);
    }
}

#[test]
fn a_mixed_report_gets_smaller_at_each_level_and_the_estimate_is_close() {
    let (_, _, m) = measure("mixed", mixed());
    let (light, medium, strong) = (size(&m.real, Level::Light), size(&m.real, Level::Medium), size(&m.real, Level::Strong));
    assert!(strong < medium && medium < light && light < m.original, "{:?} from {}", m.real, m.original);
    check_estimates("mixed", &m);
}

#[test]
fn light_and_medium_keep_bookmarks_links_and_metadata_strong_drops_metadata() {
    let mut doc = Document::new_empty();
    let (w, h) = (1200, 800);
    let img = add_image(&mut doc, &photo(w, h, 3, 4), w, h, 3, 8, Store::Jpeg(95));
    let page = add_page(&mut doc, &[("Im1", img, [288.0, 192.0, 72.0, 400.0])], "", &[], None);
    // A link to a named destination nothing else uses, a bookmark and document information.
    let mut dest = Dict::new();
    dest.set(b"D".to_vec(), Object::Array(vec![Object::Ref(page), Object::name("Fit")]));
    let mut names = Dict::new();
    names.set(b"Names".to_vec(), Object::Array(vec![Object::String(PdfString::literal(b"chapter".to_vec())), Object::Dict(dest)]));
    let names = doc.add(Object::Dict(names));
    let mut outline_item = Dict::new();
    outline_item.set(b"Title".to_vec(), Object::String(PdfString::literal(b"Start".to_vec())));
    outline_item.set(b"Dest".to_vec(), Object::Array(vec![Object::Ref(page), Object::name("Fit")]));
    let item = doc.add(Object::Dict(outline_item));
    let mut outlines = Dict::new();
    outlines.set(b"Type".to_vec(), Object::name("Outlines"));
    outlines.set(b"First".to_vec(), Object::Ref(item));
    outlines.set(b"Last".to_vec(), Object::Ref(item));
    outlines.set(b"Count".to_vec(), Object::Int(1));
    let outlines = doc.add(Object::Dict(outlines));
    doc.update_dict(item, |d| d.set(b"Parent".to_vec(), Object::Ref(outlines))).unwrap();
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| {
        c.set(b"Outlines".to_vec(), Object::Ref(outlines));
        let mut n = Dict::new();
        n.set(b"Dests".to_vec(), Object::Ref(names));
        c.set(b"Names".to_vec(), Object::Dict(n));
    })
    .unwrap();
    let mut info = Dict::new();
    info.set(b"Title".to_vec(), Object::String(PdfString::literal(b"Quarterly report".to_vec())));
    let info = doc.add(Object::Dict(info));
    doc.trailer_mut().set(b"Info".to_vec(), Object::Ref(info));
    let mut s = Session::new();
    let id = s.open("kept.pdf", None, Arc::new(bytes(&doc)), None).unwrap();
    for l in Level::ALL {
        let (out, _) = s.reduced_bytes(id, l).unwrap();
        let text = all_text(&out);
        let kept = l != Level::Strong;
        assert!(text.contains("(Start)"), "{}: the bookmark stays", l.name());
        assert_eq!(text.contains("(chapter)"), kept, "{}: the named destination", l.name());
        assert_eq!(text.contains("(Quarterly report)"), kept, "{}: the title", l.name());
    }
}

#[test]
fn an_estimate_stops_when_cancelled_and_reports_nothing() {
    let mut s = Session::new();
    let id = s.open("photos.pdf", None, Arc::new(photos()), None).unwrap();
    let mut found = Vec::new();
    let r = s.estimate_job(id).unwrap().run(|| true, |l, b| found.push((l, b)));
    assert!(matches!(r, Err(pdfcraft_engine::EditError::Cancelled)), "{r:?}");
    assert!(found.is_empty());
    // Cancelled once the first level is known: only that one is reported.
    let stop = std::cell::Cell::new(false);
    let mut found = Vec::new();
    let r = s.estimate_job(id).unwrap().run(
        || stop.get(),
        |l, b| {
            found.push((l, b));
            stop.set(true);
        },
    );
    assert!(matches!(r, Err(pdfcraft_engine::EditError::Cancelled)), "{r:?}");
    assert_eq!(found.iter().map(|(l, _)| *l).collect::<Vec<_>>(), [Level::Light]);
}

/// Mean SSIM of two luma planes over 8×8 windows every 4 pixels, counting only windows with
/// detail in `a` when at least 1 in 100 has some (as the optimizer measures it).
fn ssim(a: &[u8], b: &[u8], w: usize, h: usize) -> f64 {
    let (mut all, mut n_all, mut det, mut n_det) = (0.0, 0usize, 0.0, 0usize);
    for y0 in (0..=h - 8).step_by(4) {
        for x0 in (0..=w - 8).step_by(4) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for y in y0..y0 + 8 {
                for x in x0..x0 + 8 {
                    let (p, q) = (a[y * w + x] as f64, b[y * w + x] as f64);
                    (sa, sb, saa, sbb, sab) = (sa + p, sb + q, saa + p * p, sbb + q * q, sab + p * q);
                }
            }
            let (ma, mb) = (sa / 64.0, sb / 64.0);
            let (va, vb, cov) = ((saa / 64.0 - ma * ma).max(0.0), (sbb / 64.0 - mb * mb).max(0.0), sab / 64.0 - ma * mb);
            let s = ((2.0 * ma * mb + 6.5025) * (2.0 * cov + 58.5225)) / ((ma * ma + mb * mb + 6.5025) * (va + vb + 58.5225));
            (all, n_all) = (all + s, n_all + 1);
            if va >= 25.0 {
                (det, n_det) = (det + s, n_det + 1);
            }
        }
    }
    if n_det * 100 >= n_all && n_det > 0 { det / n_det as f64 } else { all / n_all as f64 }
}

fn luma(px: &[u8], n: usize) -> Vec<u8> {
    if n == 1 {
        return px.to_vec();
    }
    px.chunks(3).map(|p| ((77 * p[0] as u32 + 150 * p[1] as u32 + 29 * p[2] as u32 + 128) >> 8) as u8).collect()
}

/// The one image of a PDF, as drawn: size, components and 8-bit samples (an indexed palette
/// expanded, a JPEG decoded).
fn the_image(bytes: &Arc<Vec<u8>>) -> (u32, u32, usize, Vec<u8>) {
    let doc = Document::open(bytes.clone()).unwrap();
    let s = doc
        .object_numbers()
        .into_iter()
        .find_map(|n| match &*doc.get(ObjRef::new(n, doc.generation(n))) {
            Object::Stream(s) if s.dict.name(b"Subtype") == Some(b"Image") => Some(s.clone()),
            _ => None,
        })
        .unwrap();
    let (w, h) = (s.dict.int(b"Width").unwrap() as u32, s.dict.int(b"Height").unwrap() as u32);
    if s.dict.name(b"Filter") == Some(b"DCTDecode") {
        let img = image::load_from_memory(&s.raw).unwrap();
        return if img.color().channel_count() == 1 { (w, h, 1, img.into_luma8().into_raw()) } else { (w, h, 3, img.into_rgb8().into_raw()) };
    }
    let data = s.decoded_strict().unwrap();
    let cs = (*doc.resolve(s.dict.get(b"ColorSpace").unwrap())).clone();
    match cs {
        Object::Array(a) if a[0].as_name() == Some(b"Indexed") => {
            let n = if doc.resolve(&a[1]).as_name() == Some(b"DeviceGray") { 1 } else { 3 };
            let lookup = match &a[3] {
                Object::String(s) => s.bytes.clone(),
                _ => panic!("lookup"),
            };
            let bits = s.dict.int(b"BitsPerComponent").unwrap() as usize;
            let row = (w as usize * bits).div_ceil(8);
            let mut px = Vec::new();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let byte = data[y * row + x * bits / 8];
                    let i = (byte >> (8 - bits - (x * bits) % 8)) as usize & ((1 << bits) - 1);
                    px.extend_from_slice(&lookup[i * n..i * n + n]);
                }
            }
            (w, h, n, px)
        }
        Object::Name(n) if n == b"DeviceGray" => (w, h, 1, data),
        _ => (w, h, 3, data),
    }
}

/// A screenshot: flat panels, a title bar and dark "text" with a grey anti-aliasing column.
fn screenshot(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut c = if y < h / 10 {
                [40u8, 90, 160]
            } else if x < w / 4 {
                [235, 238, 242]
            } else {
                [255, 255, 255]
            };
            let (tx, ty) = (x % 14, y % 22);
            if y > h / 5 && x > w / 3 && x < w - 20 && ty < 12 && (tx == 2 || tx == 3 || ty == 5) && (x / 70) % 4 != 3 {
                c = if tx == 3 { [90, 90, 100] } else { [20, 20, 30] };
            }
            px.extend(c);
        }
    }
    px
}

/// Text on a photo: a photo background with black lettering strokes.
fn text_on_image(w: u32, h: u32) -> Vec<u8> {
    let mut px = photo(w, h, 3, 77);
    for y in 0..h {
        for x in 0..w {
            if (y / 30) % 3 == 1 && (x % 11 < 2 || (y % 30) < 3) && x > w / 8 && x < w * 7 / 8 {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&[10, 10, 10]);
            }
        }
    }
    px
}

/// A picture of each kind: samples, components and how it is stored.
fn kind(name: &str, w: u32, h: u32) -> (Vec<u8>, usize, Store) {
    match name {
        "photo" => (photo(w, h, 3, 5), 3, Store::Jpeg(92)),
        "scan" => (scan(w, h, 9), 1, Store::Jpeg(85)),
        "screenshot" => (screenshot(w, h), 3, Store::Flate),
        _ => (text_on_image(w, h), 3, Store::Jpeg(90)),
    }
}

/// Each kind of picture at 96, 150, 200 and 300 ppi: every level is no larger than the one
/// below it, Light keeps every pixel, Medium and Strong keep their SSIM targets (0.985 and
/// 0.97, against the original resampled the same way when they resample; grain that can't reach
/// them at the ceiling quality comes within 0.01), and the estimates are close.
#[test]
fn photos_scans_screenshots_and_text_on_images_at_four_resolutions() {
    let mut report = Vec::new();
    for name in ["photo", "scan", "screenshot", "text on image"] {
        for ppi in [96u32, 150, 200, 300] {
            // A 4 × 3 inch picture.
            let (w, h) = (4 * ppi, 3 * ppi);
            let (px, n, store) = kind(name, w, h);
            let mut doc = Document::new_empty();
            let img = add_image(&mut doc, &px, w, h, n, 8, store);
            add_page(&mut doc, &[("Im1", img, [288.0, 216.0, 100.0, 300.0])], "", &[], None);
            let original_bytes = Arc::new(bytes(&doc));
            // What the page drew before: the samples as decoded.
            let (_, _, on, opx) = the_image(&original_bytes);
            let (s, id, m) = measure(&format!("{name} {ppi} ppi"), (*original_bytes).clone());
            check_estimates(name, &m);
            let (light, medium, strong) = (size(&m.real, Level::Light), size(&m.real, Level::Medium), size(&m.real, Level::Strong));
            assert!(strong <= medium && medium <= light && light <= m.original, "{name} {ppi}: {:?} from {}", m.real, m.original);
            let mut line = format!("| {name} | {ppi} | {} |", m.original);
            for (l, target) in [(Level::Light, 1.0), (Level::Medium, 0.985), (Level::Strong, 0.97)] {
                let (out, _) = s.reduced_bytes(id, l).unwrap();
                let (nw, nh, nn, npx) = the_image(&out);
                let reference = if (nw, nh) == (w, h) {
                    luma(&opx, on)
                } else {
                    use image::imageops::{FilterType, resize};
                    let r = if on == 1 {
                        resize(&image::GrayImage::from_raw(w, h, opx.clone()).unwrap(), nw, nh, FilterType::Lanczos3).into_raw()
                    } else {
                        resize(&image::RgbImage::from_raw(w, h, opx.clone()).unwrap(), nw, nh, FilterType::Lanczos3).into_raw()
                    };
                    luma(&r, on)
                };
                let q = ssim(&reference, &luma(&npx, nn), nw as usize, nh as usize);
                if l == Level::Light {
                    assert_eq!((nw, nh, nn), (w, h, on), "{name} {ppi} light");
                    assert!(npx == opx, "{name} {ppi}: Light changed pixels");
                } else {
                    // Grain that misses the target even at the ceiling quality (92, 90) is stored
                    // at the ceiling: a little below the target, never far.
                    assert!(q >= target - 0.01, "{name} {ppi} {}: SSIM {q:.4} below {target}", l.name());
                }
                let pct = 100.0 * (1.0 - size(&m.real, l) as f64 / m.original as f64);
                line.push_str(&format!(" {} (−{pct:.0}%, {nw}×{nh}, SSIM {q:.4}) |", size(&m.real, l)));
            }
            report.push(line);
        }
    }
    println!("| Picture | ppi | Original | Light | Medium | Strong |");
    for l in report {
        println!("{l}");
    }
}

/// What users reported on task 024: a colour scan of black-and-white pages at 200 ppi, stored as
/// JPEG quality 75, came out the same at Light and Medium (nothing above 200 ppi to resample,
/// and re-encoding saved less than a tenth). Medium must now be clearly smaller than Light,
/// which is smaller than the original, and Strong smaller still.
#[test]
fn a_colour_scan_at_200_ppi_gets_smaller_at_medium_than_at_light() {
    let mut doc = Document::new_empty();
    let (w, h) = (1700u32, 2200u32);
    for i in 0..2 {
        let grey = scan(w, h, 21 + i);
        let rgb: Vec<u8> = grey.iter().flat_map(|v| [*v, *v, v.saturating_add(2)]).collect();
        let img = add_image(&mut doc, &rgb, w, h, 3, 8, Store::Jpeg(75));
        add_page(&mut doc, &[("Im1", img, [612.0, 792.0, 0.0, 0.0])], "", &[], None);
    }
    let (_, _, m) = measure("colour scan 200 ppi", bytes(&doc));
    let (light, medium, strong) = (size(&m.real, Level::Light), size(&m.real, Level::Medium), size(&m.real, Level::Strong));
    assert!(light < m.original, "{light} from {}", m.original);
    assert!(medium * 10 < light * 9, "Medium {medium} should be well below Light {light}");
    assert!(strong < medium, "{strong} vs {medium}");
    check_estimates("colour scan", &m);
}
