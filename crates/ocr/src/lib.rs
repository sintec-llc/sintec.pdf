//! pdfcraft-ocr — Scan & OCR ▸ Recognize text (L4).
//!
//! English is read by the ocrs engine (MIT/Apache-2.0) with its pre-trained models
//! (CC-BY-SA-4.0); every other language by PaddleOCR's PP-OCRv5 models (Apache-2.0, see
//! [`paddle`]): one shared text detector and a recognition model per writing system ([`Script`]).
//! All are fetched by `cargo xtask models` (see ATTRIBUTION.toml). The caller renders a page to pixels;
//! [`Ocr::recognize`] finds the words in them, and [`text_layer`] turns words placed in user space
//! into page content: invisible text (rendering mode 3) over each word, so the page becomes a
//! searchable image (Acrobat's "Searchable Image (Exact)": the image is left untouched).
//!
//! The ocrs models read the Latin alphabet without accents (English); the PP-OCRv5 models read
//! the languages in [`LANGUAGES`], each with the Latin letters, digits and punctuation mixed in.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod paddle;

use std::path::{Path, PathBuf};

pub use pdfcraft_fonts::helvetica_width;

/// The model files, as named in ATTRIBUTION.toml.
pub const DETECTION_MODEL: &str = "text-detection.rten";
pub const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// The PP-OCRv5 text detection model every [`Script`] but [`Script::Latin`] shares, as named in
/// ATTRIBUTION.toml.
pub const PADDLE_DETECTION_MODEL: &str = "pp-ocrv5_mobile_det.onnx";

/// The languages text can be recognised in: ISO 639-1 code and English name (the interface
/// translates it). English and Russian first, then by writing system.
pub const LANGUAGES: &[(&str, &str)] = &[
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

/// The writing system a set of models reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    /// ocrs: English and other Latin-alphabet text without accents.
    Latin,
    /// PP-OCRv5 East Slavic: Russian, Ukrainian, Belarusian.
    EastSlavic,
    /// PP-OCRv5 Cyrillic: Kazakh, Bulgarian, Serbian, Macedonian, Mongolian, Kyrgyz, Tajik.
    Cyrillic,
    /// PP-OCRv5 Latin: European languages with accents, Turkish, Indonesian.
    LatinAccented,
    /// PP-OCRv5 main model: Simplified and Traditional Chinese, Japanese.
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
    /// Every script, [`Script::Latin`] first.
    pub const ALL: [Script; 12] = [
        Script::Latin,
        Script::EastSlavic,
        Script::Cyrillic,
        Script::LatinAccented,
        Script::Chinese,
        Script::Korean,
        Script::Arabic,
        Script::Devanagari,
        Script::Greek,
        Script::Thai,
        Script::Tamil,
        Script::Telugu,
    ];

    /// The script for a [`LANGUAGES`] code; unknown codes read as Latin.
    pub fn for_language(code: &str) -> Script {
        match code {
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
            _ => Script::Latin,
        }
    }

    /// The PP-OCRv5 recognition model and its character dictionary, as named in
    /// ATTRIBUTION.toml; `None` for [`Script::Latin`] (ocrs).
    pub fn paddle_files(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Script::Latin => return None,
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
        })
    }

    /// Written right to left: the recognition model reads a line left to right as it is drawn,
    /// so its words are put back into reading order (`paddle::logical_rtl`).
    pub fn rtl(self) -> bool {
        self == Script::Arabic
    }

    /// Whether this script's model files are installed.
    pub fn available(self) -> bool {
        match self {
            Script::Latin => Models::find().is_some(),
            _ => PaddleModels::find(self).is_some(),
        }
    }
}

