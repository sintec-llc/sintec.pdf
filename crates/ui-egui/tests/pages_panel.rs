//! The Pages panel: thumbnails as sharp as they are drawn, also in documents that mix small
//! pages with large drawing sheets; and its right-click menu (extract, rotate, blank pages).

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

fn mixed_sizes() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        let bytes = app.session.create_from_text("t", "A small page").unwrap();
        app.open_bytes("mixed.pdf", None, bytes.as_ref().clone()).unwrap();
        // An A1 drawing sheet after it (about 4× as wide).
        assert!(app.apply_edit(Edit::InsertBlankPage { at: 1, width: 2384.0, height: 1684.0 }));
        app.set_option("panel", "pages").unwrap();
        app
    });
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    let start = Instant::now();
    h.run_steps(4);
    while start.elapsed() < Duration::from_secs(20) {
        h.run_steps(2);
        if !h.state().render_pending() {
            h.run_steps(3);
            if !h.state().render_pending() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn small_pages_next_to_large_sheets_get_sharp_thumbnails() {
    let h = mixed_sizes();
    let ppp = h.ctx.pixels_per_point();
    let small = h.state().views[0].thumb(0).expect("the first page's thumbnail is rendered");
    // Drawn up to 200 points wide in the panel: rendered for that, not scaled by the A1 sheet
    // (which would make it ~30 pixels wide).
    assert!(small.size()[0] as f32 >= 200.0 * ppp * 0.85, "thumbnail is {:?} pixels", small.size());
    assert!(h.state().views[0].thumb(1).is_some(), "the sheet's thumbnail too");
}

#[test]
fn the_page_menu_rotates_and_inserts_blank_pages() {
    let mut h = mixed_sizes();
    let id = h.state().views[0].id;
    let pages = |h: &Harness<'static, PdfCraftApp>| h.state().session.get(id).unwrap().info.pages.len();
    assert_eq!(pages(&h), 2);

    h.get_by_label("Page 1").click_secondary();
    h.run_steps(2);
    h.get_by_label("Rotate clockwise").click();
    settle(&mut h);
    assert_eq!(h.state().session.get(id).unwrap().info.pages[0].rotation, 90);

    h.get_by_label("Page 1").click_secondary();
    h.run_steps(2);
    h.get_by_label("Insert blank page after").click();
    settle(&mut h);
    assert_eq!(pages(&h), 3);
    h.get_by_label("Page 1").click_secondary();
    h.run_steps(2);
    h.get_by_label("Insert blank page before").click();
    settle(&mut h);
    assert_eq!(pages(&h), 4);
    // The blank pages take the page's size; the original page is now the second.
    let info = &h.state().session.get(id).unwrap().info;
    assert_eq!(info.pages[1].rotation, 90);
    assert!((info.pages[0].width - info.pages[2].width).abs() < 1.0);

    // Extract opens Extract pages for that page, with "delete after extracting" as chosen.
    h.get_by_label("Page 2").click_secondary();
    h.run_steps(2);
    h.get_by_label("Extract and delete from file…").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, Some(pdfcraft_ui_egui::Dialog::Extract));
    assert!(h.state().extract_draft.delete);
    assert_eq!(h.state().views[0].target_pages(), [1]);
}
