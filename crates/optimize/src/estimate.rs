//! Estimating what an optimization would make of a document before running it.
//!
//! [`estimate_with_progress`] runs the quick steps for real (merging image fragments, dropping
//! unused images, the Discard Objects choices, the link clean-up) and predicts the slow ones:
//!
//! - **Images:** the image settings are planned for every image without touching its pixels
//!   (which are resampled, and to what size). The largest image of each kind (black-and-white,
//!   grey or colour, JPEG or not, resampled or not) is then really worked out as the optimizer
//!   would (the same decision, quality search and monochrome step included), and the next largest
//!   as long as the [`Budget`]'s pixels last. The others are
//!   predicted from their kind's trials: resampled ones from the bytes per output pixel, the
//!   others from the share of their size the trials kept, weighted by how many trials were
//!   replaced at all; an image is replaced only if that is smaller, as the optimizer would.
//! - **Streams:** the lossless recompression is really tried on the largest streams of each kind
//!   (image samples, other compressed streams, unencoded streams) within the budget's bytes; the
//!   others are predicted from the ratio their class reached (kind, compressed or not, bits per
//!   component). [`TrialCache`] keeps the results,
//!   so estimating several levels of one document does the lossless trials once.
//!
//! The result is the stored size of every stream the optimizer would change; the caller writes
//! the document with those streams emptied and adds the sizes (the engine does this, after
//! merging identical objects as the real run does). Small documents fit the budget whole, so
//! their estimate is the real result give or take a few bytes of dictionaries.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use pdfcraft_cos::{Document, ImageBudget, ObjRef, Object, serialize};

use crate::images::{self, Plan};
use crate::recompress::{self, Kind};
use crate::{MonoCompression, OptimizeError, Report, Settings, Stage};

/// How much real work an estimate may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Source pixels of the images trial-encoded (beyond the largest image of each kind, which
    /// is always tried).
    pub image_pixels: u64,
    /// Stored bytes of the streams trial-recompressed (beyond the smallest of each kind, which
    /// is tried if nothing else of its kind fits).
    pub stream_bytes: u64,
}

impl Default for Budget {
    /// About a second or two of work on an ordinary machine.
    fn default() -> Self {
        Budget { image_pixels: 24_000_000, stream_bytes: 48 << 20 }
    }
}

/// The predicted stored data length (bytes after the filters) of every stream the optimizer
/// would change.
pub type StreamSizes = HashMap<ObjRef, usize>;

/// Results of lossless trials, by stream content: shared by the estimates of one document's
/// levels, which recompress the same streams.
#[derive(Default)]
pub struct TrialCache {
    sizes: HashMap<u64, Option<usize>>,
}

/// The objects a full save writes: everything reachable from the trailer.
pub fn reachable(doc: &Document) -> HashSet<ObjRef> {
    let mut seen = HashSet::new();
    let mut stack: Vec<Object> = doc.trailer().iter().map(|(_, v)| v.clone()).collect();
    while let Some(obj) = stack.pop() {
        match obj {
            Object::Ref(r) => {
                if !seen.insert(r) {
                    continue;
                }
                match &*doc.get(r) {
                    Object::Dict(d) => stack.extend(d.iter().map(|(_, v)| v.clone())),
                    Object::Stream(s) => stack.extend(s.dict.iter().map(|(_, v)| v.clone())),
                    Object::Array(a) => stack.extend(a.iter().cloned()),
                    _ => {}
                }
            }
            Object::Dict(d) => stack.extend(d.iter().map(|(_, v)| v.clone())),
            Object::Array(a) => stack.extend(a),
            _ => {}
        }
    }
    seen
}

