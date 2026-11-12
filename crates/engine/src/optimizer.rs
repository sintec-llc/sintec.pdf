//! Optimize PDF ▸ Advanced optimization as a job that reports its progress.
//!
//! Optimizing a large file takes a while (every image is decoded, resampled and re-encoded), so
//! it is split like OCR: [`Session::optimize_job`] captures a copy of the document and the
//! choices, and [`OptimizeJob::run`] works anywhere (the UI runs it on a worker thread),
//! reporting each [`OptimizeStage`] and stopping when asked to.
//!
//! Reduce File Size offers three [`Level`]s and shows, before the user picks one, about how
//! large each would make the file: [`Session::estimate_job`] and [`EstimateJob::run`], also on
//! a worker thread.
//!
//! # The levels
//!
//! This table is the one definition of the levels ([`Level::settings`], [`Level::discard`]);
//! `docs/file-size-levels.md` repeats it for users.
//!
//! | | Light | Medium | Strong |
//! |---|---|---|---|
//! | Images | lossless only: every pixel stays the same | above 220 ppi resampled to 150 ppi (Lanczos); JPEG quality searched per image for SSIM ≥ 0.985 (quality 70–92) | above 160 ppi resampled to 120 ppi; JPEG quality searched for SSIM ≥ 0.970 (quality 65–90) |
//! | JPEGs not resampled | transcoded losslessly (Huffman tables made for the image, progressive when smaller, Exif/XMP/comments dropped) | grey-looking colour pages stored as their luma alone; quantization steps doubled where SSIM ≥ 0.985 holds; kept only if 5 % smaller than the lossless transcode | as Medium, steps up to tripled, SSIM ≥ 0.970, 3 % smaller |
//! | Graphics (≤ 256 colours, or ≥ half the pixels equal to their neighbour) | indexed palette (1–8 bits) when smaller | same: never JPEG, never resampled | same |
//! | Black-and-white images (1-bit, image masks, CCITT, JBIG2; grey or RGB of only black and white) | CCITT Group 4 when smaller (lossless), never resampled | above 450 ppi averaged to 300 ppi; Group 4 | as Medium, stored as lossless JBIG2 (a symbol dictionary shared by up to 16 images) when smaller |
//! | Streams | Flate at the best level, PNG predictors for image samples; LZW stored as Flate | same | same |
//! | Structure | object and cross-reference streams, identical objects merged, unused objects dropped | same | same |
//! | Discarded | page thumbnails, alternate images | same | same, plus print settings, document information and XMP metadata, other applications' private data, invalid links and unreferenced named destinations |
//! | Kept | text, fonts, forms, comments, attachments, bookmarks, links, tags, layers | same | same, except what is discarded |
//!
//! Images are measured where pages draw them, and every image or stream is replaced only when
//! the new one is smaller. The SSIM is the luma's, over 8×8 windows, counting only windows with
//! detail (the text of a scan, not the paper); grain that misses the target even at the ceiling
//! quality is stored at the ceiling. Colour images are stored grey only when no pixel is more
//! than slightly tinted (a coloured stamp or signature keeps the colour), and with 4:2:0 chroma
//! only when that keeps sharp coloured edges. Signed documents are refused: a full rewrite would
//! invalidate their signatures.

use std::sync::Arc;

use pdfcraft_cos::{Object, SaveOptions, Stream, write_full};

use crate::{DocId, EditError, Hidden, OptimizeReport, Session, guard, optimize};

/// Where an optimization is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimizeStage {
    /// Removing the user data chosen under Discard User Data, if any was.
    Discarding,
    /// Reading what the pages draw: `done` of `total` read.
    Reading { done: usize, total: usize },
    /// Merging the fragments of a page picture into one image.
    Fragments,
    /// About to process image `done` (0-based) of `total`.
    Images { done: usize, total: usize },
    /// Discarding objects and compressing unencoded streams.
    CleaningUp,
    /// Recompressing stream `done` of `total` losslessly.
    Streams { done: usize, total: usize },
    /// Merging identical fonts and images.
    Merging,
    /// Writing the compressed copy.
    Writing,
}

impl Default for OptimizeStage {
    fn default() -> Self {
        OptimizeStage::Reading { done: 0, total: 0 }
    }
}

