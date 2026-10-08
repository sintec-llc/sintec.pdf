//! PaddleOCR PP-OCRv5 text detection + recognition (Apache-2.0 models), run through rten.
//!
//! Used for the scripts the ocrs models can't read (Cyrillic). The pipeline follows PaddleOCR's
//! published inference settings: detection on a BGR image normalised with ImageNet statistics,
//! a probability map thresholded at 0.3, boxes kept when their mean probability is at least 0.6
//! and grown by the "unclip" ratio 1.5; recognition on each line scaled to 48 pixels high,
//! normalised to [-1, 1], decoded greedily (CTC: index 0 is blank, then the dictionary, then a
//! space). Word boxes come from the CTC time steps, so search highlights fall on the word.
//!
//! Boxes are axis-aligned: pages are expected to be roughly straight (as scans usually are).

use std::path::Path;

use rten::Model;
use rten_imageproc::{RetrievalMode, find_contours};
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, Tensor};

use crate::{Line, OcrError, Word};

/// Detection input: the longer side is scaled to at most this (a multiple of 32), so small
/// print on a 300 dpi page is still found without a huge tensor.
const DET_MAX_SIDE: usize = 2560;
const DET_THRESH: f32 = 0.3;
const BOX_THRESH: f32 = 0.6;
const UNCLIP_RATIO: f32 = 1.5;
/// Lines smaller than this (detection pixels) are noise.
const MIN_BOX: i32 = 3;
const REC_HEIGHT: usize = 48;
/// Very long lines are squeezed to this width rather than making one enormous tensor.
const REC_MAX_WIDTH: usize = 3200;
/// Recognised characters below this mean confidence are dropped as noise.
const MIN_LINE_CONFIDENCE: f32 = 0.5;

pub(crate) struct PaddleOcr {
    detection: Model,
    recognition: Model,
    /// Index `k` (1-based in the model output) is `dictionary[k - 1]`; one past the end is a space.
    dictionary: Vec<String>,
}

/// An RGB image borrowed from the caller.
struct Rgb<'a> {
    px: &'a [u8],
    width: usize,
    height: usize,
}

impl Rgb<'_> {
    /// Channel `c` (0 = R) at (x, y), clamped to the image; 255 (white) if out of data.
    fn at(&self, x: usize, y: usize, c: usize) -> f32 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let i = y.saturating_mul(self.width).saturating_add(x).saturating_mul(3).saturating_add(c);
        self.px.get(i).copied().map_or(255.0, f32::from)
    }

    /// Bilinear sample of channel `c` at a fractional position.
    fn sample(&self, fx: f32, fy: f32, c: usize) -> f32 {
        let fx = fx.max(0.0);
        let fy = fy.max(0.0);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let top = self.at(x0, y0, c) * (1.0 - tx) + self.at(x0 + 1, y0, c) * tx;
        let bottom = self.at(x0, y0 + 1, c) * (1.0 - tx) + self.at(x0 + 1, y0 + 1, c) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// The region `[x0, y0, x1, y1)` resized to `w` × `h` as a 1×3×h×w tensor in BGR order,
    /// each value mapped through `norm(channel, 0..=1 value)`.
    fn tensor(&self, region: [usize; 4], w: usize, h: usize, norm: impl Fn(usize, f32) -> f32) -> NdTensor<f32, 4> {
        let [x0, y0, x1, y1] = region;
        let sx = x1.saturating_sub(x0).max(1) as f32 / w.max(1) as f32;
        let sy = y1.saturating_sub(y0).max(1) as f32 / h.max(1) as f32;
        let mut t = NdTensor::<f32, 4>::zeros([1, 3, h, w]);
        for y in 0..h {
            let fy = y0 as f32 + (y as f32 + 0.5) * sy - 0.5;
            for x in 0..w {
                let fx = x0 as f32 + (x as f32 + 0.5) * sx - 0.5;
                for c in 0..3 {
                    // Paddle's models were trained on BGR images.
                    let v = self.sample(fx, fy, 2 - c) / 255.0;
                    // In bounds: y < h, x < w, c < 3 by the loops.
                    t[[0, c, y, x]] = norm(c, v);
                }
            }
        }
        t
    }
}