/// The recognition model's alphabet, as it was trained (ocrs-models' `DEFAULT_ALPHABET`): its
/// output classes follow this order. The ocrs crate's own copy has an `E` where the training had
/// `€`, which made a scanned "€250" read as "E250".
pub const ALPHABET: &str = " 0123456789!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~€ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// Shown to end users (release packages ship the models; a source build fetches them with
    /// `cargo xtask models`). The UI catalogs key this exact text.
    #[error("the text recognition models are not installed (reinstall Sintec.PDF, or set PDFCRAFT_MODELS to the folder that holds them)")]
    NoModels,
    #[error("loading {0}: {1}")]
    Load(String, String),
    #[error("text recognition failed: {0}")]
    Recognize(String),
    #[error("the image is empty")]
    EmptyImage,
}

/// Where the two model files are.
#[derive(Clone, Debug, PartialEq)]
pub struct Models {
    pub detection: PathBuf,
    pub recognition: PathBuf,
}

impl Models {
    /// The models in `dir`, if both files are there.
    pub fn in_dir(dir: &Path) -> Option<Models> {
        let m = Models { detection: dir.join(DETECTION_MODEL), recognition: dir.join(RECOGNITION_MODEL) };
        (m.detection.is_file() && m.recognition.is_file()).then_some(m)
    }

    /// Look for the models: `$PDFCRAFT_MODELS`, then where the release packages install them
    /// beside the executable ([`Models::dirs_beside_exe`]), then the source tree's `assets/models/`.
    pub fn find() -> Option<Models> {
        Self::search_dirs().iter().find_map(|d| Self::in_dir(d))
    }

    /// The directories [`Models::find`] looks in, in order.
    pub fn search_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(d) = std::env::var_os("PDFCRAFT_MODELS") {
            dirs.push(PathBuf::from(d));
        }
        if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            dirs.extend(Self::dirs_beside_exe(&exe));
        }
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"));
        dirs
    }

    /// Where the release packages put the models, relative to the directory holding the
    /// executable (packaging/*):
    /// - `models/` beside it: the Windows MSI and portable zip;
    /// - `../Resources/models`: the macOS app bundle (`Contents/MacOS` → `Contents/Resources`);
    /// - `../share/pdfcraft/models`: the FHS-style trees (`bin/` → `share/`) of the Linux
    ///   deb/rpm/tar.gz/AppImage/Flatpak and the FreeBSD tarball, wherever they are installed.
    pub fn dirs_beside_exe(exe_dir: &Path) -> Vec<PathBuf> {
        vec![exe_dir.join("models"), exe_dir.join("../Resources/models"), exe_dir.join("../share/pdfcraft/models")]
    }
}

/// Where a PP-OCRv5 script's three model files are.
#[derive(Clone, Debug, PartialEq)]
pub struct PaddleModels {
    pub script: Script,
    pub detection: PathBuf,
    pub recognition: PathBuf,
    pub dictionary: PathBuf,
}

impl PaddleModels {
    /// `script`'s models in `dir`, if all three files are there (`None` for [`Script::Latin`]).
    pub fn in_dir(dir: &Path, script: Script) -> Option<PaddleModels> {
        let (recognition, dictionary) = script.paddle_files()?;
        let m = PaddleModels {
            script,
            detection: dir.join(PADDLE_DETECTION_MODEL),
            recognition: dir.join(recognition),
            dictionary: dir.join(dictionary),
        };
        (m.detection.is_file() && m.recognition.is_file() && m.dictionary.is_file()).then_some(m)
    }

    /// Look in the same places as [`Models::find`].
    pub fn find(script: Script) -> Option<PaddleModels> {
        Models::search_dirs().iter().find_map(|d| Self::in_dir(d, script))
    }
}

/// A recognised word: its text and its box in image pixels `[left, top, right, bottom]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: [f32; 4],
}

/// A recognised line of words, in reading order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Line {
    pub words: Vec<Word>,
}

