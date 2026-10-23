# Reduce File Size: Strong, Medium, Light

Status: shipped (task 024, 2026-10-22; quality-targeted levels, task 026, 2026-10-23). The levels
are defined once, in `crates/engine/src/optimizer.rs` (`Level::settings`, `Level::discard`);
this page repeats that table for users and must change with it.

**File ▸ Reduce file size…** opens a small dialog with three levels. Next to each it shows about
how large the copy would be (`~1.2 MB (45% smaller)`), worked out on a worker thread while the
dialog is open ("calculating…" until then). **Reduce** writes the copy and a notice gives the
real sizes before and after; **Cancel** writes nothing. The open document is never changed: the
reduced copy is saved as a new file, and a copy that would be no smaller is not saved.

| | Light | Medium (default) | Strong |
|---|---|---|---|
| Promise | lossless: every pixel stays the same | visually lossless | smallest, still no obvious artifacts |
| Images | not resampled | above 220 ppi resampled to 150 ppi (Lanczos 3) | above 160 ppi resampled to 120 ppi |
| JPEG quality | — | per image: the lowest from 70 to 92 whose result keeps SSIM ≥ 0.985 | the lowest from 65 to 90 keeping SSIM ≥ 0.970 |
| JPEGs not resampled | transcoded losslessly: Huffman tables made for the image, progressive when smaller, Exif/XMP/Photoshop data and comments dropped | colour pages that are grey to the eye kept as their luma alone; quantization steps doubled where SSIM ≥ 0.985 holds; kept only if 5 % smaller than the lossless transcode | the same with steps up to tripled, SSIM ≥ 0.970, 3 % smaller |
| Graphics (screenshots, charts, line art: ≤ 256 colours, or at least half the pixels equal to their neighbour) | indexed palette (1, 2, 4 or 8 bits) when smaller than Flate | lossless as in Light; never JPEG, never resampled | same |
| Streams | every stream in a general-purpose encoding stored again with Flate at the best level; image samples with PNG predictors | same | same |
| Structure | object streams and a cross-reference stream (400 objects per stream, best level), identical fonts and images merged, unused objects dropped | same | same |
| Discarded | page thumbnails, alternate images | same | same, plus print settings, document information and XMP metadata, other applications' private data (`/PieceInfo`), invalid links and unreferenced named destinations |
| Kept | text, fonts, forms, comments, attachments, bookmarks, links, tags, layers | same | same, except what is discarded |

How the quality is chosen:

- **SSIM** (structural similarity, Wang et al. 2004) of the luma over 8×8 windows every 4
  pixels, counting only windows with detail (variance ≥ 25) when at least 1 in 100 has some: on
  a scanned page that measures the text, not the white paper, which JPEG keeps at any quality.
  The reference is the image as drawn before (resampled the same way when it is resampled).
- The quality is binary-searched; images over 2 megapixels search on a mosaic of up to 16×16
  tiles of 64 pixels, then the whole image is checked (and the quality raised if it misses).
  Grain (sensor noise, paper texture) can miss the target even at the ceiling quality; such an
  image is stored at the ceiling (92 or 90), which looks the same, and comes within about 0.01.
- **Chroma:** 4:2:0 when halving the colour resolution keeps the colours (photos, scans), 4:4:4
  when it would bleed sharp coloured edges (coloured text).
- **Grey:** a DeviceRGB image is stored as grey only when no pixel is more than slightly tinted
  (channels at most 24 apart, and at most 1 in 1000 pixels more than 8 apart): a blue stamp,
  signature or highlight keeps the page in colour.
- **JPEGs not resampled** are never decoded and re-encoded from pixels: a second DCT
  generation gains nothing over the lossless transcode (re-encoding a quality 75 scan at 75 is
  12 % smaller, the transcode 13 %) and lower qualities that don't line up with the original
  tables lose more than they save. Instead the original coefficients are divided by whole
  factors (the DC steps stay, so flat areas don't band), and a grey YCbCr page keeps its luma
  component exactly.

Rules that hold at every level:

- An image's resolution is measured where the pages draw it (the smallest over all its uses);
  an image drawn somewhere that can't be measured is never resampled. Images smaller than 64×64
  are left alone. A soft mask is resampled with its image (a tent filter, no halo).
- Every image and stream is replaced only when the new one is smaller; a lossless JPEG transcode
  is kept only when it decodes to exactly the same pixels.
- 1-bit scans, CCITT, JBIG2 and JPEG 2000 images are never turned into JPEG; 1-bit and other
  Flate or uncompressed images get the lossless recompression.
- CMYK and other colour spaces, masks, decode arrays and 16-bit images keep their samples
  (a CMYK JPEG still gets the lossless transcode).
- Progressive and arithmetic-coded JPEGs aren't transcoded (they are re-encoded from pixels when
  resampled).
