//! Sintec.PDF: Pages panel ▸ Export to: the selected pages (or the page right-clicked) as Word,
//! Excel, HTML, RTF, images or text, and only those pages.

#[path = "support/render.rs"]
mod render_lock;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

/// A three-page document whose pages say "Page one", "Page two", "Page three", with the Pages panel open.
fn three_pages() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        let bytes = app.session.create_from_text("t", "Page one\u{c}Page two\u{c}Page three").unwrap();
        app.open_bytes("report.pdf", None, bytes.as_ref().clone()).unwrap();
        app.right = Some(RightPanel::Pages);
        app
    });
    h.run_steps(4);
    h
}

fn export_from_menu(h: &mut Harness<'static, PdfCraftApp>, thumbnail: &str, item: &str) {
    h.get_by_label(thumbnail).click_secondary();
    h.run_steps(2);
    h.get_by_label("Export to ⏵").hover();
    h.run_steps(3);
    if let Ok(dir) = std::env::var("SINTEC_SHOT_DIR") {
        render_lock::locked(|| h.render()).unwrap().save(std::path::Path::new(&dir).join("pages-export-menu.png")).unwrap();
    }
    h.get_by_label(item).click();
    h.run_steps(3);
}

#[test]
fn the_selected_pages_export_to_excel_and_word_alone() {
    let mut h = three_pages();
    assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().info.pages.len(), 3, "three pages");
    let dir = std::env::temp_dir().join(format!("sintec-page-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Pages 1 and 3 picked; the menu of page 3 exports both.
    h.state_mut().views[0].select_pages(&[0, 2]);
    h.run_steps(2);
    let xlsx = dir.join("pages.xlsx");
    h.state_mut().save_override = Some(xlsx.to_string_lossy().into_owned());
    export_from_menu(&mut h, "Page 3", "Excel (.xlsx)…");
    let bytes = std::fs::read(&xlsx).expect("the workbook is written");
    assert!(bytes.starts_with(b"PK"));
    let sheets = bytes.windows(b"xl/worksheets/sheet".len()).filter(|w| *w == b"xl/worksheets/sheet").count();
    // The XML inside is compressed: a sheet's name shows only in the archive's local and central
    // headers, two mentions a sheet, so two sheets for two pages.
    assert_eq!(sheets, 4, "two worksheets, for pages 1 and 3");
    // A page outside the selection, right-clicked, becomes the selection: page 2 alone, to HTML.
    let html = dir.join("page2.html");
    h.state_mut().save_override = Some(html.to_string_lossy().into_owned());
    export_from_menu(&mut h, "Page 2", "HTML…");
    let text = std::fs::read_to_string(&html).expect("the page is written");
    assert!(text.contains("Page two") && !text.contains("Page one") && !text.contains("Page three"), "{text}");
    assert!(!h.state().session.get(h.state().views[0].id).unwrap().dirty, "exporting changes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_from_the_selection_says_which_pages_and_the_menus_export_the_range_again() {
    let mut h = three_pages();
    let dir = std::env::temp_dir().join(format!("sintec-page-export-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    h.state_mut().export_dir_override = Some(dir.to_string_lossy().into_owned());
    h.state_mut().views[0].select_pages(&[1, 2]);
    h.run_steps(2);
    export_from_menu(&mut h, "Page 2", "Text…");
    h.get_by_label_contains("Selected pages: 2-3");
    h.get_by_label("Export").click();
    h.run_steps(4);
    let file = dir.join("report (p. 2-3).txt");
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(text.contains("Page two") && text.contains("Page three") && !text.contains("Page one"), "{text}");
    // Export to text from the menus: the range again, not the panel's selection.
    h.state_mut().execute("export.text");
    h.run_steps(3);
    assert!(h.query_by_label_contains("Selected pages").is_none());
    h.get_by_label("All pages");
    let _ = std::fs::remove_dir_all(&dir);
}
