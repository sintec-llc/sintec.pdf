//! Explorer ▸ «Преобразовать в PDF» in the real shell: the converted PDF opens unsaved in the page
//! grid with a review banner, nothing is written next to the files, and Continue goes on to the
//! reader.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

#[test]
fn converted_files_open_unsaved_for_review_then_continue_to_the_reader() {
    let dir = std::env::temp_dir().join(format!("sintec-convert-ui-{}", std::process::id())).join("Счета");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A Russian note in Windows-1251, a picture and a one-page PDF.
    let cp1251: Vec<u8> = "Счёт оплачен"
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c as u8
            } else if c == 'ё' {
                0xB8
            } else {
                (c as u32 - 0x410 + 0xC0) as u8
            }
        })
        .collect();
    std::fs::write(dir.join("1 заметка.txt"), cp1251).unwrap();
    std::fs::write(dir.join("2 фото.png"), pdfcraft_engine::export::encode_png(2, 2, &[255; 16]).unwrap()).unwrap();
    let pdf = pdfcraft_engine::Session::new().create_blank(300.0, 300.0, 1).unwrap();
    std::fs::write(dir.join("3 счёт.pdf"), pdf.as_ref()).unwrap();
    let before: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    let paths: Vec<String> = ["3 счёт.pdf", "1 заметка.txt", "2 фото.png"].iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect();

    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.run_inline = true;
        app.convert_to_pdf_paths(&paths);
        app
    });
    h.run_steps(4);

    let after: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(before.len(), after.len(), "nothing is written next to the files: {after:?}");
    let app = h.state();
    assert_eq!(app.views.len(), 1);
    let view = &app.views[0];
    let doc = app.session.get(view.id).unwrap();
    assert_eq!(doc.name, "Счета.pdf", "named after the folder");
    assert!(doc.path.is_none() && doc.dirty, "a new, unsaved document");
    assert_eq!(doc.info.pages.len(), 3);
    assert!(view.organize && view.review, "it opens in the page grid for review");
    assert_eq!(view.save_dir.as_deref(), Some(dir.as_path()), "Save suggests the files' folder");
    h.get_by_label("Arrange the pages");

    // The preview on the right shows the current page; clicking another page shows that one.
    assert_eq!(h.query_all_by_label("Page 1").count(), 2, "the thumbnail and the preview's title");
    h.get_by_label("Page 2").click();
    h.run_steps(3);
    assert_eq!(h.state().views[0].current, 1);
    assert!(h.query_all_by_label("Page 2").count() >= 2, "the thumbnail and the preview's title");
    // Zoom: larger and smaller pages, within its range.
    let zoom = h.state().views[0].grid_zoom;
    h.get_by_label("Larger pages").click();
    h.run_steps(2);
    assert!(h.state().views[0].grid_zoom > zoom);
    h.get_by_label("Smaller pages").click();
    h.get_by_label("Smaller pages").click();
    h.run_steps(2);
    assert!(h.state().views[0].grid_zoom < zoom);
    // The preview can be hidden.
    h.get_by_label("Hide the page preview").click();
    h.run_steps(2);
    assert!(!h.state().views[0].preview_open);

    h.get_by_label("Continue").click();
    h.run_steps(3);
    let view = &h.state().views[0];
    assert!(!view.organize && !view.review, "Continue goes on to the reader");
    assert!(h.query_by_label("Arrange the pages").is_none());
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
