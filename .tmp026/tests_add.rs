
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
    let cs = doc.resolve(s.dict.get(b"ColorSpace").unwrap()).into_owned();
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
/// 0.97, against the original resampled the same way when they resample), and the estimates
/// are close.
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
                    assert!(q >= target, "{name} {ppi} {}: SSIM {q:.4} below {target}", l.name());
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