/// Run the quick steps of optimizing `doc` with `settings` and predict the slow ones (see the
/// module documentation). `doc` is left with the quick steps done and its images and streams
/// as they were. `progress` is called as an optimization's would be; `false` cancels.
pub fn estimate_with_progress(
    doc: &mut Document,
    settings: &Settings,
    budget: &Budget,
    cache: &mut TrialCache,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<StreamSizes, OptimizeError> {
    let pages = pdfcraft_annot::page_refs(doc).map_err(|_| OptimizeError::NoPages)?;
    let mut report = Report::default();
    // The run's own allowance, as the optimizer would have it: an estimate refuses what it refuses.
    let mut allowance = ImageBudget::new(doc);
    if settings.merge_fragments {
        crate::fragments::run(doc, &pages, &mut report, progress, &mut allowance)?;
    }
    let mut sizes = estimate_images(doc, &pages, settings, budget, &mut allowance, progress)?;
    crate::clean_up(doc, &pages, settings, &mut report, progress)?;
    estimate_streams(doc, settings, budget, cache, &mut sizes, &mut allowance, progress)?;
    Ok(sizes)
}

/// One image the settings may change.
struct Item {
    r: ObjRef,
    ppi: f64,
    /// Its pixels, and how many it would have once resampled (as many when it isn't).
    source_px: u64,
    out_px: f64,
    /// Whether the resampled size predicts its bytes (grey and colour images resampled).
    by_pixels: bool,
    raw: usize,
    class: Class,
    /// The soft mask, its stored size and its pixel count, when the image is resampled.
    mask: Option<(ObjRef, usize, u64)>,
}

/// Images of one kind: colour components (0 for black-and-white images), JPEG or not, resampled
/// or not.
type Class = (usize, bool, bool);

#[derive(Default)]
struct ClassStats {
    tried: usize,
    replaced: usize,
    /// Of the images replaced: new bytes, their stored bytes before, and their output pixels.
    bytes: f64,
    before: f64,
    pixels: f64,
}

/// The image settings' item for picture `r` at `ppi`, if they may change it.
fn item(doc: &Document, r: ObjRef, ppi: f64, settings: &Settings) -> Option<Item> {
    let obj = doc.get(r);
    let Object::Stream(s) = &*obj else { return None };
    let side = |k: &[u8]| s.dict.int(k).and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    if images::one_bit_image(s) {
        let mono = &settings.mono;
        if mono.compression == MonoCompression::Retain && !mono.downsample {
            return None;
        }
        let (w, h) = (side(b"Width")?, side(b"Height")?);
        let source_px = u64::from(w) * u64::from(h);
        let resized = mono.downsample && ppi.is_finite() && ppi > mono.above_ppi && mono.target_ppi > 0.0;
        let k = if resized { (mono.target_ppi / ppi).min(1.0) } else { 1.0 };
        let out_px = source_px as f64 * k * k;
        return Some(Item { r, ppi, source_px, out_px, by_pixels: false, raw: s.raw.len(), class: (0, false, resized), mask: None });
    }
    let n = s.dict.get(b"ColorSpace").and_then(|cs| images::components(doc, cs))?;
    let st = if n == 1 { settings.gray } else { settings.color };
    let plan: Plan = images::plan(doc, s, ppi, &st, n)?;
    let mask = s.dict.reference(b"SMask").filter(|_| plan.resized()).and_then(|mr| match &*doc.get(mr) {
        Object::Stream(m) => {
            let px = (m.dict.int(b"Width")?).checked_mul(m.dict.int(b"Height")?).and_then(|p| u64::try_from(p).ok())?;
            Some((mr, m.raw.len(), px.max(1)))
        }
        _ => None,
    });
    Some(Item {
        r,
        ppi,
        source_px: u64::from(plan.w) * u64::from(plan.h),
        out_px: f64::from(plan.nw) * f64::from(plan.nh),
        by_pixels: plan.resized(),
        raw: s.raw.len(),
        class: (plan.n, plan.was_dct, plan.resized()),
        mask,
    })
}

fn estimate_images(
    doc: &Document,
    pages: &[ObjRef],
    settings: &Settings,
    budget: &Budget,
    allowance: &mut ImageBudget,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<StreamSizes, OptimizeError> {
    let (drawn, defaults) = images::drawn(doc, pages, settings, &mut |done| progress(Stage::Reading { done, total: pages.len() }))?;
    let items: Vec<Item> = drawn.into_iter().filter_map(|(r, ppi)| item(doc, r, ppi, settings)).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    // Largest first; images of one size (the pages of a scan) in an order spread evenly over
    // the document, so a sample of them stands for all.
    let spread = |i: usize| (i as u32).reverse_bits();
    order.sort_by_key(|i| (std::cmp::Reverse(items.get(*i).map_or(0, |i| i.source_px)), spread(*i)));
    let mut chosen = vec![false; items.len()];
    let (mut classes, mut spent) = (HashSet::new(), 0u64);
    for i in &order {
        if let (Some(item), Some(c)) = (items.get(*i), chosen.get_mut(*i))
            && classes.insert(item.class)
        {
            *c = true;
            spent = spent.saturating_add(item.source_px);
        }
    }
    for i in &order {
        if let (Some(item), Some(c)) = (items.get(*i), chosen.get_mut(*i))
            && !*c
            && spent.saturating_add(item.source_px) <= budget.image_pixels
        {
            *c = true;
            spent = spent.saturating_add(item.source_px);
        }
    }
    let total = chosen.iter().filter(|c| **c).count();
    let mut stats: HashMap<Class, ClassStats> = HashMap::new();
    let mut sizes = StreamSizes::new();
    for (done, (item, _)) in items.iter().zip(&chosen).filter(|(_, c)| **c).enumerate() {
        if !progress(Stage::Images { done, total }) {
            return Err(OptimizeError::Cancelled);
        }
        let st = stats.entry(item.class).or_default();
        st.tried += 1;
        if let Some((new, smask)) = images::trial(doc, item.r, item.ppi, defaults.contains(&item.r.num), settings, allowance) {
            st.replaced += 1;
            st.bytes += new.raw.len() as f64;
            st.before += item.raw as f64;
            st.pixels += item.out_px;
            sizes.insert(item.r, new.raw.len());
            if let Some((mr, m)) = smask {
                sizes.insert(mr, m.raw.len());
            }
        }
    }
    if !progress(Stage::Images { done: total, total }) {
        return Err(OptimizeError::Cancelled);
    }
    for (item, _) in items.iter().zip(&chosen).filter(|(_, c)| !**c) {
        // Every kind had a trial; one whose trials were never replaced isn't replaced.
        let Some(st) = stats.get(&item.class).filter(|s| s.replaced > 0 && s.pixels > 0.0 && s.before > 0.0) else { continue };
        // Resampled images come to about the same bytes per pixel as their kind's trials; the
        // others to about the same share of their own size (re-encoding depends on content).
        let replaced = if item.by_pixels { st.bytes / st.pixels * item.out_px } else { item.raw as f64 * st.bytes / st.before };
        // Trials of the kind that weren't replaced stand for images that keep their size.
        let share = st.replaced as f64 / st.tried.max(1) as f64;
        let est = share * replaced + (1.0 - share) * item.raw as f64;
        // A soft mask is resampled losslessly with its image: about in proportion to pixels.
        let mask_est = item.mask.map(|(_, raw, px)| raw as f64 * item.out_px / px as f64);
        let before = item.raw as f64 + item.mask.map_or(0.0, |(_, raw, _)| raw as f64);
        let after = est + mask_est.unwrap_or(0.0);
        if after < before && est.is_finite() {
            // Finite, non-negative and below `before`, a `usize`: the casts are exact enough.
            sizes.insert(item.r, est.round() as usize);
            if let (Some((mr, _, _)), Some(m)) = (item.mask, mask_est) {
                sizes.insert(mr, m.round() as usize);
            }
        }
    }
    Ok(sizes)
}

/// A key for a stream's content: its dictionary and data.
fn content_key(s: &pdfcraft_cos::Stream) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut dict = Vec::new();
    serialize(&Object::Dict(s.dict.clone()), &mut dict);
    dict.hash(&mut h);
    s.raw.hash(&mut h);
    h.finish()
}

/// Streams that recompress alike: their kind, whether they are compressed already, and for
/// images the bits per component (1-bit scans pack very differently from photos).
type StreamClass = (Kind, bool, u8);

fn estimate_streams(
    doc: &Document,
    settings: &Settings,
    budget: &Budget,
    cache: &mut TrialCache,
    sizes: &mut StreamSizes,
    allowance: &mut ImageBudget,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<(), OptimizeError> {
    if !settings.recompress_streams && !settings.flate_unencoded {
        return Ok(());
    }
    let encrypted = doc.trailer().contains(b"Encrypt");
    let mut live: Vec<ObjRef> = reachable(doc).into_iter().collect();
    live.sort_by_key(|r| r.num);
    // (stream, stored bytes, class)
    let mut candidates: Vec<(ObjRef, usize, StreamClass)> = Vec::new();
    for r in live {
        if sizes.contains_key(&r) {
            continue;
        }
        let obj = doc.get(r);
        let Object::Stream(s) = &*obj else { continue };
        match recompress::eligible(s, encrypted) {
            Some(kind) if settings.recompress_streams => {
                let bits = if kind == Kind::Image { s.dict.int(b"BitsPerComponent").unwrap_or(0).clamp(0, 16) as u8 } else { 0 };
                candidates.push((r, s.raw.len(), (kind, s.dict.contains(b"Filter"), bits)));
            }
            // Without recompression, the plain Flate pass is quick enough to run as it is.
            _ if settings.flate_unencoded => {
                if let Some(new) = crate::flate_unencoded(s) {
                    sizes.insert(r, new.raw.len());
                }
            }
            _ => {}
        }
    }
    candidates.sort_by_key(|(r, raw, _)| (std::cmp::Reverse(*raw), r.num));
    let mut chosen = vec![false; candidates.len()];
    let mut spent = 0u64;
    let mut kinds = HashSet::new();
    for ((_, raw, kind), c) in candidates.iter().zip(chosen.iter_mut()) {
        let raw = *raw as u64;
        if spent.saturating_add(raw) <= budget.stream_bytes {
            *c = true;
            spent = spent.saturating_add(raw);
            kinds.insert(*kind);
        }
    }
    // A class nothing fitted for gets its smallest stream tried, so it has a ratio.
    for ((_, _, kind), c) in candidates.iter().zip(chosen.iter_mut()).rev() {
        if kinds.insert(*kind) {
            *c = true;
        }
    }
    let total = chosen.iter().filter(|c| **c).count();
    let mut ratio: HashMap<StreamClass, (f64, f64)> = HashMap::new();
    for (done, ((r, raw, kind), _)) in candidates.iter().zip(&chosen).filter(|(_, c)| **c).enumerate() {
        if !progress(Stage::Streams { done, total }) {
            return Err(OptimizeError::Cancelled);
        }
        let obj = doc.get(*r);
        let Object::Stream(s) = &*obj else { continue };
        let key = content_key(s);
        let new = match cache.sizes.get(&key) {
            Some(known) => *known,
            None => {
                let new = recompress::recompress(doc, s, allowance).map(|n| n.raw.len());
                cache.sizes.insert(key, new);
                new
            }
        };
        let e = ratio.entry(*kind).or_default();
        e.0 += *raw as f64;
        e.1 += new.unwrap_or(*raw) as f64;
        if let Some(new) = new {
            sizes.insert(*r, new);
        }
    }
    if !progress(Stage::Streams { done: total, total }) {
        return Err(OptimizeError::Cancelled);
    }
    for ((r, raw, kind), _) in candidates.iter().zip(&chosen).filter(|(_, c)| !**c) {
        let Some((before, after)) = ratio.get(kind).filter(|(b, _)| *b > 0.0) else { continue };
        let est = *raw as f64 * after / before;
        if est.is_finite() && est < *raw as f64 {
            // Finite and between 0 and `raw`: the cast is in range.
            sizes.insert(*r, est.round() as usize);
        }
    }
    Ok(())
}
