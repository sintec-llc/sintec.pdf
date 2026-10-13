//! Sintec.PDF: Scan into document. The scanner's Windows dialog can't run in tests, so the
//! "scanner" hands over pages queued in `scan_fakes`; everything after that is the real path.

use pdfcraft_engine::scan::ScanError;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

/// A small page as a scanner would return it: A4-ish proportions at 50 dpi.
fn scanned_page() -> Vec<u8> {
    let (w, h) = (413u32, 585u32);
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_pixel_dims(Some(png::PixelDimensions { xppu: 1969, yppu: 1969, unit: png::Unit::Meter }));
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&vec![230u8; (w * h) as usize]).unwrap();
    }
    png
}

fn app() -> PdfCraftApp {
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.run_inline = true;
    app
}

fn pages(app: &PdfCraftApp) -> usize {
    app.active_ids().and_then(|(_, id)| app.session.get(id)).map_or(0, |d| d.info.pages.len())
}

#[test]
fn scanned_pages_go_in_after_the_selection_one_after_another() {
    let mut app = app();
    let bytes = app.session.create_blank(612.0, 792.0, 3).unwrap();
    app.open_bytes("doc.pdf", None, bytes.as_ref().clone()).unwrap();
    let (i, id) = app.active_ids().unwrap();
    app.views[i].select_pages(&[0]);
    app.scan_fakes = vec![Ok(Some(scanned_page())), Ok(Some(scanned_page()))];
    app.execute("page.scan");
    assert_eq!(pages(&app), 4);
    assert_eq!(app.dialog, Some(Dialog::ScanMore), "offers the next page");
    // The scan is page 2, at the paper's size (50 dpi), not one point per pixel.
    let crop = app.session.get(id).unwrap().info.pages[1].crop;
    assert!(((crop[2] - crop[0]).abs() - 594.7).abs() < 1.0, "{crop:?}");
    // «Scan another page»: in after the first scan.
    app.dialog = None;
    app.scan_next();
    assert_eq!(pages(&app), 5);
    assert_eq!(app.scan_target, Some((id, 3)));
    assert_eq!(app.views[i].target_pages(), vec![2], "the new page is selected");
}

#[test]
fn a_scan_with_nothing_open_starts_a_new_document() {
    let mut app = app();
    app.scan_fakes = vec![Ok(Some(scanned_page()))];
    app.execute("page.scan");
    assert_eq!(pages(&app), 1);
    let (i, _) = app.active_ids().unwrap();
    assert!(app.views[i].organize, "in the page grid, ready for more");
}

#[test]
fn a_cancelled_or_failed_scan_changes_nothing() {
    let mut app = app();
    let bytes = app.session.create_blank(612.0, 792.0, 2).unwrap();
    app.open_bytes("doc.pdf", None, bytes.as_ref().clone()).unwrap();
    app.scan_fakes = vec![Ok(None), Err(ScanError::NoScanner)];
    app.execute("page.scan");
    assert_eq!((pages(&app), app.dialog), (2, None));
    app.execute("page.scan");
    assert_eq!((pages(&app), app.dialog), (2, None));
    assert!(app.scan_run.is_none());
}