impl PaddleOcr {
    pub(crate) fn load(detection: &Path, recognition: &Path, dictionary: &Path) -> Result<PaddleOcr, OcrError> {
        let load = |p: &Path| Model::load_file(p).map_err(|e| OcrError::Load(p.display().to_string(), e.to_string()));
        let text = std::fs::read_to_string(dictionary).map_err(|e| OcrError::Load(dictionary.display().to_string(), e.to_string()))?;
        let entries: Vec<String> = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
        if entries.is_empty() {
            return Err(OcrError::Load(dictionary.display().to_string(), "the dictionary is empty".into()));
        }
        Ok(PaddleOcr { detection: load(detection)?, recognition: load(recognition)?, dictionary: entries })
    }

    pub(crate) fn recognize(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        let img = Rgb { px: rgb, width: width as usize, height: height as usize };
        let mut lines = Vec::new();
        for b in self.detect(&img)? {
            if let Some(line) = self.read_line(&img, b)? {
                lines.push(line);
            }
        }
        Ok(lines)
    }

    /// Text line boxes `[x0, y0, x1, y1)` in image pixels, top to bottom, left to right.
    fn detect(&self, img: &Rgb) -> Result<Vec<[usize; 4]>, OcrError> {
        let scale = (DET_MAX_SIDE as f32 / img.width.max(img.height).max(1) as f32).min(1.0);
        let round32 = |v: usize| (((v as f32 * scale) / 32.0).round() as usize).max(1) * 32;
        let (dw, dh) = (round32(img.width), round32(img.height));
        let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
        let input =
            img.tensor([0, 0, img.width, img.height], dw, dh, |c, v| (v - mean.get(c).copied().unwrap_or(0.5)) / std.get(c).copied().unwrap_or(0.25));
        let out: Tensor<f32> = self
            .detection
            .run_one(input.view().into(), None)
            .map_err(|e| OcrError::Recognize(e.to_string()))?
            .try_into()
            .map_err(|_| OcrError::Recognize("text detection returned an unexpected output".into()))?;
        if out.shape() != [1, 1, dh, dw] {
            return Err(OcrError::Recognize(format!("text detection returned shape {:?}, expected [1, 1, {dh}, {dw}]", out.shape())));
        }
        let prob = out.into_shape([dh, dw]);
        let bin = prob.map(|v| *v > DET_THRESH);
        let (fx, fy) = (img.width as f32 / dw as f32, img.height as f32 / dh as f32);
        let mut boxes = Vec::new();
        for poly in find_contours(bin.view(), RetrievalMode::External).iter() {
            let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for p in poly {
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
            if x1 - x0 < MIN_BOX || y1 - y0 < MIN_BOX {
                continue;
            }
            // Mean probability inside the box: low means a faint blob, not text.
            let (mut sum, mut n) = (0.0f32, 0usize);
            for y in y0.max(0) as usize..=(y1.max(0) as usize).min(dh - 1) {
                for x in x0.max(0) as usize..=(x1.max(0) as usize).min(dw - 1) {
                    if let Some(v) = prob.get([y, x]) {
                        sum += *v;
                        n += 1;
                    }
                }
            }
            if n == 0 || sum / (n as f32) < BOX_THRESH {
                continue;
            }
            // The model marks a shrunk core of each line; grow it back (area × ratio / perimeter).
            let (bw, bh) = ((x1 - x0) as f32, (y1 - y0) as f32);
            let off = bw * bh * UNCLIP_RATIO / (2.0 * (bw + bh));
            let left = ((x0 as f32 - off) * fx).max(0.0) as usize;
            let top = ((y0 as f32 - off) * fy).max(0.0) as usize;
            let right = (((x1 as f32 + off + 1.0) * fx) as usize).min(img.width);
            let bottom = (((y1 as f32 + off + 1.0) * fy) as usize).min(img.height);
            if right > left + 1 && bottom > top + 1 {
                boxes.push([left, top, right, bottom]);
            }
        }
        // Reading order: rows (boxes whose vertical centres are within half a line), then x.
        boxes.sort_by_key(|b| (b[1] + b[3]) / 2);
        let mut rows: Vec<Vec<[usize; 4]>> = Vec::new();
        for b in boxes {
            let centre = (b[1] + b[3]) / 2;
            match rows.last_mut() {
                Some(row) if row.first().is_some_and(|r| centre.abs_diff((r[1] + r[3]) / 2) <= (r[3] - r[1]) / 2) => row.push(b),
                _ => rows.push(vec![b]),
            }
        }
        Ok(rows
            .into_iter()
            .flat_map(|mut r| {
                r.sort_by_key(|b| b[0]);
                r
            })
            .collect())
    }

    /// Read one line box; `None` when nothing confident was read.
    fn read_line(&self, img: &Rgb, b: [usize; 4]) -> Result<Option<Line>, OcrError> {
        let [x0, y0, x1, y1] = b;
        let (cw, ch) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
        let w = ((cw as f32 * REC_HEIGHT as f32 / ch as f32).ceil() as usize).clamp(REC_HEIGHT / 3, REC_MAX_WIDTH);
        let input = img.tensor(b, w, REC_HEIGHT, |_, v| (v - 0.5) / 0.5);
        let out: Tensor<f32> = self
            .recognition
            .run_one(input.view().into(), None)
            .map_err(|e| OcrError::Recognize(e.to_string()))?
            .try_into()
            .map_err(|_| OcrError::Recognize("text recognition returned an unexpected output".into()))?;
        let out: NdTensor<f32, 3> = out.try_into().map_err(|_| OcrError::Recognize("text recognition output is not 3-D".into()))?;
        let [_, steps, classes] = out.shape();
        // Greedy CTC decode: (character, time step, probability).
        let mut chars: Vec<(String, usize, f32)> = Vec::new();
        let mut prev = 0usize;
        for t in 0..steps {
            let (mut best, mut best_p) = (0usize, f32::MIN);
            for k in 0..classes {
                let p = out.get([0, t, k]).copied().unwrap_or(f32::MIN);
                if p > best_p {
                    (best, best_p) = (k, p);
                }
            }
            if best != 0 && best != prev {
                let ch = self.dictionary.get(best - 1).cloned().unwrap_or_else(|| " ".to_string());
                chars.push((ch, t, best_p));
            }
            prev = best;
        }
        if chars.is_empty() {
            return Ok(None);
        }
        let confidence = chars.iter().map(|c| c.2).sum::<f32>() / chars.len() as f32;
        if confidence < MIN_LINE_CONFIDENCE {
            return Ok(None);
        }
        // Each time step covers this many pixels of the line.
        let step = cw as f32 / steps.max(1) as f32;
        let mut words = Vec::new();
        let mut word: Option<(String, usize, usize)> = None;
        let flush = |w: Option<(String, usize, usize)>, words: &mut Vec<Word>| {
            if let Some((text, first, last)) = w
                && !text.trim().is_empty()
            {
                let left = x0 as f32 + first as f32 * step;
                let right = (x0 as f32 + (last + 1) as f32 * step).min(x1 as f32);
                words.push(Word { text, rect: [left, y0 as f32, right.max(left + 1.0), y1 as f32] });
            }
        };
        for (ch, t, _) in chars {
            if ch.trim().is_empty() {
                flush(word.take(), &mut words);
                continue;
            }
            match word.as_mut() {
                Some((text, _, last)) => {
                    text.push_str(&ch);
                    *last = t;
                }
                None => word = Some((ch, t, t)),
            }
        }
        flush(word.take(), &mut words);
        Ok((!words.is_empty()).then_some(Line { words }))
    }
}
