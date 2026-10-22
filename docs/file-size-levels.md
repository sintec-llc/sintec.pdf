# Reduce File Size: Strong, Medium, Light

Status: shipped (task 024, 2026-10-22). The levels are defined once, in
`crates/engine/src/optimizer.rs` (`Level::settings`, `Level::discard`); this page repeats that
table for users and must change with it.

**File ▸ Reduce file size…** opens a small dialog with three levels. Next to each it shows about
how large the copy would be (`~1.2 MB (45% smaller)`), worked out on a worker thread while the
dialog is open ("calculating…" until then). **Reduce** writes the copy and a notice gives the
real sizes before and after; **Cancel** writes nothing. The open document is never changed: the
reduced copy is saved as a new file, and a copy that would be no smaller is not saved.

| | Light | Medium (default) | Strong |
|---|---|---|---|
| Images | unchanged (lossless only) | above 200 ppi resampled to 150 ppi (bicubic); stored as JPEG quality 75 | above 130 ppi resampled to 100 ppi; JPEG quality 55 |
| Streams | every stream in a general-purpose encoding stored again with Flate at the best level; image samples with PNG predictors | same | same |
| Structure | object streams and a cross-reference stream (400 objects per stream, best level), identical fonts and images merged, unused objects dropped | same | same |
| Discarded | page thumbnails, alternate images | same | same, plus print settings, document information and XMP metadata, other applications' private data (`/PieceInfo`), invalid links and unreferenced named destinations |
| Kept | text, fonts, forms, comments, attachments, bookmarks, links, tags, layers | same | same, except what is discarded |

Rules that hold at every level:

- An image's resolution is measured where the pages draw it (the smallest over all its uses);
  an image drawn somewhere that can't be measured is never resampled.
- Every image and stream is replaced only when the new one is smaller. A JPEG that isn't
  resampled is re-encoded only if that saves at least a tenth (a second generation of JPEG loss
  isn't worth less). An image that wasn't JPEG stays lossless (Flate with a predictor) when that
  is smaller than the JPEG would be, as it is for charts, screenshots and line art.
- 1-bit scans, CCITT, JBIG2 and JPEG 2000 images are never turned into JPEG; 1-bit and other
  Flate or uncompressed images get the lossless recompression.
- CMYK and other colour spaces, masks, decode arrays and 16-bit images keep their samples.
- Signed documents are refused: a full rewrite would invalidate their signatures. The dialog
  says so and offers no Reduce button.

## The estimate

The quick steps are really run on a copy (merging page pictures printed as strips, dropping
unused images, the discards). Then:

- the largest image of each kind (grey or colour, JPEG or not) is really resampled and encoded,
  and the next largest ones as long as a budget of 40 million source pixels lasts; the others
  are predicted from the bytes per output pixel their kind came to;
- the lossless recompression is really tried on the largest streams of each class (kind,
  compressed or not, bits per component) within 48 MB; the others are predicted from the ratio
  their class reached;
- the copy is written with those streams empty, and the predicted sizes are added.

Files that fit the budget are estimated within a few hundred bytes of the result (the
dictionaries the real run changes). In the tests (`crates/engine/tests/reduce_levels.rs`) every
estimate is within 0.5% of the real size, and within 0.5% even with no budget at all, but the
tests allow 5% (25% for a sample-only estimate): documents whose images differ a lot within one
kind are predicted less exactly. Automation: `doc_reduce` takes `level` (light, medium, strong;
default medium) and `doc_reduce_estimate` returns the same estimates as the dialog.

## Measured

Synthetic documents shaped like real ones (`reduce_levels.rs`; run it with `--nocapture`):

| Document | Original | Light | Medium | Strong |
|---|---|---|---|---|
| Photos: 3 × 1500×1000 at 300 ppi, two JPEG q92, one Flate | 4.46 MB | 3.26 MB (−27%) | 55 KB (−98.8%) | 25 KB (−99.4%) |
| Scans: 2 grey pages at 300 ppi Flate, one 1-bit page uncompressed | 2.78 MB | 2.52 MB (−9.5%) | 119 KB (−95.7%) | 43 KB (−98.5%) |
| Text: 12 pages, uncompressed content, 12 copies of one embedded font, thumbnails | 1.05 MB | 65 KB (−93.8%) | 65 KB | 65 KB |
| Report: text, a flat-colour chart stored uncompressed, one photo stored twice | 2.01 MB | 225 KB (−88.8%) | 77 KB (−96.1%) | 69 KB (−96.6%) |

The synthetic photos are smoother than real ones, so real photos shrink less at Medium and
Strong; the ordering (Strong < Medium < Light < original) is what the tests hold to.

## Not done yet

- Fonts are merged when identical but not subset, and unused glyphs stay.
- Unused fonts, colour spaces and patterns that a page lists but never uses stay (only images
  and forms are dropped).
- No JBIG2 or CCITT encoder: 1-bit scans get Flate with a predictor, which is good but larger
  than JBIG2.
- Zopfli-style Flate is not used (no pure-Rust dependency is in the tree); Flate level 9 is.

## Revision history

- 2026-10-22: first version (task 024).
