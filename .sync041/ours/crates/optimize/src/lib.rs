//! The PDF Optimizer (execution plan M11.1; Acrobat: Reduce File Size and Optimize PDF ▸
//! Advanced optimization).
//!
//! - **Merge image fragments:** a page picture some PDF printers draw as thousands of images one
//!   pixel row high is put back together into one image first (see [`fragments`]), so the image
//!   settings can work on it.
//! - **Images:** each image's effective resolution is measured where pages draw it (the
//!   smallest over its uses, through form XObjects); colour and grayscale images above a
//!   threshold are resampled (Lanczos) to a target resolution and recompressed as JPEG or Flate:
//!   at a fixed quality, or ([`Compression::Targeted`]) at the lowest quality that keeps the
//!   image's SSIM, graphics staying lossless and JPEGs that aren't resampled made smaller on
//!   their own coefficients ([`Compression::Lossless`] only converts few-colour images to an
//!   indexed palette). JPEG is written by PdfCraft's own encoder (4:2:0 chroma when the colours
//!   allow it, Huffman tables made for each image). A new image replaces the old one only if it
//!   is smaller. Images PdfCraft can't decode
//!   faithfully (CMYK and other colour spaces, masks, decode arrays other than the default,
//!   JPEG 2000, JBIG2, CCITT, other than 8 bits) are left alone.
//! - **Discard objects:** page thumbnails, alternate images, document tags (structure tree),
//!   print settings.
//! - **Clean up:** drop images and forms that pages list but never draw (such as a deleted
//!   image); Flate-compress streams that have no filter; remove a redundant outer ASCII85 or
//!   ASCIIHex layer from streams that have one.
//! - **Recompress streams** (lossless): every stream in a general-purpose encoding is stored
//!   again with Flate at the best level, image samples with a PNG predictor; JPEGs are transcoded losslessly (the same
//!   coefficients, better Huffman tables, no Exif); a stream is replaced only when that is
//!   smaller and, for a JPEG, decodes to the same pixels. JPEG 2000, JBIG2 and CCITT data stay
//!   as they are.
//! - **Estimate:** [`estimate_with_progress`] predicts what the slow steps would make of each
//!   stream, from trial encodes of a budgeted sample, so a caller can show sizes before it
//!   optimizes.
//!
//! Metadata, attachments, comments, scripts, private data, hidden layers, bookmarks and form
//! fields are discarded by `pdfcraft-redact`'s Remove Hidden Information, which the engine
//! runs alongside.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod audit;
mod estimate;
pub mod fragments;
mod images;
mod jpeg;
mod links;
mod quality;
mod recompress;
mod unused;

pub use estimate::{Budget, StreamSizes, TrialCache, estimate_with_progress, reachable};

pub use audit::{SpaceCategory, SpaceUse, audit_space};

use pdfcraft_cos::{Document, ObjRef, Object, Stream};

pub use images::effective_resolutions;

#[derive(Debug, thiserror::Error)]
pub enum OptimizeError {
    #[error("the document has no page tree")]
    NoPages,
    #[error(transparent)]
    Cos(#[from] pdfcraft_cos::CosError),
    #[error("the optimization was cancelled")]
    Cancelled,
}

/// Where an optimization is, reported to [`optimize_with_progress`]'s callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// About to process image `done` (0-based) of `total`; `done == total` once all are done.
    Images { done: usize, total: usize },
    /// Merging the fragments of a page picture into one image (only when a page has some).
    Fragments,
    /// Discarding objects, cleaning up links and compressing unencoded streams.
    CleanUp,
    /// Recompressing stream `done` of `total` (the objects of the file, in order).
    Streams { done: usize, total: usize },
}

/// How resampled (or recompressed) images are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// JPEG with a quality from 1 to 100.
    Jpeg(u8),
    /// Lossless (zlib).
    Flate,
    /// Keep the image's own compression (resampled images become Flate if they weren't JPEG).
    Retain,
    /// Lossless only: an image that isn't JPEG is stored as an indexed palette when it has few
    /// enough colours and that is smaller (JPEGs are left to the lossless stream pass).
    Lossless,
    /// JPEG at the lowest quality that keeps the image looking the same ([`Target`]); graphics
    /// (few colours or large flat areas: screenshots, charts, line art, text) stay lossless.
    Targeted(Target),
}

