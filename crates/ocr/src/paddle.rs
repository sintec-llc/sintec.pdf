//! PaddleOCR PP-OCRv5 text detection + recognition (Apache-2.0 models), run through rten.
//!
//! Used for every script but unaccented Latin (see [`crate::Script`]). The pipeline follows PaddleOCR's
//! published inference settings: detection on a BGR image normalised with ImageNet statistics,
//! a probability map thresholded at 0.3, boxes kept when their mean probability is at least 0.6
//! and grown by the "unclip" ratio 1.5; recognition on each line scaled to 48 pixels high,
//! normalised to [-1, 1], decoded greedily (CTC: index 0 is blank, then the dictionary, then a
//! space). Word boxes come from the CTC time steps, so search highlights fall on the word.
//! Right-to-left scripts come out of the model in drawing order and are put back into reading
//! order ([`logical_rtl`]).
//!
//! Boxes are axis-aligned: pages are expected to be roughly straight (as scans usually are).

use std::path::Path;

use rten::Model;
use rten_imageproc::{RetrievalMode, find_contours};
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, Tensor};

use crate::{Line, OcrError, Word};

/// What [`crate::Word::source`] carries for words this engine read.
pub const ENGINE_ID: &str = "paddle";

/// The PP-OCRv5 text detection model every [`Script`] shares, as named in ATTRIBUTION.toml.
pub const PADDLE_DETECTION_MODEL: &str = "pp-ocrv5_mobile_det.onnx";

