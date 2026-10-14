//! Sintec.PDF: the pointing hand over everything clickable (egui shows it only over its own
//! `Button`), and not over the page or empty space.

use egui::CursorIcon;
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

fn cursor_at(h: &mut Harness<'static, PdfCraftApp>, p: egui::Pos2) -> CursorIcon {
    h.hover_at(p);
    h.run_steps(2);
    h.output().platform_output.cursor_icon
}

#[test]
fn clickables_show_the_hand_and_the_page_does_not() {
    let mut h = harness();
    // A mode tab and the document's tab are painted widgets, not egui Buttons.
    let read = h.get_by_label("Read").rect().center();
    assert_eq!(cursor_at(&mut h, read), CursorIcon::PointingHand, "mode tab");
    let tab = h.get_by_label_contains("a.pdf").rect().center();
    assert_eq!(cursor_at(&mut h, tab), CursorIcon::PointingHand, "document tab");
    // Toolbar buttons keep the hand while their tooltip shows.
    let save = h.get_by_label_contains("Save (").rect().center();
    assert_eq!(cursor_at(&mut h, save), CursorIcon::PointingHand, "Save");
    for _ in 0..20 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::PointingHand, "Save, tooltip shown");
    // The page itself (the Select tool) keeps its own cursor, never the hand.
    let page = h.state().views[0].page_screen_rect(0).expect("on screen").center();
    assert_ne!(cursor_at(&mut h, page), CursorIcon::PointingHand, "page");
    // A checkbox in Preferences (a modal): the page under the dialog must not lend it its I-beam.
    h.state_mut().execute("app.preferences");
    // The dialog sizes itself over its first frames.
    h.run_steps(10);
    let check = h.get_by_label("Check for updates automatically").rect().center();
    assert_eq!(cursor_at(&mut h, check), CursorIcon::PointingHand, "checkbox");
    // Empty space in the dialog keeps the arrow.
    let blank = h.get_by_label("Identity").rect().right_center() + egui::vec2(200.0, 0.0);
    assert_eq!(cursor_at(&mut h, blank), CursorIcon::Default, "dialog background");
}