impl OptimizeStage {
    /// How far along the whole run is, from 0 to 1, weighed by where the time goes: reading pages
    /// and pictures. The rest are small fixed steps.
    pub fn fraction(self) -> f32 {
        let part = |done: usize, total: usize| if total == 0 { 1.0 } else { done.min(total) as f32 / total as f32 };
        match self {
            OptimizeStage::Discarding | OptimizeStage::Fragments => 0.0,
            OptimizeStage::Reading { done, total } => 0.25 * if total == 0 { 0.0 } else { part(done, total) },
            OptimizeStage::Images { done, total } => 0.25 + 0.55 * part(done, total),
            OptimizeStage::CleaningUp => 0.8,
            OptimizeStage::Streams { done, total } => 0.8 + 0.07 * part(done, total),
            OptimizeStage::Merging => 0.88,
            OptimizeStage::Writing => 0.92,
        }
    }
}

/// The optimized copy's bytes and what was done.
pub type Optimized = (Arc<Vec<u8>>, OptimizeReport);

/// What an optimization needs, independent of the session.
pub struct OptimizeJob {
    cos: pdfcraft_cos::Document,
    settings: optimize::Settings,
    discard: Vec<Hidden>,
    opts: SaveOptions,
}

impl Session {
    /// Capture what optimizing document `id` with `settings` and the Remove Hidden Information
    /// `discard` categories needs. A full rewrite: signed documents are refused.
    pub fn optimize_job(&self, id: DocId, settings: &optimize::Settings, discard: &[Hidden]) -> Result<OptimizeJob, EditError> {
        let cos = self.optimizable(id)?;
        Ok(OptimizeJob { cos, settings: settings.clone(), discard: discard.to_vec(), opts: save_options(self, settings) })
    }

    /// [`Self::optimize_job`] with a Reduce File Size level's choices.
    pub fn reduce_job(&self, id: DocId, level: Level) -> Result<OptimizeJob, EditError> {
        self.optimize_job(id, &level.settings(), level.discard())
    }

    /// Capture what estimating each level's result for document `id` needs. Refused like
    /// [`Self::optimize_job`]: a signed document can't be reduced, so there is nothing to
    /// estimate.
    pub fn estimate_job(&self, id: DocId) -> Result<EstimateJob, EditError> {
        let cos = self.optimizable(id)?;
        Ok(EstimateJob { cos, now: self.now().map(pdfcraft_cos::pdf_date), budget: optimize::Budget::default() })
    }

    /// A copy of document `id`'s objects, if it may be optimized (not signed, editable).
    fn optimizable(&self, id: DocId) -> Result<pdfcraft_cos::Document, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if doc.is_signed() {
            return Err(EditError::Signed);
        }
        let editor = doc.editor.as_ref().ok_or_else(|| EditError::ReadOnly(doc.read_only_reason.clone().unwrap_or_default()))?;
        Ok(editor.cos.clone())
    }
}

/// How optimized copies are written: object streams as `settings` say, at the best Flate level.
fn save_options(session: &Session, settings: &optimize::Settings) -> SaveOptions {
    options(session.now().map(pdfcraft_cos::pdf_date), settings)
}

fn options(mod_date: Option<String>, settings: &optimize::Settings) -> SaveOptions {
    SaveOptions { mod_date, object_streams: settings.effective_object_streams(), best_compression: true, ..SaveOptions::default() }
}

/// Reduce File Size's levels (the [module documentation](self) has the table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Level {
    /// Lossless only: every pixel stays; JPEGs transcoded, black-and-white images as CCITT Group
    /// 4, streams recompressed, structure compacted, thumbnails dropped.
    Light,
    /// Visually lossless: images to 150 ppi above 220, JPEG quality chosen per image for
    /// SSIM ≥ 0.985; black-and-white images to 300 ppi above 450.
    #[default]
    Medium,
    /// Images to 120 ppi above 160, SSIM ≥ 0.970 and JPEG quality at least 65, black-and-white
    /// images as JBIG2, extras discarded.
    Strong,
}

impl Level {
    /// From the least to the most reduction.
    pub const ALL: [Level; 3] = [Level::Light, Level::Medium, Level::Strong];