/// Every language text can be recognised in (Sintec.PDF): English through ocrs, then
/// [`PADDLE_LANGUAGES`]. ISO 639-1 code and English name (the interface translates it).
pub const ALL_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("be", "Belarusian"),
    ("kk", "Kazakh"),
    ("bg", "Bulgarian"),
    ("sr", "Serbian"),
    ("mk", "Macedonian"),
    ("mn", "Mongolian"),
    ("ky", "Kyrgyz"),
    ("tg", "Tajik"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("cs", "Czech"),
    ("sk", "Slovak"),
    ("hu", "Hungarian"),
    ("ro", "Romanian"),
    ("tr", "Turkish"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("no", "Norwegian"),
    ("fi", "Finnish"),
    ("hr", "Croatian"),
    ("sl", "Slovenian"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("et", "Estonian"),
    ("id", "Indonesian"),
    ("el", "Greek"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("ar", "Arabic"),
    ("fa", "Persian"),
    ("ur", "Urdu"),
    ("hi", "Hindi"),
    ("mr", "Marathi"),
    ("ne", "Nepali"),
    ("th", "Thai"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
];

/// The languages PP-OCRv5 reads here: [`ALL_LANGUAGES`] but English (ocrs reads that). Russian
/// first, then by writing system.
pub const PADDLE_LANGUAGES: &[(&str, &str)] = ALL_LANGUAGES.split_at(1).1;

/// The writing system a set of PP-OCRv5 models reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    /// East Slavic: Russian, Ukrainian, Belarusian.
    EastSlavic,
    /// Cyrillic: Kazakh, Bulgarian, Serbian, Macedonian, Mongolian, Kyrgyz, Tajik.
    Cyrillic,
    /// Latin: European languages with accents, Turkish, Indonesian.
    LatinAccented,
    /// The main model: Simplified and Traditional Chinese, Japanese.
    Chinese,
    Korean,
    /// Arabic, Persian, Urdu: written right to left (see [`Script::rtl`]).
    Arabic,
    /// Hindi, Marathi, Nepali.
    Devanagari,
    Greek,
    Thai,
    Tamil,
    Telugu,
}

impl Script {
    /// The script for a [`PADDLE_LANGUAGES`] code; `None` for any other code (English and
    /// unknown codes are read by the other engines).
    pub fn for_language(code: &str) -> Option<Script> {
        Some(match code {
            "ru" | "uk" | "be" => Script::EastSlavic,
            "kk" | "bg" | "sr" | "mk" | "mn" | "ky" | "tg" => Script::Cyrillic,
            "de" | "fr" | "es" | "it" | "pt" | "nl" | "pl" | "cs" | "sk" | "hu" | "ro" | "tr" | "sv" | "da" | "no" | "fi" | "hr" | "sl" | "lt"
            | "lv" | "et" | "id" => Script::LatinAccented,
            "zh" | "ja" => Script::Chinese,
            "ko" => Script::Korean,
            "ar" | "fa" | "ur" => Script::Arabic,
            "hi" | "mr" | "ne" => Script::Devanagari,
            "el" => Script::Greek,
            "th" => Script::Thai,
            "ta" => Script::Tamil,
            "te" => Script::Telugu,
            _ => return None,
        })
    }

    /// The recognition model and its character dictionary, as named in ATTRIBUTION.toml.
    pub fn files(self) -> (&'static str, &'static str) {
        match self {
            Script::EastSlavic => ("eslav_pp-ocrv5_mobile_rec.onnx", "ppocrv5_eslav_dict.txt"),
            Script::Cyrillic => ("cyrillic_pp-ocrv5_mobile_rec.onnx", "ppocrv5_cyrillic_dict.txt"),
            Script::LatinAccented => ("latin_pp-ocrv5_mobile_rec.onnx", "ppocrv5_latin_dict.txt"),
            Script::Chinese => ("pp-ocrv5_mobile_rec.onnx", "ppocrv5_dict.txt"),
            Script::Korean => ("korean_pp-ocrv5_mobile_rec.onnx", "ppocrv5_korean_dict.txt"),
            Script::Arabic => ("arabic_pp-ocrv5_mobile_rec.onnx", "ppocrv5_arabic_dict.txt"),
            Script::Devanagari => ("devanagari_pp-ocrv5_mobile_rec.onnx", "ppocrv5_devanagari_dict.txt"),
            Script::Greek => ("el_pp-ocrv5_mobile_rec.onnx", "ppocrv5_el_dict.txt"),
            Script::Thai => ("th_pp-ocrv5_mobile_rec.onnx", "ppocrv5_th_dict.txt"),
            Script::Tamil => ("ta_pp-ocrv5_mobile_rec.onnx", "ppocrv5_ta_dict.txt"),
            Script::Telugu => ("te_pp-ocrv5_mobile_rec.onnx", "ppocrv5_te_dict.txt"),
        }
    }

    /// Written right to left: the recognition model reads a line left to right as it is drawn,
    /// so its words are put back into reading order ([`logical_rtl`]).
    pub fn rtl(self) -> bool {
        self == Script::Arabic
    }
}

/// Where a script's three model files are.
#[derive(Clone, Debug, PartialEq)]
pub struct PaddleModels {
    pub script: Script,
    pub detection: std::path::PathBuf,
    pub recognition: std::path::PathBuf,
    pub dictionary: std::path::PathBuf,
}

impl PaddleModels {
    /// `script`'s models in `dir`, if all three files are there.
    pub fn in_dir(dir: &Path, script: Script) -> Option<PaddleModels> {
        let (recognition, dictionary) = script.files();
        let m = PaddleModels {
            script,
            detection: dir.join(PADDLE_DETECTION_MODEL),
            recognition: dir.join(recognition),
            dictionary: dir.join(dictionary),
        };
        (m.detection.is_file() && m.recognition.is_file() && m.dictionary.is_file()).then_some(m)
    }

    /// Look in the same places as [`crate::Models::find`].
    pub fn find(script: Script) -> Option<PaddleModels> {
        crate::Models::search_dirs().iter().find_map(|d| Self::in_dir(d, script))
    }
}

/// A loaded PP-OCRv5 recogniser for one [`Script`]. Loading takes a moment; keep one and reuse
/// it (it is shared across threads as an `Arc<dyn Recognizer>`).
pub struct PaddleRecognizer {
    script: Script,
    engine: PaddleOcr,
}

impl PaddleRecognizer {
    /// Load `models`.
    pub fn load(models: &PaddleModels) -> Result<PaddleRecognizer, OcrError> {
        let engine = PaddleOcr::load(&models.detection, &models.recognition, &models.dictionary, models.script.rtl())?;
        Ok(PaddleRecognizer { script: models.script, engine })
    }

    /// Load the installed models for `script` ([`OcrError::NoModels`] when they are missing).
    pub fn find(script: Script) -> Result<PaddleRecognizer, OcrError> {
        Self::load(&PaddleModels::find(script).ok_or(OcrError::NoModels)?)
    }

    /// The script this recogniser reads.
    pub fn script(&self) -> Script {
        self.script
    }
}

impl crate::Recognizer for PaddleRecognizer {
    fn id(&self) -> &'static str {
        ENGINE_ID
    }

    fn available(&self) -> bool {
        true
    }

    fn languages(&self) -> Vec<(String, String)> {
        PADDLE_LANGUAGES
            .iter()
            .filter(|(c, _)| Script::for_language(c) == Some(self.script))
            .map(|(c, n)| ((*c).to_string(), (*n).to_string()))
            .collect()
    }

    fn recognize(&self, image: &crate::OcrImage, _options: &crate::RecognizeOptions) -> Result<Vec<Line>, OcrError> {
        self.engine.recognize(image.rgba(), image.width(), image.height())
    }
}

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
    /// The script is written right to left.
    rtl: bool,
}

/// An RGB or RGBA image borrowed from the caller (`channels` bytes per pixel, R first).
struct Rgb<'a> {
    px: &'a [u8],
    width: usize,
    height: usize,
    channels: usize,
}