impl Line {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// A loaded recogniser. Loading takes a moment; keep one and reuse it.
pub struct Ocr {
    engine: Engine,
}

enum Engine {
    Ocrs(Box<ocrs::OcrEngine>),
    Paddle(Box<paddle::PaddleOcr>),
}

impl Ocr {
    pub fn load(models: &Models) -> Result<Ocr, OcrError> {
        let load = |p: &Path| rten::Model::load_file(p).map_err(|e| OcrError::Load(p.display().to_string(), e.to_string()));
        let params = ocrs::OcrEngineParams {
            detection_model: Some(load(&models.detection)?),
            recognition_model: Some(load(&models.recognition)?),
            alphabet: Some(ALPHABET.into()),
            ..Default::default()
        };
        let engine = ocrs::OcrEngine::new(params).map_err(|e| OcrError::Load("ocr engine".into(), e.to_string()))?;
        Ok(Ocr { engine: Engine::Ocrs(Box::new(engine)) })
    }

    /// Load a PP-OCRv5 script's models.
    pub fn load_paddle(models: &PaddleModels) -> Result<Ocr, OcrError> {
        let p = paddle::PaddleOcr::load(&models.detection, &models.recognition, &models.dictionary, models.script.rtl())?;
        Ok(Ocr { engine: Engine::Paddle(Box::new(p)) })
    }

    /// Load the Latin models found by [`Models::find`].
    pub fn find() -> Result<Ocr, OcrError> {
        Self::load(&Models::find().ok_or(OcrError::NoModels)?)
    }

    /// Load the installed models for `script`.
    pub fn find_for(script: Script) -> Result<Ocr, OcrError> {
        match script {
            Script::Latin => Self::find(),
            _ => Self::load_paddle(&PaddleModels::find(script).ok_or(OcrError::NoModels)?),
        }
    }

    /// Recognise the text in an RGBA (or RGB, or grey) image, `width` × `height` pixels.
    pub fn recognize(&self, pixels: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        if width == 0 || height == 0 || pixels.is_empty() {
            return Err(OcrError::EmptyImage);
        }
        // ocrs wants 1 or 3 channels.
        let rgb: std::borrow::Cow<[u8]> = if pixels.len() == (width * height * 4) as usize {
            pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect::<Vec<u8>>().into()
        } else {
            pixels.into()
        };
        let engine = match &self.engine {
            Engine::Ocrs(e) => e,
            Engine::Paddle(p) => {
                // The Paddle pipeline wants exactly 3 channels.
                let rgb3: std::borrow::Cow<[u8]> =
                    if rgb.len() == (width * height) as usize { rgb.iter().flat_map(|g| [*g, *g, *g]).collect::<Vec<u8>>().into() } else { rgb };
                if rgb3.len() < (width as usize).saturating_mul(height as usize).saturating_mul(3) {
                    return Err(OcrError::EmptyImage);
                }
                return p.recognize(&rgb3, width, height);
            }
        };
        let err = |e: &dyn std::fmt::Display| OcrError::Recognize(e.to_string());
        let source = ocrs::ImageSource::from_bytes(&rgb, (width, height)).map_err(|e| err(&e))?;
        let input = engine.prepare_input(source).map_err(|e| err(&e))?;
        let found = engine.detect_words(&input).map_err(|e| err(&e))?;
        let lines = engine.find_text_lines(&input, &found);
        let read = engine.recognize_text(&input, &lines).map_err(|e| err(&e))?;
        use ocrs::TextItem;
        Ok(read
            .into_iter()
            .flatten()
            .map(|line| Line {
                words: line
                    .words()
                    .map(|w| {
                        let r = w.bounding_rect();
                        Word { text: w.to_string(), rect: [r.left() as f32, r.top() as f32, r.right() as f32, r.bottom() as f32] }
                    })
                    .filter(|w| !w.text.trim().is_empty())
                    .collect(),
            })
            .filter(|l| !l.words.is_empty())
            .collect())
    }
}

/// A word placed on a page, in PDF user space: the bottom-left corner of its box, and the
/// vectors along its bottom edge (`across`, left → right as read) and its left edge (`up`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedWord {
    pub text: String,
    pub origin: [f64; 2],
    pub across: [f64; 2],
    pub up: [f64; 2],
}

