p='crates/engine/src/optimizer.rs'
s=open(p,encoding='utf-8').read()
a=s.index("//! | | Light | Medium | Strong |")
b=s.index("use std::sync::Arc;")
s=s[:a]+"""//! | | Light | Medium | Strong |
//! |---|---|---|---|
//! | Images | lossless only: every pixel stays the same | above 220 ppi resampled to 150 ppi (Lanczos); JPEG quality searched per image for SSIM ≥ 0.985 (quality 70–92) | above 160 ppi resampled to 120 ppi; JPEG quality searched for SSIM ≥ 0.970 (quality 65–90) |
//! | JPEGs not resampled | transcoded losslessly (Huffman tables made for the image, progressive when smaller, Exif/XMP/comments dropped) | grey-looking colour pages stored as their luma alone; quantization steps doubled where SSIM ≥ 0.985 holds; kept only if 5 % smaller than the lossless transcode | as Medium, steps up to tripled, SSIM ≥ 0.970, 3 % smaller |
//! | Graphics (≤ 256 colours, or ≥ half the pixels equal to their neighbour) | indexed palette (1–8 bits) when smaller | same: never JPEG, never resampled | same |
//! | Streams | Flate at the best level, PNG predictors for image samples | same | same |
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

"""+s[b:]
s=s.replace("""    /// Lossless only: streams recompressed, structure compacted, thumbnails dropped.
    Light,
    /// Images to 150 ppi, JPEG quality 75.
    #[default]
    Medium,
    /// Images to 100 ppi, JPEG quality 55, extras discarded.
    Strong,""","""    /// Lossless only: every pixel stays; JPEGs transcoded, streams recompressed, structure
    /// compacted, thumbnails dropped.
    Light,
    /// Visually lossless: images to 150 ppi above 220, JPEG quality chosen per image for
    /// SSIM ≥ 0.985.
    #[default]
    Medium,
    /// Images to 120 ppi above 160, SSIM ≥ 0.970 and JPEG quality at least 65, extras discarded.
    Strong,""")
open(p,'w',encoding='utf-8',newline='\n').write(s)

p='crates/automation/src/tools.rs'
s=open(p,encoding='utf-8').read()
a=s.index('"Write a smaller copy of the document to `path` at `level` (default medium).')
b=s.index('doc_reduce_estimate predicts each level\'s size first.")')
s=s[:a]+'"Write a smaller copy of the document to `path` at `level` (default medium). light: lossless only, every pixel stays the same (JPEGs transcoded losslessly with Huffman tables made for them, progressive when smaller, Exif/XMP dropped; images with few colours stored as indexed palettes; every other stream recompressed with Flate at the best level, image samples with PNG predictors; thumbnails and alternate images dropped). medium: visually lossless: images above 220 ppi resampled to 150 ppi; the JPEG quality chosen per image as the lowest keeping SSIM >= 0.985 (quality 70-92); JPEGs not resampled are made smaller on their own coefficients (grey-looking colour pages as grey, quantization steps doubled while SSIM holds); screenshots, charts and line art stay lossless. strong: images above 160 ppi to 120 ppi, SSIM >= 0.97 (quality 65-90), and print settings, document information and XMP metadata, other applications\' private data, invalid links and unreferenced named destinations discarded. At every level identical fonts and images are merged, unused objects dropped and objects packed in compressed object streams; an image or stream is replaced only when smaller, and text, fonts, forms, comments, attachments, bookmarks, links and tags are kept. Signed documents are refused. If the copy wouldn\'t be smaller, nothing is written and `written` is false. The open document is unchanged. '+s[b:]
open(p,'w',encoding='utf-8',newline='\n').write(s)

p='crates/optimize/src/lib.rs'
s=open(p,encoding='utf-8').read()
old="""//!   threshold are resampled (bicubic) to a target resolution and recompressed as JPEG or Flate.
//!   A new image replaces the old one only if it is smaller."""
assert old in s
s=s.replace(old,"""//!   threshold are resampled (Lanczos) to a target resolution and recompressed as JPEG or Flate:
//!   at a fixed quality, or ([`Compression::Targeted`]) at the lowest quality that keeps the
//!   image's SSIM, graphics staying lossless and JPEGs that aren't resampled made smaller on
//!   their own coefficients ([`Compression::Lossless`] only converts few-colour images to an
//!   indexed palette). JPEG is written by PdfCraft's own encoder (4:2:0 chroma when the colours
//!   allow it, Huffman tables made for each image). A new image replaces the old one only if it
//!   is smaller.""")
old="""//!   level, image samples with a PNG predictor; a stream is replaced only when that is smaller.
//!   JPEG, JPEG 2000, JBIG2 and CCITT data stay as they are."""
assert old in s
s=s.replace(old,"""//!   level, image samples with a PNG predictor; JPEGs are transcoded losslessly (the same
//!   coefficients, better Huffman tables, no Exif); a stream is replaced only when that is
//!   smaller and, for a JPEG, decodes to the same pixels. JPEG 2000, JBIG2 and CCITT data stay
//!   as they are.""")
open(p,'w',encoding='utf-8',newline='\n').write(s)
