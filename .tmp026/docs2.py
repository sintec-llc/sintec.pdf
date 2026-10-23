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
old="""//!   again with Flate at the best level, image samples with a PNG predictor; a stream is
//!   replaced only when that is smaller. JPEG, JPEG 2000, JBIG2 and CCITT data stay as they are."""
assert old in s
s=s.replace(old,"""//!   again with Flate at the best level, image samples with a PNG predictor; JPEGs are transcoded losslessly (the same
//!   coefficients, better Huffman tables, no Exif); a stream is replaced only when that is
//!   smaller and, for a JPEG, decodes to the same pixels. JPEG 2000, JBIG2 and CCITT data stay
//!   as they are.""")
open(p,'w',encoding='utf-8',newline='\n').write(s)