/// What [`Compression::Targeted`] holds an image to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// The lowest structural similarity (SSIM of the luma over 8×8 windows) the JPEG may have to
    /// the image it replaces (resampled, if it is), in ten-thousandths: 9850 is 0.985.
    pub min_ssim: u16,
    /// The JPEG qualities searched (1–100). The floor holds even when the SSIM would allow less.
    pub floor: u8,
    pub ceiling: u8,
    /// A JPEG that isn't resampled is re-encoded only when that saves at least this share (in
    /// percent) of its size after the lossless transcode.
    pub min_saving: u8,
    /// Store colour images that are grey to the eye (scans of black-and-white pages) as grey.
    pub grey: bool,
    /// A JPEG that isn't resampled is made smaller on its own coefficients: its quantization
    /// steps multiplied by up to this (1 leaves them; the SSIM target still decides how far).
    pub requantize: u8,
}

/// Acrobat's JPEG quality steps.
pub const QUALITIES: [(&str, u8); 5] = [("Minimum", 25), ("Low", 45), ("Medium", 60), ("High", 80), ("Maximum", 95)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSettings {
    /// Resample images whose effective resolution is above `above_ppi` down to `target_ppi`.
    pub downsample: bool,
    pub target_ppi: f64,
    pub above_ppi: f64,
    pub compression: Compression,
}

impl ImageSettings {
    /// Reduce File Size and the Standard preset: bicubic to 150 ppi above 225 ppi, JPEG medium
    /// quality.
    pub const REDUCE: ImageSettings = ImageSettings { downsample: true, target_ppi: 150.0, above_ppi: 225.0, compression: Compression::Jpeg(60) };
    /// The Mobile preset: bicubic to 96 ppi above 144 ppi, JPEG high quality. Pages read on a
    /// screen; text in scans stays legible.
    pub const MOBILE: ImageSettings = ImageSettings { downsample: true, target_ppi: 96.0, above_ppi: 144.0, compression: Compression::Jpeg(80) };
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Put page pictures drawn as many stacked fragments back together first ([`fragments`]).
    pub merge_fragments: bool,
    pub color: ImageSettings,
    pub gray: ImageSettings,
    pub discard_thumbnails: bool,
    pub discard_alternate_images: bool,
    pub discard_tags: bool,
    pub discard_print_settings: bool,
    /// Flate-compress streams stored without a filter, and strip a redundant outer ASCII85 or
    /// ASCIIHex layer from streams that have one.
    pub flate_unencoded: bool,
    /// Clean Up: remove links (and bookmark destinations) that point nowhere.
    pub remove_invalid_links: bool,
    /// Clean Up: remove named destinations nothing refers to.
    pub remove_unreferenced_dests: bool,
    /// Clean Up: store every stream in a general-purpose encoding again with Flate at the best
    /// level (image samples with a PNG predictor), when that is smaller. Lossless.
    pub recompress_streams: bool,
}

impl Default for Settings {
    /// Acrobat's Reduce File Size choices.
    fn default() -> Self {
        Self {
            merge_fragments: true,
            color: ImageSettings::REDUCE,
            gray: ImageSettings::REDUCE,
            discard_thumbnails: true,
            discard_alternate_images: true,
            discard_tags: false,
            discard_print_settings: false,
            flate_unencoded: true,
            remove_invalid_links: true,
            remove_unreferenced_dests: true,
            recompress_streams: true,
        }
    }
}

/// The optimizer's ready-made settings, as in Acrobat's Settings menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Reduce File Size's choices ([`Settings::default`]).
    Standard,
    /// Smaller files for reading on a phone or screen: images to 96 ppi above 144, JPEG high.
    Mobile,
}

impl Preset {
    pub const ALL: [Preset; 2] = [Preset::Standard, Preset::Mobile];

    /// The name tools use.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Standard => "standard",
            Preset::Mobile => "mobile",
        }
    }

    pub fn from_name(name: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.name() == name)
    }

    pub fn settings(self) -> Settings {
        match self {
            Preset::Standard => Settings::default(),
            Preset::Mobile => Settings { color: ImageSettings::MOBILE, gray: ImageSettings::MOBILE, ..Settings::default() },
        }
    }

    /// The preset `settings` are, if they are one.
    pub fn of(settings: &Settings) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.settings() == *settings)
    }
}