impl Rgb<'_> {
    /// Channel `c` (0 = R) at (x, y), clamped to the image; 255 (white) if out of data.
    fn at(&self, x: usize, y: usize, c: usize) -> f32 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let i = y.saturating_mul(self.width).saturating_add(x).saturating_mul(self.channels).saturating_add(c);
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
    pub(crate) fn load(detection: &Path, recognition: &Path, dictionary: &Path, rtl: bool) -> Result<PaddleOcr, OcrError> {
        let load = |p: &Path| Model::load_file(p).map_err(|e| OcrError::Load(p.display().to_string(), e.to_string()));
        let text = std::fs::read_to_string(dictionary).map_err(|e| OcrError::Load(dictionary.display().to_string(), e.to_string()))?;
        let entries: Vec<String> = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
        if entries.is_empty() {
            return Err(OcrError::Load(dictionary.display().to_string(), "the dictionary is empty".into()));
        }
        Ok(PaddleOcr { detection: load(detection)?, recognition: load(recognition)?, dictionary: entries, rtl })
    }

    /// Read an RGBA image (an [`crate::OcrImage`]'s pixels).
    pub(crate) fn recognize(&self, rgba: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        let img = Rgb { px: rgba, width: width as usize, height: height as usize, channels: 4 };
        let mut lines = Vec::new();
        for b in self.detect(&img)? {
            if let Some(line) = self.read_line(&img, b)? {
                lines.push(if self.rtl { logical_rtl(line) } else { line });
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
                words.push(Word::new(text, [left, y0 as f32, right.max(left + 1.0), y1 as f32], ENGINE_ID));
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

/// Characters drawn left to right inside right-to-left text: Latin letters, digits and the
/// punctuation that goes with them (PaddleOCR keeps the same set in order when it reverses an
/// Arabic result).
fn ltr_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '*' | '.' | '/' | '%' | '+' | '-')
}

/// A word read right to left: each right-to-left character reversed, runs of [`ltr_char`]s
/// (numbers, Latin words) kept in their own order.
fn reverse_rtl_word(text: &str) -> String {
    let mut pieces: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        if ltr_char(c) {
            run.push(c);
            continue;
        }
        if !run.is_empty() {
            pieces.push(std::mem::take(&mut run));
        }
        pieces.push(c.to_string());
    }
    if !run.is_empty() {
        pieces.push(run);
    }
    pieces.into_iter().rev().collect()
}

