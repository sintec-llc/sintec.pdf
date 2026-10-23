//! How to store an image so it looks the same: what kind of picture it is (a photo or scan, or a
//! graphic with few colours and flat areas), whether its colour is grey or survives half-size
//! chroma, and the lowest JPEG quality whose result keeps its structural similarity (SSIM).

use std::collections::HashMap;

use crate::Target;

/// Luma (BT.601) of 8-bit grey (`n` = 1) or RGB (`n` = 3) samples.
pub(crate) fn luma(px: &[u8], n: usize) -> Vec<u8> {
    if n == 3 {
        px.as_chunks::<3>().0.iter().map(|[r, g, b]| ((77 * u32::from(*r) + 150 * u32::from(*g) + 29 * u32::from(*b) + 128) >> 8) as u8).collect()
    } else {
        px.to_vec()
    }
}

/// Stabilizing constants of SSIM for 8-bit samples: (0.01 · 255)² and (0.03 · 255)².
const C1: f64 = 6.5025;
const C2: f64 = 58.5225;

/// Windows of the reference whose variance is below this are flat (paper, sky): JPEG keeps them
/// whatever the quality, and counting them would hide what happens to the detail.
const FLAT_VARIANCE: f64 = 25.0;

/// The mean structural similarity of two `w` × `h` planes (Wang et al. 2004) over 8 × 8
/// windows every 4 samples (one window over all of a smaller image): 1 when they are the same.
/// Only windows with detail in `a` (the reference) count, unless fewer than 1 in 100 have any:
/// on a scanned page that is the text, not the paper around it.
pub(crate) fn ssim(a: &[u8], b: &[u8], w: usize, h: usize) -> f64 {
    if w.checked_mul(h).is_none_or(|c| c == 0 || a.len() < c || b.len() < c) {
        return 0.0;
    }
    let (ww, wh) = (w.min(8), h.min(8));
    let xs: Vec<usize> = (0..=w - ww).step_by(4).collect();
    let ys: Vec<usize> = (0..=h - wh).step_by(4).collect();
    let n = (ww * wh) as f64;
    let (mut total, mut windows) = (0.0, 0usize);
    let (mut detail_total, mut detail) = (0.0, 0usize);
    for y0 in &ys {
        for x0 in &xs {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in *y0..y0 + wh {
                let row = y * w;
                let (Some(ra), Some(rb)) = (a.get(row + x0..row + x0 + ww), b.get(row + x0..row + x0 + ww)) else { continue };
                for (p, q) in ra.iter().zip(rb) {
                    let (p, q) = (u64::from(*p), u64::from(*q));
                    sa += p;
                    sb += q;
                    saa += p * p;
                    sbb += q * q;
                    sab += p * q;
                }
            }
            let (ma, mb) = (sa as f64 / n, sb as f64 / n);
            let va = (saa as f64 / n - ma * ma).max(0.0);
            let vb = (sbb as f64 / n - mb * mb).max(0.0);
            let cov = sab as f64 / n - ma * mb;
            let s = ((2.0 * ma * mb + C1) * (2.0 * cov + C2)) / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            total += s;
            windows += 1;
            if va >= FLAT_VARIANCE {
                detail_total += s;
                detail += 1;
            }
        }
    }
    if detail > 0 && detail * 100 >= windows {
        detail_total / detail as f64
    } else if windows == 0 {
        0.0
    } else {
        total / windows as f64
    }
}

/// The colours of an image when it has at most `max`: the palette (`n` bytes an entry, in order
/// of first use) and each pixel's index.
pub(crate) fn palette(px: &[u8], n: usize, max: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    if n == 0 || max == 0 || max > 256 {
        return None;
    }
    let mut seen: HashMap<&[u8], u8> = HashMap::new();
    let mut entries = Vec::new();
    let mut indices = Vec::with_capacity(px.len() / n);
    for p in px.chunks_exact(n) {
        let i = match seen.get(p) {
            Some(i) => *i,
            None => {
                if seen.len() >= max {
                    return None;
                }
                let i = u8::try_from(seen.len()).ok()?;
                seen.insert(p, i);
                entries.extend_from_slice(p);
                i
            }
        };
        indices.push(i);
    }
    Some((entries, indices))
}

