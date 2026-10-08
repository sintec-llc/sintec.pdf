//! Pictures the interface prepares before they go into a PDF: a signature from a photo or scan
//! of it, and small previews of images.

use std::io::Cursor;

use image::{ImageFormat, ImageReader, RgbaImage};

use crate::CreateError;

/// The most pixels a picture is read with (a 40-megapixel photo).
const MAX_PIXELS: u64 = 40_000_000;
/// A prepared signature is at most this many pixels on its longer side.
const SIGNATURE_SIDE: u32 = 1000;

fn read(name: &str, bytes: &[u8]) -> Result<RgbaImage, CreateError> {
    let bad = |m: String| CreateError::Image(name.into(), m);
    let format = image::guess_format(bytes).map_err(|e| bad(e.to_string()))?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::Bmp) {
        return Err(bad("use a PNG, JPEG, GIF or BMP image".into()));
    }
    let (w, h) = ImageReader::with_format(Cursor::new(bytes), format).into_dimensions().map_err(|e| bad(e.to_string()))?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(bad(format!("the picture is {w}×{h} pixels; at most {} megapixels", MAX_PIXELS / 1_000_000)));
    }
    let img = ImageReader::with_format(Cursor::new(bytes), format).decode().map_err(|e| bad(e.to_string()))?;
    Ok(img.to_rgba8())
}

fn luminance(p: &[u8]) -> u32 {
    (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000
}

/// A picture of a signature (a photo or scan of it on paper, or a PNG with transparency) → a
/// PNG ready to place: the paper made transparent (a picture that already has transparency
/// keeps it), trimmed to the ink, at most 1000 pixels on its longer side. Fails when nothing
/// darker than the paper is found.
pub fn signature_png(name: &str, bytes: &[u8]) -> Result<Vec<u8>, CreateError> {
    let bad = |m: &str| CreateError::Image(name.into(), m.into());
    let mut img = read(name, bytes)?;
    let has_alpha = img.pixels().any(|p| p.0[3] < 250);
    if !has_alpha {
        // The paper's tone: the 90th percentile of brightness (the brightest few are glare).
        let mut histogram = [0u64; 256];
        for p in img.pixels() {
            histogram[luminance(&p.0).min(255) as usize] += 1;
        }
        let total: u64 = histogram.iter().sum();
        let mut seen = 0;
        let paper = histogram
            .iter()
            .enumerate()
            .find(|(_, n)| {
                seen += **n;
                seen * 10 >= total * 9
            })
            .map_or(255, |(l, _)| l as i32);
        // Ink keeps its colour; the paper and its noise fade out.
        for p in img.pixels_mut() {
            let l = luminance(&p.0) as i32;
            p.0[3] = ((paper - l - 12) * 4).clamp(0, 255) as u8;
        }
    }
    // Trim to the ink, with a small margin.
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 24 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 || y0 > y1 {
        return Err(bad("no signature found in the picture: use dark ink on light paper"));
    }
    let margin = ((x1 - x0).max(y1 - y0) / 50).max(2);
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = ((x1 + margin).min(w - 1), (y1 + margin).min(h - 1));
    let mut img = image::imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    let (cw, ch) = img.dimensions();
    let side = cw.max(ch);
    if side > SIGNATURE_SIDE {
        let k = f64::from(SIGNATURE_SIDE) / f64::from(side);
        let (nw, nh) = (((f64::from(cw) * k).round() as u32).max(1), ((f64::from(ch) * k).round() as u32).max(1));
        img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    }
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| bad(&e.to_string()))?;
    Ok(out)
}

/// A picture as RGBA pixels at most `max_side` pixels on its longer side (previews).
pub fn preview_rgba(bytes: &[u8], max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let img = read("", bytes).ok()?;
    let (w, h) = img.dimensions();
    let side = w.max(h);
    let img = if side > max_side.max(1) {
        let k = f64::from(max_side.max(1)) / f64::from(side);
        image::imageops::resize(&img, ((f64::from(w) * k) as u32).max(1), ((f64::from(h) * k) as u32).max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(img: &RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn a_scan_on_paper_becomes_ink_on_transparency_trimmed_to_it() {
        // Greyish paper with a little noise, and a dark blue stroke in the middle.
        let mut img = RgbaImage::from_fn(400, 200, |x, y| image::Rgba([235 + ((x + y) % 7) as u8, 233, 228, 255]));
        for x in 100..300 {
            for y in 95..105 {
                img.put_pixel(x, y, image::Rgba([20, 30, 120, 255]));
            }
        }
        let out = signature_png("scan.jpg", &png(&img)).unwrap();
        let back = image::load_from_memory(&out).unwrap().to_rgba8();
        // Trimmed to the stroke (plus a few pixels).
        assert!(back.width() < 220 && back.height() < 30, "{}×{}", back.width(), back.height());
        let centre = back.get_pixel(back.width() / 2, back.height() / 2).0;
        assert_eq!(centre[3], 255, "ink stays opaque");
        assert!(centre[2] > centre[0], "and keeps its colour");
        assert_eq!(back.get_pixel(0, 0).0[3], 0, "paper is transparent");
    }

    #[test]
    fn pictures_with_transparency_keep_it_and_large_ones_are_scaled() {
        let mut img = RgbaImage::new(3000, 500);
        for x in 10..2990 {
            img.put_pixel(x, 250, image::Rgba([0, 0, 0, 200]));
        }
        let out = signature_png("sig.png", &png(&img)).unwrap();
        let back = image::load_from_memory(&out).unwrap().to_rgba8();
        assert!(back.width() <= SIGNATURE_SIDE);
        assert!(back.pixels().any(|p| p.0[3] == 200 || p.0[3] > 0));
    }

    #[test]
    fn blank_paper_and_non_images_are_refused() {
        let blank = RgbaImage::from_pixel(50, 50, image::Rgba([250, 250, 250, 255]));
        assert!(signature_png("blank.png", &png(&blank)).is_err());
        assert!(signature_png("x.pdf", b"%PDF-1.7 nonsense").is_err());
        assert!(preview_rgba(b"not an image", 100).is_none());
        let (w, h, px) = preview_rgba(&png(&RgbaImage::new(800, 400)), 200).unwrap();
        assert_eq!((w, h, px.len()), (200, 100, 200 * 100 * 4));
    }
}
