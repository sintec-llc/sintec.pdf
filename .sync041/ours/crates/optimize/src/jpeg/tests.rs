use super::*;

/// A deterministic photo-like RGB or grey image: gradients, a blob and grain.
fn photo(w: u32, h: u32, n: usize) -> Vec<u8> {
    let mut seed = 0x2545_f491_u32;
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for c in 0..n {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let v = 60.0 + 120.0 * ((x as f32 / 23.0 + c as f32).sin() * (y as f32 / 17.0).cos()).abs() + (seed % 9) as f32;
                px.push(v.clamp(0.0, 255.0) as u8);
            }
        }
    }
    px
}

fn image_crate_jpeg(px: &[u8], w: u32, h: u32, n: usize, q: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let ty = if n == 1 { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q).encode(px, w, h, ty).unwrap();
    out
}

fn decoded(jpeg: &[u8]) -> image::DynamicImage {
    image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap()
}

#[test]
fn a_standard_table_jpeg_is_transcoded_smaller_and_pixel_identical() {
    for (n, w, h) in [(3usize, 203u32, 117u32), (1, 64, 64), (3, 8, 8), (1, 1, 1)] {
        let src = image_crate_jpeg(&photo(w, h, n), w, h, n, 85);
        let j = read(&src).expect("baseline JPEG reads");
        assert_eq!((j.width, j.height, j.components.len()), (w as u16, h as u16, n));
        // Baseline, progressive and the smaller of both all keep the coefficients.
        for layout in [Some(false), Some(true), None] {
            let out = write::write_as(&j, layout).unwrap();
            assert_eq!(decoded(&out).as_bytes(), decoded(&src).as_bytes(), "{n} {w}x{h} {layout:?}");
            if layout == Some(false) {
                assert_eq!(read(&out).unwrap(), j, "baseline reads back to the same coefficients");
            }
        }
        if w * h > 1000 {
            let t = transcode(&src).expect("smaller");
            assert!(t.len() < src.len() * 97 / 100, "{n}: {} from {}", t.len(), src.len());
        }
    }
}

#[test]
fn the_encoder_writes_420_and_444_jpegs_that_read_back() {
    let (w, h) = (150u32, 97u32);
    let px = photo(w, h, 3);
    for sub in [true, false] {
        let jpeg = encode(&px, 3, w, h, 80, sub).unwrap();
        let img = decoded(&jpeg).into_rgb8();
        assert_eq!(img.dimensions(), (w, h));
        let err: f64 = img.as_raw().iter().zip(&px).map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs()).sum::<f64>() / px.len() as f64;
        assert!(err < 6.0, "mean error {err}");
        assert_eq!(luma_sampling(&jpeg), Some(if sub { 0x22 } else { 0x11 }));
    }
    // Higher quality, larger file.
    assert!(encode(&px, 3, w, h, 50, true).unwrap().len() < encode(&px, 3, w, h, 90, true).unwrap().len());
    // Bad input: nothing, no panic.
    assert!(encode(&px, 3, w, h + 1, 80, true).is_none());
    assert!(encode(&px, 2, w, h, 80, true).is_none());
    assert!(encode(&[], 1, 0, 0, 80, true).is_none());
}

/// The luma sampling factors in the frame header (baseline or progressive).
fn luma_sampling(jpeg: &[u8]) -> Option<u8> {
    let at = jpeg.windows(2).position(|m| m == [0xFF, 0xC0] || m == [0xFF, 0xC2])?;
    jpeg.get(at + 11).copied()
}

#[test]
fn metadata_segments_are_dropped_and_the_colour_transform_kept() {
    let (w, h) = (64u32, 48u32);
    let src = image_crate_jpeg(&photo(w, h, 3), w, h, 3, 90);
    // Exif (APP1), Photoshop (APP13), a comment and Adobe's APP14 after SOI.
    let mut with = vec![0xFF, 0xD8];
    for (m, body) in [(0xE1u8, vec![b'E'; 3000]), (0xED, vec![b'P'; 500]), (0xFE, b"hello".to_vec()), (0xEE, b"Adobe\0\x64\0\0\0\0\x01".to_vec())] {
        with.extend([0xFF, m]);
        with.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        with.extend(body);
    }
    with.extend(&src[2..]);
    let j = read(&with).unwrap();
    assert_eq!(j.markers.iter().map(|(m, _)| *m).collect::<Vec<_>>(), [0xEE, 0xE0]);
    let t = transcode(&with).unwrap();
    assert!(t.len() + 3500 < with.len());
}

#[test]
fn damaged_and_unsupported_jpegs_are_refused_without_panicking() {
    let (w, h) = (40u32, 24u32);
    let src = image_crate_jpeg(&photo(w, h, 3), w, h, 3, 75);
    for cut in [0, 1, 2, 10, src.len() / 2, src.len() - 3] {
        assert!(read(&src[..cut]).is_none(), "cut at {cut}");
    }
    let mut seed = 7u32;
    for i in 0..400 {
        let mut bad = src.clone();
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let at = (seed as usize >> 8) % bad.len();
        bad[at] ^= 1 << (i % 8);
        // Whatever it makes of it, it doesn't panic, and a transcode is only ever exact.
        if let Some(t) = transcode(&bad) {
            assert_eq!(decoded(&t).as_bytes(), decoded(&bad).as_bytes());
        }
    }
    // Progressive (SOF2) isn't read.
    let prog = write::write_as(&read(&src).unwrap(), Some(true)).unwrap();
    assert!(read(&prog).is_none());
    assert!(transcode(&prog).is_none());
}