/// The share of pixels equal to their left neighbour: high for screenshots, charts and line art,
/// low for photos and scans, whose grain makes neighbours differ.
pub(crate) fn flatness(px: &[u8], n: usize, w: usize) -> f64 {
    let row = w.saturating_mul(n);
    if n == 0 || row <= n {
        return 0.0;
    }
    let (mut same, mut total) = (0usize, 0usize);
    for r in px.chunks_exact(row) {
        for (a, b) in r.chunks_exact(n).zip(r.chunks_exact(n).skip(1)) {
            same += usize::from(a == b);
            total += 1;
        }
    }
    if total == 0 { 0.0 } else { same as f64 / total as f64 }
}

/// Whether RGB samples are grey to the eye: no pixel's channels far apart, and only a few a
/// little apart (colour fringes of a scanner, JPEG chroma noise). A coloured stamp, signature or
/// highlight keeps the image in colour.
pub(crate) fn is_grey(px: &[u8]) -> bool {
    let (mut tinted, mut count) = (0usize, 0usize);
    for p in px.as_chunks::<3>().0 {
        let (Some(lo), Some(hi)) = (p.iter().min(), p.iter().max()) else { continue };
        let spread = hi - lo;
        if spread > 24 {
            return false;
        }
        tinted += usize::from(spread > 8);
        count += 1;
    }
    count > 0 && tinted * 1000 <= count
}

/// Whether half-resolution chroma (4:2:0) keeps an RGB image's colours: true for photos and
/// scans, false for sharp coloured edges such as coloured text, where the colour would bleed.
pub(crate) fn chroma_halves_well(px: &[u8], w: usize, h: usize) -> bool {
    let chroma = |p: &[u8]| -> (i32, i32) {
        let (r, g, b) = match p {
            [r, g, b] => (i32::from(*r), i32::from(*g), i32::from(*b)),
            _ => (0, 0, 0),
        };
        ((-43 * r - 85 * g + 128 * b) >> 8, (128 * r - 107 * g - 21 * b) >> 8)
    };
    let at = |x: usize, y: usize| -> (i32, i32) { px.get((y * w + x) * 3..(y * w + x) * 3 + 3).map_or((0, 0), chroma) };
    let (mut off, mut count) = (0usize, 0usize);
    for y in (0..h.saturating_sub(1)).step_by(2) {
        for x in (0..w.saturating_sub(1)).step_by(2) {
            let c = [at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)];
            let (mb, mr) = (c.iter().map(|v| v.0).sum::<i32>() / 4, c.iter().map(|v| v.1).sum::<i32>() / 4);
            off += c.iter().filter(|v| (v.0 - mb).abs() > 10 || (v.1 - mr).abs() > 10).count();
            count += 4;
        }
    }
    count == 0 || off * 100 <= count
}

/// The decoded luma of a JPEG the encoder made from `n`-component samples.
pub(crate) fn decoded_luma(jpeg: &[u8], n: usize) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?;
    Some(if n == 1 { img.into_luma8().into_raw() } else { luma(&img.into_rgb8().into_raw(), 3) })
}

/// Images larger than this many pixels choose their quality on a sample of tiles.
const SAMPLE_ABOVE: usize = 1 << 21;
/// The side of a sample tile: whole 4:2:0 macroblocks, so tiles don't share any.
const TILE: usize = 64;
/// Sample tiles across and down, at most.
const TILES: usize = 16;

/// A mosaic of tiles taken evenly across a large image, for choosing its quality quickly.
fn sample(px: &[u8], n: usize, w: usize, h: usize) -> Option<(Vec<u8>, usize, usize)> {
    let (tx, ty) = (w / TILE, h / TILE);
    if w.checked_mul(h)? <= SAMPLE_ABOVE || tx < 2 || ty < 2 {
        return None;
    }
    let (kx, ky) = (tx.min(TILES), ty.min(TILES));
    let (sw, sh) = (kx * TILE, ky * TILE);
    let mut out = vec![0u8; sw * sh * n];
    for j in 0..ky {
        let sy = (j * ty / ky) * TILE;
        for i in 0..kx {
            let sx = (i * tx / kx) * TILE;
            for r in 0..TILE {
                let from = ((sy + r) * w + sx) * n;
                let to = ((j * TILE + r) * sw + i * TILE) * n;
                out.get_mut(to..to + TILE * n)?.copy_from_slice(px.get(from..from + TILE * n)?);
            }
        }
    }
    Some((out, sw, sh))
}