/// What an optimization did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Images merged from fragments, and how many fragments went into them.
    pub merged_images: usize,
    pub fragments_merged: usize,
    pub images: usize,
    pub images_resampled: usize,
    pub images_recompressed: usize,
    /// Encoded image bytes before and after (images that changed).
    pub image_bytes_before: usize,
    pub image_bytes_after: usize,
    pub thumbnails: usize,
    pub alternate_images: usize,
    pub tags_removed: bool,
    pub print_settings: usize,
    pub streams_compressed: usize,
    /// Links removed and bookmarks whose destination was cleared (Clean Up).
    pub invalid_links: usize,
    pub invalid_bookmarks: usize,
    pub unreferenced_dests: usize,
    /// Images and forms the pages listed but never drew, dropped (Clean Up).
    pub unused_xobjects: usize,
    /// Streams recompressed losslessly (Flate at the best level, predictors for images).
    pub streams_recompressed: usize,
}

/// Optimize `doc` in place.
pub fn optimize(doc: &mut Document, settings: &Settings) -> Result<Report, OptimizeError> {
    optimize_with_progress(doc, settings, &mut |_| true)
}

/// [`optimize`], calling `progress` before each image and before the clean-up (and now and then
/// during a long one). Returning `false` stops with [`OptimizeError::Cancelled`]; `doc` is then
/// partly optimized and should be dropped.
pub fn optimize_with_progress(doc: &mut Document, settings: &Settings, progress: &mut dyn FnMut(Stage) -> bool) -> Result<Report, OptimizeError> {
    let pages = pdfcraft_annot::page_refs(doc).map_err(|_| OptimizeError::NoPages)?;
    let mut report = Report::default();
    if settings.merge_fragments {
        fragments::run(doc, &pages, &mut report, progress)?;
    }
    images::run(doc, &pages, settings, &mut report, progress)?;
    clean_up(doc, &pages, settings, &mut report, progress)?;
    compress_streams(doc, settings, &mut report, progress)?;
    Ok(report)
}

