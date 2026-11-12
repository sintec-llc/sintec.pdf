//! 8-bit samples to JPEG: YCbCr (JFIF), optional 4:2:0 chroma, the forward DCT and the
//! Annex K quantization tables scaled by quality (the scaling the IJG software made common).

use super::{Component, Jpeg, MAX_BLOCKS, ZIGZAG};

/// T.81 Table K.1 (luminance), natural order.
const LUMA: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56,
    68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// T.81 Table K.2 (chrominance), natural order.
const CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// `base` scaled for `quality` (1–100), in zigzag order.
fn table(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = u32::from(quality.clamp(1, 100));
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut t = [0u16; 64];
    for (k, z) in ZIGZAG.iter().enumerate() {
        t[k] = ((u32::from(base[*z]) * scale + 50) / 100).clamp(1, 255) as u16;
    }
    t
}

/// One plane of samples.
struct Plane {
    w: usize,
    h: usize,
    px: Vec<f32>,
}

impl Plane {
    /// The sample at `(x, y)`, the edge repeated beyond it.
    fn at(&self, x: usize, y: usize) -> f32 {
        let (x, y) = (x.min(self.w.saturating_sub(1)), y.min(self.h.saturating_sub(1)));
        self.px.get(y * self.w + x).copied().unwrap_or(0.0)
    }

    /// Half the size each way, each sample the mean of four.
    fn halved(&self) -> Plane {
        let (w, h) = (self.w.div_ceil(2), self.h.div_ceil(2));
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let s = self.at(2 * x, 2 * y) + self.at(2 * x + 1, 2 * y) + self.at(2 * x, 2 * y + 1) + self.at(2 * x + 1, 2 * y + 1);
                px.push(s / 4.0);
            }
        }
        Plane { w, h, px }
    }
}

/// `px` (8-bit, `n` = 1 grey or 3 RGB components, `w` × `h`) as a JPEG of `quality`, the chroma
/// at half resolution each way when `subsample`.
pub(crate) fn encode(px: &[u8], n: usize, w: u32, h: u32, quality: u8, subsample: bool) -> Option<Vec<u8>> {
    let (width, height) = (u16::try_from(w).ok()?, u16::try_from(h).ok()?);
    let (wu, hu) = (usize::from(width), usize::from(height));
    if wu == 0 || hu == 0 || !matches!(n, 1 | 3) || px.len() < wu.checked_mul(hu)?.checked_mul(n)? {
        return None;
    }
    let count = wu * hu;
    let mut planes: Vec<Plane> = (0..n).map(|_| Plane { w: wu, h: hu, px: Vec::with_capacity(count) }).collect();
    for p in px.chunks_exact(n).take(count) {
        if let ([y, cb, cr], [r, g, b]) = (planes.as_mut_slice(), p) {
            let (r, g, b) = (f32::from(*r), f32::from(*g), f32::from(*b));
            y.px.push(0.299 * r + 0.587 * g + 0.114 * b);
            cb.px.push(-0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0);
            cr.px.push(0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0);
        } else if let ([y], [v]) = (planes.as_mut_slice(), p) {
            y.px.push(f32::from(*v));
        }
    }
    let subsample = subsample && n == 3;
    if subsample {
        for p in planes.iter_mut().skip(1) {
            *p = p.halved();
        }
    }
    let luma_h = if subsample { 2u8 } else { 1 };
    let (mx, my) = (wu.div_ceil(8 * usize::from(luma_h)), hu.div_ceil(8 * usize::from(luma_h)));
    let blocks: usize = if subsample { mx * my * 6 } else { mx * my * n };
    if blocks > MAX_BLOCKS {
        return None;
    }
    let tables = [table(&LUMA, quality), table(&CHROMA, quality)];
    // The DCT basis: c[u][x] = C(u)/2 · cos((2x + 1)uπ/16), so F = c · f · cᵀ (T.81 A.3.3).
    let mut c = [[0f32; 8]; 8];
    for (u, row) in c.iter_mut().enumerate() {
        let a = if u == 0 { (0.125f32).sqrt() } else { 0.5 };
        for (x, v) in row.iter_mut().enumerate() {
            *v = a * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    let mut components = Vec::with_capacity(n);
    for (i, plane) in planes.iter().enumerate() {
        let (hs, tq) = if i == 0 { (luma_h, 0u8) } else { (1, 1) };
        let (bw, bh) = (mx * usize::from(hs), my * usize::from(hs));
        let q = &tables[usize::from(tq)];
        let mut out = Vec::with_capacity(bw * bh);
        for by in 0..bh {
            for bx in 0..bw {
                let mut f = [[0f32; 8]; 8];
                for (y, row) in f.iter_mut().enumerate() {
                    for (x, v) in row.iter_mut().enumerate() {
                        *v = plane.at(bx * 8 + x, by * 8 + y) - 128.0;
                    }
                }
                // Rows, then columns.
                let mut t = [[0f32; 8]; 8];
                for (y, row) in f.iter().enumerate() {
                    for (u, cu) in c.iter().enumerate() {
                        t[y][u] = row.iter().zip(cu).map(|(a, b)| a * b).sum();
                    }
                }
                let mut block = [0i16; 64];
                for (k, z) in ZIGZAG.iter().enumerate() {
                    let (v, u) = (z / 8, z % 8);
                    let coef: f32 = (0..8).map(|y| t[y][u] * c[v][y]).sum();
                    let limit = if k == 0 { 1024.0 } else { 1023.0 };
                    block[k] = (coef / f32::from(q[k])).round().clamp(-limit, limit) as i16;
                }
                out.push(block);
            }
        }
        components.push(Component { id: i as u8 + 1, h: hs, v: hs, tq, bw, bh, blocks: out });
    }
    let mut qtables = [None; 4];
    qtables[0] = Some((0u8, tables[0]));
    if n == 3 {
        qtables[1] = Some((0u8, tables[1]));
    }
    // JFIF 1.01, no density, no thumbnail: decoders then take three components as YCbCr.
    let jfif = b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0".to_vec();
    super::write(&Jpeg { width, height, components, qtables, markers: vec![(0xE0, jfif)] })
}
