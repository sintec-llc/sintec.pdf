//! Sintec.PDF: Save, Print and Document properties sit at the left of the mode bar, and the save
//! button opens a menu (Save, Save as, Export ▸ formats) instead of saving at once.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        let bytes = app.session.create_from_text("t", "Hello").unwrap();
        app.open_bytes("a.pdf", None, bytes.as_ref().clone()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn the_document_buttons_sit_left_and_save_opens_a_menu() {
    let mut h = harness();
    let menu = h.get_by_label("Menu").rect();
    let tabs = h.get_by_label("Read").rect();
    let save = h.get_by_label_contains("Save (").rect();
    let print = h.get_by_label_contains("Print (").rect();
    let info = h.get_by_label_contains("Document properties (").rect();
    assert!(menu.right() < save.left() && save.right() < print.left() && print.right() < info.left(), "Menu, Save, Print, Info in order");
    assert!(info.right() < tabs.left(), "before the mode tabs");
    let id = h.state().views[0].id;
    h.get_by_label_contains("Save (").click();
    h.run_steps(3);
    // A menu, not a save.
    assert!(h.state().session.get(id).unwrap().path.is_none());
    // The menu: Save and Save as (their names carry the shortcut), and Export.
    assert!(h.query_all_by_label_contains("Save").count() >= 3, "Save and Save as listed");
    let save_as = h.get_by_label_contains("Save as…").rect();
    // Export ▸, the menu row under Save as (the left panel's "Export PDF" matches the text too).
    let export = h
        .query_all_by_label_contains("Export")
        .map(|n| n.rect())
        .find(|r| (r.left() - save_as.left()).abs() < 2.0 && r.top() >= save_as.bottom() - 1.0)
        .expect("Export in the save menu");
    h.hover_at(export.center());
    h.run_steps(10);
    for f in ["Export to Word…", "Export to HTML…", "Export to RTF…", "Export to image…", "Export to text…"] {
        h.get_by_label_contains(f);
    }
    h.get_by_label_contains("Export to image…").click();
    h.run_steps(3);
    assert!(matches!(h.state().dialog, Some(pdfcraft_ui_egui::Dialog::Export(_))));
}