    /// The name tools use.
    pub fn name(self) -> &'static str {
        match self {
            Level::Light => "light",
            Level::Medium => "medium",
            Level::Strong => "strong",
        }
    }

    pub fn from_name(name: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.name() == name)
    }

    /// The optimizer's choices for this level.
    pub fn settings(self) -> optimize::Settings {
        use optimize::{Compression, ImageSettings, MonoCompression, MonoSettings, Settings, Target};
        let base = Settings {
            merge_fragments: true,
            discard_thumbnails: true,
            discard_alternate_images: true,
            discard_tags: false,
            discard_print_settings: false,
            flate_unencoded: true,
            lzw_to_flate: true,
            recompress_streams: true,
            object_streams: true,
            remove_invalid_links: false,
            remove_unreferenced_dests: false,
            ..Settings::default()
        };
        let targeted = |target_ppi, above_ppi, min_ssim, floor, ceiling, min_saving, requantize| ImageSettings {
            downsample: true,
            target_ppi,
            above_ppi,
            compression: Compression::Targeted(Target { min_ssim, floor, ceiling, min_saving, grey: true, requantize }),
        };
        match self {
            Level::Light => {
                let keep = ImageSettings { downsample: false, compression: Compression::Lossless, ..ImageSettings::REDUCE };
                let mono = MonoSettings { downsample: false, ..MonoSettings::REDUCE };
                Settings { color: keep, gray: keep, mono, ..base }
            }
            Level::Medium => {
                let images = targeted(150.0, 220.0, 9850, 70, 92, 5, 2);
                Settings { color: images, gray: images, mono: MonoSettings::REDUCE, ..base }
            }
            Level::Strong => {
                let images = targeted(120.0, 160.0, 9700, 65, 90, 3, 3);
                Settings {
                    color: images,
                    gray: images,
                    mono: MonoSettings { compression: MonoCompression::Jbig2, ..MonoSettings::REDUCE },
                    discard_print_settings: true,
                    remove_invalid_links: true,
                    remove_unreferenced_dests: true,
                    ..base
                }
            }
        }
    }

    /// The Remove Hidden Information categories this level discards.
    pub fn discard(self) -> &'static [Hidden] {
        match self {
            Level::Light | Level::Medium => &[],
            Level::Strong => &[Hidden::Metadata, Hidden::PrivateData],
        }
    }
}

impl OptimizeJob {
    /// Optimize and write the copy. `progress` is called as each stage starts and before each
    /// image; returning `false` stops with [`EditError::Cancelled`]. A panic is returned as an
    /// error, so a worker thread always reports back.
    pub fn run(self, mut progress: impl FnMut(OptimizeStage) -> bool) -> Result<Optimized, EditError> {
        guard(move || self.work(&mut progress)).map_err(EditError::Optimize)?
    }

    fn work(self, progress: &mut dyn FnMut(OptimizeStage) -> bool) -> Result<Optimized, EditError> {
        let OptimizeJob { mut cos, settings, discard, opts } = self;
        // Like every file written, without the values of Password-flag fields (#3926, #3944).
        pdfcraft_forms::clear_password_values(&mut cos).map_err(|e| EditError::Write(e.to_string()))?;
        pdfcraft_xfa::clear_password_values(&mut cos).map_err(|e| EditError::Write(e.to_string()))?;
        let mut step = |stage| if progress(stage) { Ok(()) } else { Err(EditError::Cancelled) };
        let discarded = if discard.is_empty() {
            Vec::new()
        } else {
            step(OptimizeStage::Discarding)?;
            discard_hidden(&mut cos, &discard)?
        };
        let report = optimize::optimize_with_progress(&mut cos, &settings, &mut |s| step(stage(s)).is_ok()).map_err(optimize_error)?;
        step(OptimizeStage::Merging)?;
        let all: Vec<pdfcraft_cos::ObjRef> = cos.object_numbers().into_iter().map(|n| pdfcraft_cos::ObjRef::new(n, cos.generation(n))).collect();
        let merged = pdfcraft_organize::dedupe_resources(&mut cos, &all, false);
        step(OptimizeStage::Writing)?;
        let bytes = write_full(&cos, &opts).map_err(|e| EditError::Write(e.to_string()))?;
        Ok((Arc::new(bytes), OptimizeReport { optimize: report, merged, discarded }))
    }
}

/// Remove Hidden Information's `discard` categories (none: nothing to do).
fn discard_hidden(cos: &mut pdfcraft_cos::Document, discard: &[Hidden]) -> Result<Vec<(Hidden, usize)>, EditError> {
    if discard.is_empty() {
        return Ok(Vec::new());
    }
    // Every ticked category, as `remove_hidden` decides between them: with form fields ticked,
    // discarding hidden text also takes the hidden text the flattened appearances bring onto the
    // page, which no scan before flattening counts. It refuses when its scan counts every one as
    // 0, which would end the whole optimization, and may have changed the copy by then (an empty
    // outline root goes with bookmarks), so the copy from before is kept instead: nothing is
    // discarded.
    let before = cos.clone();
    match pdfcraft_redact::sanitize::remove_hidden(cos, discard) {
        Err(pdfcraft_redact::RedactError::NothingToApply) => {
            *cos = before;
            Ok(Vec::new())
        }
        r => Ok(r?),
    }
}

