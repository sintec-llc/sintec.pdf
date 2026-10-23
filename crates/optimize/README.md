# pdfcraft-optimize

Layer L4: the PDF Optimizer (execution plan M11.1; Acrobat's Reduce File Size and Optimize PDF ▸
Advanced optimization).

```rust
let report = optimize(&mut doc, &Settings::default())?;   // Reduce File Size's choices
let report = optimize(&mut doc, &Preset::Mobile.settings())?; // smaller, for screens
// Or with progress: called before each image and the clean-up; `false` cancels.
let report = optimize_with_progress(&mut doc, &settings, &mut |stage| keep_going)?;
```

- **Presets:** Standard (Reduce File Size's choices: images above 225 ppi to 150 ppi, JPEG
  medium) and Mobile (above 144 ppi to 96 ppi, JPEG high), as in Acrobat's Settings menu.
- **Merge image fragments** (`fragments`, on by default): some PDF printers draw a page picture
  as thousands of images one pixel row high. Runs of upright 8-bit grey or RGB images in one
  colour space, sharing their left and right edges and stacked edge to edge (at least four,
  drawn with nothing else between them), become one lossless image at their finest resolution,
  which the image settings then resample; the strips are dropped by Clean Up. A real page
  printed this way went from 4.3 MB to 231 KB (Standard) and 161 KB (Mobile). Not merged yet:
  side-by-side tiles, rotated strips, masks.
- **Images:** the effective resolution of each image is measured where pages draw it (the CTM
  through form XObjects, up to 12 deep; the smallest over all uses). If a use can't be
  measured (forms nested deeper, or drawn too many times), no image is resampled. Colour and grayscale images above a
  threshold are resampled with Lanczos 3 to a target resolution and stored as JPEG (Acrobat's
  quality steps, or `Compression::Targeted`: the lowest quality keeping a luma SSIM target, see
  `quality.rs`) or ZIP; soft masks are resampled with their image (tent filter). Targeted keeps
  graphics (few colours, flat areas) lossless and makes JPEGs it doesn't resample smaller on
  their own coefficients (grey pages as luma alone, quantization steps multiplied);
  `Compression::Lossless` turns few-colour images into indexed palettes.
  An image is replaced only if the result is smaller. Left alone: CMYK and other colour spaces,
  image masks, decode arrays, more than 8 bits, JPEG 2000, JBIG2 and CCITT.
- **Discard objects:** thumbnails, alternate images, document tags, print settings.
- **Recompress streams** (lossless, `recompress_streams`, on by default): every stream in a
  general-purpose encoding (Flate, LZW, RunLength, ASCII85, ASCIIHex or none) is decoded and
  stored again with Flate at the best level; image samples (any colour space and bit depth,
  1-bit scans included) also try a PNG predictor. JPEGs are transcoded losslessly (`jpeg/`:
  Huffman tables made for the image, progressive when smaller, Exif/XMP dropped) and kept only
  when they decode to the same pixels. Kept only when smaller, counting the dictionary. Left
  alone: other image codecs, XMP, object and cross-reference streams, external data,
  `Crypt`, and attachments in encrypted files.
- **Estimate** (`estimate_with_progress`): the quick steps for real, the slow ones (images,
  stream recompression) tried on a budgeted sample and predicted for the rest; the engine turns
  this into Reduce File Size's per-level estimates (`docs/file-size-levels.md`).
- Images that weren't JPEG stay lossless (Flate with a predictor) when that is smaller than the
  JPEG; a JPEG re-encoded at its own size must save a tenth (fixed qualities) or the target's
  `min_saving` against its lossless transcode.
- **Clean up:** Flate for streams with no filter; for streams with an outer ASCII85 or ASCIIHex
  layer, that layer is removed when the result is smaller (the inner filters keep their encoded
  bytes). Streams with `/DecodeParms`, images and object streams are left alone.

The engine runs Remove Hidden Information (`pdfcraft-redact`) for the user-data categories,
then this, then merges identical objects and writes a full, compressed save
(`pdfcraft_engine::optimizer::OptimizeJob`, which the UI runs on a worker thread with a
progress bar). JPEG decoding (zune-jpeg) and resampling come from the `image` crate; JPEG
encoding and the coefficient-level transcoder are this crate's own (`src/jpeg/`, from T.81).