/// The steps between the images and the stream compression: unused images and forms, the
/// Discard Objects choices and the link clean-up. They are quick, so estimates run them too.
fn clean_up(
    doc: &mut Document,
    pages: &[ObjRef],
    settings: &Settings,
    report: &mut Report,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<(), OptimizeError> {
    if !progress(Stage::CleanUp) {
        return Err(OptimizeError::Cancelled);
    }
    report.unused_xobjects = unused::remove_unused(doc, pages, progress)?;
    if settings.discard_thumbnails {
        for p in pages {
            if doc.get(*p).as_dict().is_some_and(|d| d.contains(b"Thumb")) {
                doc.update_dict(*p, |d| {
                    d.remove(b"Thumb");
                })?;
                report.thumbnails += 1;
            }
        }
    }
    if settings.discard_alternate_images {
        for num in doc.object_numbers() {
            let r = ObjRef { num, generation: doc.generation(num) };
            let alt = matches!(&*doc.get(r), Object::Stream(s) if s.dict.name(b"Subtype") == Some(b"Image") && s.dict.contains(b"Alternates"));
            if alt {
                if let Object::Stream(s) = &*doc.get(r) {
                    let mut s = s.clone();
                    s.dict.remove(b"Alternates");
                    doc.set(r, Object::Stream(s));
                }
                report.alternate_images += 1;
            }
        }
    }
    let root = doc.root();
    if settings.discard_tags
        && let Some(root) = root
        && doc.get(root).as_dict().is_some_and(|c| c.contains(b"StructTreeRoot"))
    {
        doc.update_dict(root, |c| {
            c.remove(b"StructTreeRoot");
            c.remove(b"MarkInfo");
        })?;
        for p in pages {
            if doc.get(*p).as_dict().is_some_and(|d| d.contains(b"StructParents")) {
                doc.update_dict(*p, |d| {
                    d.remove(b"StructParents");
                })?;
            }
        }
        report.tags_removed = true;
    }
    if settings.discard_print_settings
        && let Some(root) = root
    {
        const PRINT: [&[u8]; 7] = [b"PrintScaling", b"Duplex", b"PickTrayByPDFSize", b"PrintPageRange", b"NumCopies", b"PrintArea", b"PrintClip"];
        let vp = doc.get(root).as_dict().and_then(|c| c.get(b"ViewerPreferences").cloned());
        let edit = |d: &mut pdfcraft_cos::Dict| PRINT.iter().filter(|k| d.remove(k).is_some()).count();
        match vp {
            Some(Object::Ref(r)) => {
                let mut n = 0;
                doc.update_dict(r, |d| n = edit(d))?;
                report.print_settings += n;
            }
            Some(Object::Dict(_)) => {
                let mut n = 0;
                doc.update_dict(root, |c| {
                    if let Some(Object::Dict(d)) = c.get_mut(b"ViewerPreferences") {
                        n = edit(d);
                    }
                })?;
                report.print_settings += n;
            }
            _ => {}
        }
    }
    if settings.remove_invalid_links {
        let (l, b) = links::remove_invalid(doc, pages)?;
        report.invalid_links = l;
        report.invalid_bookmarks = b;
    }
    if settings.remove_unreferenced_dests {
        report.unreferenced_dests = links::remove_unreferenced_dests(doc)?;
    }
    Ok(())
}

/// How many objects are looked at between two progress reports while compressing streams.
const STREAMS_PER_REPORT: usize = 64;

/// The lossless stream pass: [`Settings::recompress_streams`] and [`Settings::flate_unencoded`].
fn compress_streams(
    doc: &mut Document,
    settings: &Settings,
    report: &mut Report,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<(), OptimizeError> {
    if !settings.recompress_streams && !settings.flate_unencoded {
        return Ok(());
    }
    let encrypted = doc.trailer().contains(b"Encrypt");
    let numbers = doc.object_numbers();
    let total = numbers.len();
    for (done, num) in numbers.into_iter().enumerate() {
        if done % STREAMS_PER_REPORT == 0 && !progress(Stage::Streams { done, total }) {
            return Err(OptimizeError::Cancelled);
        }
        let r = ObjRef { num, generation: doc.generation(num) };
        let obj = doc.get(r);
        let Object::Stream(s) = &*obj else { continue };
        let (replacement, recompressed) = match recompress::eligible(s, encrypted) {
            Some(_) if settings.recompress_streams => (recompress::recompress(doc, s), true),
            _ if settings.flate_unencoded => (flate_unencoded(s), false),
            _ => (None, false),
        };
        if let Some(replacement) = replacement {
            doc.set(r, Object::Stream(replacement));
            if recompressed {
                report.streams_recompressed += 1;
            } else {
                report.streams_compressed += 1;
            }
        }
    }
    if !progress(Stage::Streams { done: total, total }) {
        return Err(OptimizeError::Cancelled);
    }
    Ok(())
}

/// Clean Up's "Use Flate to encode streams that are not encoded" for one stream: Flate for a
/// stream without a filter, or without its redundant outer ASCII layer.
fn flate_unencoded(s: &Stream) -> Option<Stream> {
    // XMP stays readable as plain text (PDF/A requires it); tiny streams don't gain.
    if s.dict.name(b"Type") == Some(b"Metadata") || s.dict.name(b"Type") == Some(b"XRef") || s.raw.len() < 64 {
        return None;
    }
    if s.dict.contains(b"Filter") {
        strip_ascii_wrapper(s)
    } else {
        let mut d = s.dict.clone();
        d.remove(b"Length");
        let packed = Stream::flate(d, &s.raw);
        (packed.raw.len() < s.raw.len()).then_some(packed)
    }
}

/// `s` without its outer ASCII85 or ASCIIHex layer (`[/ASCII85Decode /FlateDecode]` becomes
/// `[/FlateDecode]`), when that is smaller. Only the outer layer is decoded: strictly, and within
/// the stored size, which ASCII text never falls below, so the inner encoded bytes stay exact.
fn strip_ascii_wrapper(s: &Stream) -> Option<Stream> {
    if s.dict.contains(b"DecodeParms") || s.dict.name(b"Type") == Some(b"ObjStm") || s.dict.name(b"Subtype") == Some(b"Image") {
        return None;
    }
    let Some(Object::Array(filters)) = s.dict.get(b"Filter") else { return None };
    let names = filters.iter().map(Object::as_name).collect::<Option<Vec<_>>>()?;
    let (outer, inner) = names.split_first()?;
    if !matches!(*outer, b"ASCII85Decode" | b"ASCIIHexDecode") || inner.is_empty() || inner.contains(&&b"Crypt"[..]) {
        return None;
    }
    let data = s.decoded_first_filter_strict(s.raw.len()).ok()?;
    let mut dict = s.dict.clone();
    dict.remove(b"Length");
    dict.set(b"Filter".to_vec(), Object::Array(inner.iter().map(|n| Object::Name(n.to_vec())).collect()));
    let candidate = Stream::from_raw(dict, data);
    (recompress::stored_len(&candidate)? < recompress::stored_len(s)?).then_some(candidate)
}