fn stage(s: optimize::Stage) -> OptimizeStage {
    match s {
        optimize::Stage::Reading { done, total } => OptimizeStage::Reading { done, total },
        optimize::Stage::Fragments => OptimizeStage::Fragments,
        optimize::Stage::Images { done, total } => OptimizeStage::Images { done, total },
        optimize::Stage::CleanUp => OptimizeStage::CleaningUp,
        optimize::Stage::Streams { done, total } => OptimizeStage::Streams { done, total },
    }
}

fn optimize_error(e: optimize::OptimizeError) -> EditError {
    match e {
        optimize::OptimizeError::Cancelled => EditError::Cancelled,
        e => EditError::Optimize(e.to_string()),
    }
}

/// What estimating Reduce File Size's levels needs, independent of the session.
pub struct EstimateJob {
    cos: pdfcraft_cos::Document,
    now: Option<String>,
    budget: optimize::Budget,
}

impl EstimateJob {
    /// Work within `budget` instead of the default (tests use it to compare estimates made from
    /// samples with the real result).
    pub fn with_budget(mut self, budget: optimize::Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Estimate the size of the copy each level would write, from the least reduction to the
    /// most, calling `found` with each as soon as it is known. `cancelled` is asked now and then;
    /// `true` stops with [`EditError::Cancelled`]. A panic is returned as an error.
    ///
    /// Each estimate is the copy really written with the streams the optimizer would change left
    /// empty, plus the sizes predicted for them (`pdfcraft_optimize::estimate_with_progress`).
    pub fn run(self, cancelled: impl Fn() -> bool, mut found: impl FnMut(Level, usize)) -> Result<(), EditError> {
        guard(move || self.work(&cancelled, &mut found)).map_err(EditError::Optimize)?
    }

    fn work(self, cancelled: &dyn Fn() -> bool, found: &mut dyn FnMut(Level, usize)) -> Result<(), EditError> {
        let mut cache = optimize::TrialCache::default();
        for level in Level::ALL {
            if cancelled() {
                return Err(EditError::Cancelled);
            }
            let bytes = self.estimate(level, &mut cache, cancelled)?;
            found(level, bytes);
        }
        Ok(())
    }

    /// One level's estimate.
    fn estimate(&self, level: Level, cache: &mut optimize::TrialCache, cancelled: &dyn Fn() -> bool) -> Result<usize, EditError> {
        let mut cos = self.cos.clone();
        let settings = level.settings();
        // As the optimization would: every file written goes without password field values.
        pdfcraft_forms::clear_password_values(&mut cos).map_err(|e| EditError::Write(e.to_string()))?;
        pdfcraft_xfa::clear_password_values(&mut cos).map_err(|e| EditError::Write(e.to_string()))?;
        discard_hidden(&mut cos, level.discard())?;
        let sizes = optimize::estimate_with_progress(&mut cos, &settings, &self.budget, cache, &mut |_| !cancelled()).map_err(optimize_error)?;
        if cancelled() {
            return Err(EditError::Cancelled);
        }
        let all: Vec<pdfcraft_cos::ObjRef> = cos.object_numbers().into_iter().map(|n| pdfcraft_cos::ObjRef::new(n, cos.generation(n))).collect();
        pdfcraft_organize::dedupe_resources(&mut cos, &all, false);
        let live = optimize::reachable(&cos);
        let mut predicted = 0usize;
        for (r, len) in sizes {
            if !live.contains(&r) {
                continue;
            }
            let empty = match &*cos.get(r) {
                Object::Stream(s) => Stream::from_raw(s.dict.clone(), Vec::new()),
                _ => continue,
            };
            cos.set(r, Object::Stream(empty));
            predicted = predicted.saturating_add(len);
        }
        let skeleton = write_full(&cos, &options(self.now.clone(), &settings)).map_err(|e| EditError::Write(e.to_string()))?;
        Ok(skeleton.len().saturating_add(predicted))
    }
}
