//! A JPEG codec at the level of quantized DCT coefficients, written from ISO/IEC 10918-1
//! (ITU-T T.81).
//!
//! - [`read`] takes a sequential Huffman-coded JPEG (baseline or extended, 8-bit, 1 to 4
//!   components) apart into its quantization tables and coefficient blocks. Progressive,
//!   arithmetic-coded, lossless and 12-bit JPEGs are refused.
//! - [`write`] stores coefficients again with Huffman tables computed for them (Annex K.2), as
//!   one baseline scan or as progressive scans (spectral selection; each band split where it is
//!   cheapest), whichever is smaller. Restart markers and metadata that doesn't affect the
//!   pixels (Exif, XMP, Photoshop resources, comments) are left out.
//! - [`encode`] turns 8-bit samples into coefficients (YCbCr, optional 4:2:0 chroma, the
//!   Annex K tables scaled by quality) and writes them the same way.
//!
//! [`transcode`] is the lossless step built on [`read`] and [`write`]: the same coefficients and
//! tables, so every decoder draws exactly the same pixels; it is checked by decoding both.

mod fdct;
mod read;
mod write;

pub(crate) use fdct::encode;
pub(crate) use read::read;
pub(crate) use write::write;

/// The zigzag sequence: the natural (row-major) index of the `k`-th coefficient.
pub(crate) const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// JPEGs with more coefficient blocks than this are left alone (memory: 128 bytes a block).
pub(crate) const MAX_BLOCKS: usize = 1_500_000;

/// One colour component of a frame and its coefficient blocks, in zigzag order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Component {
    pub id: u8,
    /// Sampling factors (1–4).
    pub h: u8,
    pub v: u8,
    /// Quantization table selector (0–3).
    pub tq: u8,
    /// Blocks per row and rows of blocks, padded to whole MCUs of an interleaved scan.
    pub bw: usize,
    pub bh: usize,
    pub blocks: Vec<[i16; 64]>,
}

/// A decoded JPEG: what [`write`] needs to store it again.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Jpeg {
    pub width: u16,
    pub height: u16,
    pub components: Vec<Component>,
    /// Quantization tables by selector: precision (0 for 8-bit values, 1 for 16-bit) and the
    /// values in zigzag order.
    pub qtables: [Option<(u8, [u16; 64])>; 4],
    /// Application segments kept as they were (marker, payload): JFIF, ICC profiles, Adobe's
    /// colour transform and anything not known to be safe to drop.
    pub markers: Vec<(u8, Vec<u8>)>,
}

impl Jpeg {
    /// The largest horizontal and vertical sampling factors.
    pub(crate) fn max_sampling(&self) -> (usize, usize) {
        let h = self.components.iter().map(|c| usize::from(c.h)).max().unwrap_or(1).max(1);
        let v = self.components.iter().map(|c| usize::from(c.v)).max().unwrap_or(1).max(1);
        (h, v)
    }

    /// MCUs across and down in an interleaved scan.
    pub(crate) fn mcus(&self) -> (usize, usize) {
        let (hm, vm) = self.max_sampling();
        (usize::from(self.width).div_ceil(8 * hm), usize::from(self.height).div_ceil(8 * vm))
    }

    /// The blocks a non-interleaved scan of component `c` codes: across and down.
    pub(crate) fn plain_blocks(&self, c: &Component) -> (usize, usize) {
        let (hm, vm) = self.max_sampling();
        let cw = (usize::from(self.width) * usize::from(c.h)).div_ceil(hm);
        let ch = (usize::from(self.height) * usize::from(c.v)).div_ceil(vm);
        (cw.div_ceil(8).min(c.bw), ch.div_ceil(8).min(c.bh))
    }
}

/// Bits needed for the magnitude of `v` (its JPEG size category).
pub(crate) fn category(v: i32) -> u8 {
    (32 - v.unsigned_abs().leading_zeros()) as u8
}

/// `jpeg` stored again losslessly (see the module documentation), if that is smaller and decodes
/// to exactly the same pixels.
pub(crate) fn transcode(jpeg: &[u8]) -> Option<Vec<u8>> {
    let out = transcode_unchecked(jpeg)?;
    let decode = |b: &[u8]| image::load_from_memory_with_format(b, image::ImageFormat::Jpeg).ok();
    let (a, b) = (decode(jpeg)?, decode(&out)?);
    (a.color() == b.color() && a.width() == b.width() && a.height() == b.height() && a.as_bytes() == b.as_bytes()).then_some(out)
}

/// [`transcode`] without the decoding check: its size, for decisions.
pub(crate) fn transcode_unchecked(jpeg: &[u8]) -> Option<Vec<u8>> {
    let out = write(&read(jpeg)?)?;
    (out.len() < jpeg.len()).then_some(out)
}

/// Whether a three-component JPEG stores YCbCr (JFIF's default), so its first component is the
/// luma: no Adobe segment saying otherwise.
pub(crate) fn is_ycbcr(j: &Jpeg) -> bool {
    j.components.len() == 3 && j.markers.iter().all(|(m, p)| *m != 0xEE || p.get(11) == Some(&1))
}

/// A YCbCr JPEG as grey: its luma component alone, exactly as it was (no other loss).
pub(crate) fn luma_only(j: &Jpeg) -> Option<Jpeg> {
    if !is_ycbcr(j) {
        return None;
    }
    let mut y = j.components.first()?.clone();
    (y.h, y.v) = (1, 1);
    let mut qtables = [None; 4];
    *qtables.get_mut(usize::from(y.tq))? = *j.qtables.get(usize::from(y.tq))?;
    let markers = j.markers.iter().filter(|(m, _)| *m != 0xEE).cloned().collect();
    Some(Jpeg { width: j.width, height: j.height, components: vec![y], qtables, markers })
}

/// `j` with coarser quantization: the steps of zigzag coefficient `k` of luma (`chroma` false)
/// or chroma tables multiplied by `factor(chroma, k)` (at least 1), and the coefficients divided
/// by it, rounded. The DC steps stay (flat areas would band). Nothing is decoded to pixels.
pub(crate) fn requantize(j: &Jpeg, factor: &dyn Fn(bool, usize) -> u16) -> Option<Jpeg> {
    let mut out = j.clone();
    let luma_table = j.components.first().map(|c| c.tq);
    let mut by_table = [[1u16; 64]; 4];
    for (id, t) in out.qtables.iter_mut().enumerate() {
        let Some((pq, values)) = t else { continue };
        let chroma = luma_table != Some(id as u8);
        let m = &mut by_table[id & 3];
        for (k, v) in values.iter_mut().enumerate().skip(1) {
            let f = factor(chroma, k).max(1);
            m[k] = f;
            *v = v.checked_mul(f)?.min(32_767);
        }
        if values.iter().any(|v| *v > 255) {
            *pq = 1;
        }
    }
    for c in &mut out.components {
        let m = by_table.get(usize::from(c.tq))?;
        for b in &mut c.blocks {
            for (v, f) in b.iter_mut().zip(m).skip(1) {
                if *f > 1 {
                    let (a, f) = (i32::from(*v).abs(), i32::from(*f));
                    let q = (a + f / 2) / f;
                    *v = (if *v < 0 { -q } else { q }) as i16;
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests;