- Signed documents are refused: a full rewrite would invalidate their signatures. The dialog
  says so and offers no Reduce button.

## The estimate

The quick steps are really run on a copy (merging page pictures printed as strips, dropping
unused images, the discards). Then:

- the largest image of each kind (grey or colour, JPEG or not, resampled or not) is really
  encoded, quality search included, and the next largest ones (images of one size spread evenly
  over the document) as long as a budget of 24 million source pixels lasts; the others are
  predicted from their kind: resampled ones from the bytes per output pixel, the others from
  the share of their size the trials kept, weighted by how many trials were replaced;
- the lossless recompression (JPEG transcodes included) is really tried on the largest streams
  of each class (kind, compressed or not, bits per component) within 48 MB; the others are
  predicted from the ratio their class reached;
- the copy is written with those streams empty, and the predicted sizes are added.

Files that fit the budget are estimated within a few hundred bytes of the result. The tests
(`crates/engine/tests/reduce_levels.rs`) allow 5 % (25 % for a sample-only estimate); the
60-page scan below is estimated within 1 % at Medium and 8 % at Strong. Automation:
`doc_reduce` takes `level` (light, medium, strong; default medium) and `doc_reduce_estimate`
returns the same estimates as the dialog.

## Measured

Real files (not in the repository), before task 026 → now:

| Document | Original | Light | Medium | Strong |
|---|---|---|---|---|
| Scanned act, 61 colour JPEG pages at 200 ppi (q≈75, 4:2:0) | 25.2 MB | 25.2 MB → 21.9 MB (−13.4 %) | 25.2 MB → 17.5 MB (−30.6 %) | 6.8 MB at 100 ppi q55 → 9.0 MB at 120 ppi (−64.4 %) |
| Letter with logos (JPEGs carrying 13 KB of Exif/Photoshop data) | 85 KB | 84 KB → 47 KB (−44 %) | 37 KB → 44 KB (−49 %) | 32 KB → 38 KB (−55 %) |
| Invoice (indexed images) | 102 KB | 96 KB (−6 %) | 96 KB | 94 KB (−8 %) |

Before, Light and Medium were the same on the scan: its pages are at exactly 200 ppi, Medium
only resampled above 200, and re-encoding a JPEG had to save a tenth, which the old encoder
(4:4:4, standard Huffman tables) never did. Strong was smaller before only because it cut the
pages to 100 ppi at quality 55.

Synthetic pictures, 4 × 3 inches (`photos_scans_screenshots_and_text_on_images_at_four_resolutions`;
SSIM against the original as drawn, resampled alike):

| Picture | ppi | Original | Light | Medium | Strong |
|---|---|---|---|---|---|
| photo (JPEG q92, grainy) | 96 | 27.5 KB | 23.9 KB (SSIM 1) | 23.9 KB | 22.8 KB (0.971) |
| photo | 300 | 231 KB | 198 KB (1) | 23.6 KB at 150 ppi (0.983) | 5.6 KB at 120 ppi (0.983) |
| scan (grey JPEG q85) | 200 | 74.2 KB | 65.1 KB (1) | 65.1 KB | 8.4 KB at 120 ppi (0.992) |
| scan | 300 | 189 KB | 170 KB (1) | 40.8 KB at 150 ppi (0.986) | 12.3 KB (0.987) |
| screenshot (Flate) | 300 | 14.5 KB | 3.3 KB palette (1) | 3.3 KB | 3.3 KB |
| text on a photo (JPEG q90) | 150 | 71.7 KB | 67.1 KB (1) | 60.9 KB (0.991) | 31.0 KB (0.988) |
| text on a photo | 300 | 269 KB | 251 KB (1) | 43.8 KB at 150 ppi (0.986) | 15.4 KB at 120 ppi (0.975) |

Grainy JPEGs that aren't resampled (the synthetic photo and scan at 96–200 ppi) keep Light's
size at Medium: halving their quantization steps would smooth the grain below the SSIM target,
so the target wins.

## Not done yet

- Fonts are merged when identical but not subset, and unused glyphs stay.
- Unused fonts, colour spaces and patterns that a page lists but never uses stay (only images
  and forms are dropped).
- No JBIG2 or CCITT encoder: 1-bit scans get Flate with a predictor, which is good but larger
  than JBIG2.
- No trellis quantization or successive-approximation progressive scans (mozjpeg's further few
  percent); progressive JPEGs in the source aren't transcoded.
- Zopfli-style Flate is not used: no pure-Rust implementation is in the tree, and it is about a
  hundred times slower than Flate level 9, which is used.
- The SSIM is luma only; colour loss is limited by the chroma choice and the quality floor.

## Revision history

- 2026-10-23: quality-targeted levels, lossless JPEG transcode, palettes, grey pages,
  coefficient requantization; measurements (task 026).
- 2026-10-22: first version (task 024).