impl PlacedWord {
    /// Place a pixel-space word with `to_user`, which maps an image pixel (x right, y down) to
    /// user space.
    pub fn place(word: &Word, to_user: impl Fn(f32, f32) -> [f64; 2]) -> PlacedWord {
        let [l, t, r, b] = word.rect;
        let o = to_user(l, b);
        let br = to_user(r, b);
        let tl = to_user(l, t);
        PlacedWord { text: word.text.clone(), origin: o, across: [br[0] - o[0], br[1] - o[1]], up: [tl[0] - o[0], tl[1] - o[1]] }
    }
}

/// Helvetica's ascender and descender (em fractions): a word box spans about this much.
const BOX_EM: f64 = 0.93;
const DESCENT: f64 = 0.21;

fn num(v: f64) -> String {
    let s = format!("{:.4}", if v.abs() < 1e-9 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Page content that writes `words` as invisible text, each word stretched to its box so
/// selection and search highlight the right place. Words WinAnsiEncoding can hold use `/PCHelv`
/// (standard Helvetica); others (Cyrillic, Chinese, Arabic…) use the Unicode font `/PCUni` (see
/// `pdfcraft_edit::UNICODE_FONT`), whose codes are UTF-16 and every code 1 em wide. Marked
/// content `/OCR` so it can be told apart from the page's own text.
pub fn text_layer(words: &[PlacedWord]) -> Vec<u8> {
    let mut out = b"/OCR BMC\nBT\n3 Tr\n/PCHelv 1 Tf\n".to_vec();
    let mut unicode = false;
    for w in words {
        let latin = pdfcraft_fonts::is_win_ansi(&w.text);
        if latin == unicode {
            out.extend_from_slice(if latin { b"/PCHelv 1 Tf\n" } else { b"/PCUni 1 Tf\n" });
            unicode = !latin;
        }
        let width = if latin { helvetica_width(&w.text, 1.0) } else { w.text.chars().count() as f64 };
        let height = w.up[0].hypot(w.up[1]);
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        // The em square: its height spans the box; its width is stretched to fit the word.
        let em = [w.up[0] / BOX_EM, w.up[1] / BOX_EM];
        let ax = [w.across[0] / width, w.across[1] / width];
        let o = [w.origin[0] + em[0] * DESCENT, w.origin[1] + em[1] * DESCENT];
        let m = [ax[0], ax[1], em[0], em[1], o[0], o[1]];
        let nums: Vec<String> = m.iter().map(|v| num(*v)).collect();
        out.extend_from_slice(nums.join(" ").as_bytes());
        out.extend_from_slice(b" Tm ");
        if latin {
            out.extend(pdfcraft_fonts::literal(&pdfcraft_fonts::win_ansi(&w.text)));
        } else {
            out.extend(pdfcraft_fonts::unicode_hex(&w.text));
        }
        out.extend_from_slice(b" Tj\n");
    }
    out.extend_from_slice(b"ET\nEMC\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_words_through_the_mapping() {
        // 2 pixels per point, page 612 × 792, y flipped.
        let to_user = |x: f32, y: f32| [x as f64 / 2.0, 792.0 - y as f64 / 2.0];
        let w = PlacedWord::place(&Word { text: "Hello".into(), rect: [100.0, 200.0, 300.0, 240.0] }, to_user);
        assert_eq!(w.origin, [50.0, 672.0]);
        assert_eq!(w.across, [100.0, 0.0]);
        assert_eq!(w.up, [0.0, 20.0]);
    }

    #[test]
    fn text_layer_is_invisible_text_fitted_to_each_box() {
        let w = PlacedWord { text: "Hi".into(), origin: [10.0, 20.0], across: [30.0, 0.0], up: [0.0, 9.3] };
        let s = String::from_utf8(text_layer(&[w])).unwrap();
        assert!(s.contains("3 Tr") && s.contains("/PCHelv 1 Tf"), "{s}");
        let width = helvetica_width("Hi", 1.0);
        assert!(s.contains(&format!("{} 0 0 10 10 22.1 Tm (Hi) Tj", num(30.0 / width))), "{s}");
        assert!(s.starts_with("/OCR BMC") && s.ends_with("EMC\n"));
    }

    #[test]
    fn rotated_pages_keep_the_reading_direction() {
        // A page turned 90°: image x runs up the page.
        let to_user = |x: f32, y: f32| [y as f64, x as f64];
        let w = PlacedWord::place(&Word { text: "Up".into(), rect: [0.0, 0.0, 50.0, 10.0] }, to_user);
        assert_eq!(w.across, [0.0, 50.0]);
        assert_eq!(w.up, [-10.0, 0.0]);
    }

    /// #103: every release package's layout puts the models where the installed app looks.
    /// Mirrors packaging/{windows/package.ps1,macos/package.sh,linux/package.sh,freebsd/package.sh}.
    #[test]
    fn finds_the_models_in_every_release_package_layout() {
        let base = std::env::temp_dir().join(format!("pdfcraft-ocr-layouts-{}", std::process::id()));
        let layouts = [
            ("windows", "Sintec.PDF", "Sintec.PDF/models"),
            ("macos", "Sintec PDF.app/Contents/MacOS", "Sintec PDF.app/Contents/Resources/models"),
            ("linux-deb-rpm", "usr/bin", "usr/share/pdfcraft/models"),
            ("linux-tar", "pdfcraft-1.0.0-linux-x86_64/bin", "pdfcraft-1.0.0-linux-x86_64/share/pdfcraft/models"),
            ("appimage", "Sintec.PDF.AppDir/usr/bin", "Sintec.PDF.AppDir/usr/share/pdfcraft/models"),
            ("flatpak", "app/bin", "app/share/pdfcraft/models"),
            ("freebsd", "usr/local/bin", "usr/local/share/pdfcraft/models"),
        ];
        for (name, exe_dir, models_dir) in layouts {
            let root = base.join(name);
            let (exe_dir, models_dir) = (root.join(exe_dir), root.join(models_dir));
            std::fs::create_dir_all(&exe_dir).unwrap();
            std::fs::create_dir_all(&models_dir).unwrap();
            let found = || Models::dirs_beside_exe(&exe_dir).iter().find_map(|d| Models::in_dir(d));
            assert_eq!(found(), None, "{name}: no models yet");
            for file in [DETECTION_MODEL, RECOGNITION_MODEL] {
                std::fs::write(models_dir.join(file), b"model").unwrap();
            }
            let models = found().unwrap_or_else(|| panic!("{name}: models in {} not found", models_dir.display()));
            assert_eq!(models.detection.canonicalize().unwrap(), models_dir.join(DETECTION_MODEL).canonicalize().unwrap(), "{name}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The message tells end users what to do; release builds have no cargo.
    #[test]
    fn the_alphabet_is_the_trained_one() {
        let chars: Vec<char> = ALPHABET.chars().collect();
        assert_eq!(chars.len(), 96);
        // The euro sign follows the ASCII punctuation (class 44; 0 is the CTC blank), not a second E.
        assert_eq!(chars.iter().position(|&c| c == '€'), Some(43));
        assert_eq!(chars.iter().filter(|&&c| c == 'E').count(), 1);
    }

    #[test]
    fn every_language_has_a_script_and_its_model_files() {
        let mut codes = std::collections::HashSet::new();
        for (code, name) in LANGUAGES {
            assert!(codes.insert(*code) && !name.is_empty(), "{code}");
            let script = Script::for_language(code);
            assert_eq!(script == Script::Latin, *code == "en", "{code}");
            if script != Script::Latin {
                let (rec, dict) = script.paddle_files().unwrap();
                assert!(rec.ends_with("_rec.onnx") && dict.ends_with("_dict.txt"), "{code}");
            }
        }
        // Every script is reachable from some language.
        for script in Script::ALL {
            assert!(LANGUAGES.iter().any(|(c, _)| Script::for_language(c) == script), "{script:?}");
        }
        assert_eq!(Script::for_language("xx"), Script::Latin);
        assert!(Script::Arabic.rtl() && !Script::Chinese.rtl());
    }

    #[test]
    fn paddle_models_need_all_three_files() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-ocr-paddle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(PaddleModels::in_dir(&dir, Script::Greek), None);
        assert_eq!(PaddleModels::in_dir(&dir, Script::Latin), None, "ocrs is not a PaddleOCR script");
        let (rec, dict) = Script::Greek.paddle_files().unwrap();
        for f in [PADDLE_DETECTION_MODEL, rec] {
            std::fs::write(dir.join(f), b"model").unwrap();
        }
        assert_eq!(PaddleModels::in_dir(&dir, Script::Greek), None, "no dictionary yet");
        std::fs::write(dir.join(dict), b"a").unwrap();
        let m = PaddleModels::in_dir(&dir, Script::Greek).unwrap();
        assert_eq!((m.script, m.recognition.file_name().unwrap().to_str()), (Script::Greek, Some(rec)));
        assert_eq!(PaddleModels::in_dir(&dir, Script::Thai), None, "another script's model is missing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_models_message_is_for_end_users() {
        let message = OcrError::NoModels.to_string();
        assert!(!message.contains("cargo") && message.contains("PDFCRAFT_MODELS"), "{message}");
    }

    /// End to end on rendered text, when the models are installed (`cargo xtask models`).
    #[test]
    fn reads_rendered_text() {
        let Some(models) = Models::find() else {
            eprintln!("skipped: OCR models not installed");
            return;
        };
        let ocr = Ocr::load(&models).unwrap();
        let (w, h, px) = test_image::hello();
        let lines = ocr.recognize(&px, w, h).unwrap();
        let text: Vec<String> = lines.iter().map(Line::text).collect();
        assert!(text.iter().any(|t| t.contains("HELL") && t.contains("WOR")), "{text:?}");
        let word = lines.iter().flat_map(|l| &l.words).find(|w| w.text.contains("HELL")).unwrap();
        assert!(word.rect[0] >= 10.0 && word.rect[0] < 60.0, "{word:?}");
    }

    mod test_image {
        /// "HELLO WORLD" drawn with blocky 5×7 letters, 4 pixels per dot, black on white.
        pub fn hello() -> (u32, u32, Vec<u8>) {
            const GLYPHS: &[(char, [&str; 7])] = &[
                ('H', ["X...X", "X...X", "X...X", "XXXXX", "X...X", "X...X", "X...X"]),
                ('E', ["XXXXX", "X....", "X....", "XXXX.", "X....", "X....", "XXXXX"]),
                ('L', ["X....", "X....", "X....", "X....", "X....", "X....", "XXXXX"]),
                ('O', [".XXX.", "X...X", "X...X", "X...X", "X...X", "X...X", ".XXX."]),
                ('W', ["X...X", "X...X", "X...X", "X.X.X", "X.X.X", "XX.XX", "X...X"]),
                ('R', ["XXXX.", "X...X", "X...X", "XXXX.", "X.X..", "X..X.", "X...X"]),
                ('D', ["XXXX.", "X...X", "X...X", "X...X", "X...X", "X...X", "XXXX."]),
            ];
            let text = "HELLO WORLD";
            let (dot, x0, y0) = (4u32, 40u32, 40u32);
            let (w, h) = (x0 * 2 + text.len() as u32 * 6 * dot, y0 * 2 + 7 * dot);
            let mut px = vec![255u8; (w * h * 4) as usize];
            for (i, c) in text.chars().enumerate() {
                let Some((_, rows)) = GLYPHS.iter().find(|g| g.0 == c) else { continue };
                for (ry, row) in rows.iter().enumerate() {
                    for (rx, b) in row.bytes().enumerate() {
                        if b != b'X' {
                            continue;
                        }
                        for dy in 0..dot {
                            for dx in 0..dot {
                                let x = x0 + (i as u32 * 6 + rx as u32) * dot + dx;
                                let y = y0 + ry as u32 * dot + dy;
                                let o = ((y * w + x) * 4) as usize;
                                px[o..o + 3].copy_from_slice(&[0, 0, 0]);
                            }
                        }
                    }
                }
            }
            (w, h, px)
        }
    }
}