/// A JPEG of `px` at the lowest quality from `target.floor` to `target.ceiling` whose SSIM to
/// `px` (luma) is at least the target's, with half-resolution chroma when `subsample`; `None`
/// when even the ceiling falls short. Returns the JPEG, its quality and its SSIM.
pub(crate) fn search(px: &[u8], n: usize, w: u32, h: u32, target: &Target, subsample: bool) -> Option<(Vec<u8>, u8, f64)> {
    let (wu, hu) = (usize::try_from(w).ok()?, usize::try_from(h).ok()?);
    let min = f64::from(target.min_ssim) / 10_000.0;
    let (floor, ceiling) = (target.floor.clamp(1, 100), target.ceiling.clamp(1, 100));
    if floor > ceiling {
        return None;
    }
    // Choose on a sample when the image is large, then check the whole image.
    let mut q = {
        let (spx, sw, sh) = sample(px, n, wu, hu).unwrap_or_else(|| (px.to_vec(), wu, hu));
        let reference = luma(&spx, n);
        let (sw32, sh32) = (u32::try_from(sw).ok()?, u32::try_from(sh).ok()?);
        let (mut lo, mut hi) = (floor, ceiling);
        let mut best = ceiling;
        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let ok = crate::jpeg::encode(&spx, n, sw32, sh32, mid, subsample)
                .and_then(|j| decoded_luma(&j, n))
                .is_some_and(|l| ssim(&reference, &l, sw, sh) >= min);
            if ok {
                best = mid;
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        }
        best
    };
    let reference = luma(px, n);
    for _ in 0..4 {
        let jpeg = crate::jpeg::encode(px, n, w, h, q, subsample)?;
        let s = ssim(&reference, &decoded_luma(&jpeg, n)?, wu, hu);
        if s >= min {
            return Some((jpeg, q, s));
        }
        if q >= ceiling {
            return None;
        }
        q = q.saturating_add(3).min(ceiling);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssim_is_one_for_the_same_image_and_falls_with_noise() {
        let (w, h) = (40usize, 30usize);
        let a: Vec<u8> = (0..w * h).map(|i| ((i % w) * 5 + (i / w) * 3) as u8).collect();
        assert!((ssim(&a, &a, w, h) - 1.0).abs() < 1e-9);
        let b: Vec<u8> = a.iter().enumerate().map(|(i, v)| v.saturating_add(((i * 7919) % 23) as u8)).collect();
        let s = ssim(&a, &b, w, h);
        assert!(s < 0.99 && s > 0.0, "{s}");
        // Tiny and empty images don't panic.
        assert!(ssim(&a[..12], &a[..12], 4, 3) > 0.999);
        assert_eq!(ssim(&[], &[], 0, 0), 0.0);
        assert_eq!(ssim(&a, &a[..10], w, h), 0.0);
    }

    #[test]
    fn palettes_greys_and_flat_images_are_recognised() {
        let px: Vec<u8> = (0..100).flat_map(|i| if i % 3 == 0 { [255, 0, 0] } else { [255, 255, 255] }).collect();
        let (entries, idx) = palette(&px, 3, 256).unwrap();
        assert_eq!(entries, [255, 0, 0, 255, 255, 255]);
        assert_eq!(idx.len(), 100);
        assert!(palette(&px, 3, 1).is_none());
        assert!(!is_grey(&px));
        let grey: Vec<u8> = (0..300).map(|i| (i / 3) as u8).collect();
        assert!(is_grey(&grey));
        assert!(flatness(&[1, 1, 1, 2], 1, 4) > 0.6);
        assert_eq!(flatness(&[], 3, 0), 0.0);
    }

    #[test]
    fn the_search_finds_the_lowest_quality_that_keeps_the_target() {
        let (w, h) = (96u32, 64u32);
        let px: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 2) as u8, (y * 3) as u8, ((x + y) * 2) as u8]
            })
            .collect();
        let t = Target { min_ssim: 9800, floor: 30, ceiling: 95, min_saving: 10, grey: false, requantize: 1 };
        let (jpeg, q, s) = search(&px, 3, w, h, &t, true).unwrap();
        assert!(s >= 0.98, "{s}");
        assert!((30..=95).contains(&q));
        // One step lower would have missed the target (or it is the floor).
        if q > 30 {
            let lower = crate::jpeg::encode(&px, 3, w, h, q - 1, true).unwrap();
            assert!(ssim(&luma(&px, 3), &decoded_luma(&lower, 3).unwrap(), 96, 64) < 0.98 || q <= 33);
        }
        assert!(!jpeg.is_empty());
        // An impossible target gives nothing.
        let t = Target { min_ssim: 10_000, floor: 90, ceiling: 95, ..t };
        let noisy: Vec<u8> = (0..w * h * 3).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        assert!(search(&noisy, 3, w, h, &t, false).is_none());
    }
}