/// A right-to-left line, read off the image left to right, in reading order: the words of each
/// right-to-left stretch reversed (and their letters, [`reverse_rtl_word`]); a stretch of
/// left-to-right words (a Latin name, a number) keeps its own order. Word boxes stay where the
/// words are drawn.
pub(crate) fn logical_rtl(line: Line) -> Line {
    let rtl = |w: &Word| !w.text.chars().all(|c| ltr_char(c) || !c.is_alphanumeric());
    let mut groups: Vec<(bool, Vec<Word>)> = Vec::new();
    for mut w in line.words {
        let is_rtl = rtl(&w);
        if is_rtl {
            w.text = reverse_rtl_word(&w.text);
        }
        match groups.last_mut() {
            Some((kind, words)) if *kind == is_rtl => words.push(w),
            _ => groups.push((is_rtl, vec![w])),
        }
    }
    let words = groups
        .into_iter()
        .rev()
        .flat_map(|(is_rtl, mut words)| {
            if is_rtl {
                words.reverse();
            }
            words
        })
        .collect();
    Line { words }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_one_script_and_its_model_files() {
        let mut codes = std::collections::HashSet::new();
        assert_eq!(ALL_LANGUAGES.first(), Some(&("en", "English")), "English first, read by ocrs");
        assert_eq!(PADDLE_LANGUAGES.len() + 1, ALL_LANGUAGES.len());
        assert_eq!(Script::for_language("en"), None, "English is not PP-OCRv5's");
        for (code, name) in PADDLE_LANGUAGES {
            assert!(codes.insert(*code) && !name.is_empty(), "{code}");
            let script = Script::for_language(code).unwrap_or_else(|| panic!("{code} has no script"));
            let (recognition, dictionary) = script.files();
            assert!(recognition.ends_with(".onnx") && dictionary.ends_with(".txt"), "{code}");
        }
        assert!(codes.contains("ru"));
        assert_eq!(Script::for_language("xx"), None);
    }

    #[test]
    fn models_are_found_only_when_all_three_files_are_there() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-paddle-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (recognition, dictionary) = Script::EastSlavic.files();
        std::fs::write(dir.join(PADDLE_DETECTION_MODEL), b"x").unwrap();
        std::fs::write(dir.join(recognition), b"x").unwrap();
        assert_eq!(PaddleModels::in_dir(&dir, Script::EastSlavic), None, "no dictionary yet");
        std::fs::write(dir.join(dictionary), b"x").unwrap();
        let found = PaddleModels::in_dir(&dir, Script::EastSlavic).unwrap();
        assert_eq!(found.script, Script::EastSlavic);
        assert_eq!(PaddleModels::in_dir(&dir, Script::Greek), None);
        // Files that aren't models are an error, not a panic.
        assert!(PaddleRecognizer::load(&found).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn line(words: &[&str]) -> Line {
        Line { words: words.iter().enumerate().map(|(i, t)| Word::new(*t, [i as f32 * 10.0, 0.0, i as f32 * 10.0 + 9.0, 10.0], ENGINE_ID)).collect() }
    }

    fn texts(l: &Line) -> Vec<&str> {
        l.words.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn arabic_lines_come_back_in_reading_order() {
        // "كتاب جديد" (new book) as drawn, left to right: "ديدج" then "باتك".
        let read = logical_rtl(line(&["ديدج", "باتك"]));
        assert_eq!(texts(&read), ["كتاب", "جديد"]);
        // Boxes stay with their words: the first word in reading order is the rightmost.
        assert_eq!(read.words[0].rect[0], 10.0);
    }

    #[test]
    fn numbers_and_latin_keep_their_own_order() {
        // A number inside an Arabic word, and a Latin name between Arabic words.
        assert_eq!(reverse_rtl_word("2024ماع"), "عام2024");
        assert_eq!(reverse_rtl_word("12.5%"), "12.5%");
        let read = logical_rtl(line(&["ركش", "Sintec", "PDF", "جمانرب"]));
        assert_eq!(texts(&read), ["برنامج", "Sintec", "PDF", "شكر"]);
    }

    #[test]
    fn odd_lines_do_not_panic() {
        assert!(logical_rtl(Line::default()).words.is_empty());
        for t in ["", "\u{200F}", "،", "ا\u{0651}", "a", "١٢٣"] {
            let _ = logical_rtl(line(&[t, t]));
        }
    }
}
