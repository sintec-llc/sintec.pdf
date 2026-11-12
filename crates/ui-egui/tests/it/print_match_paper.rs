//! The Print dialog does not pick the paper from the PDF: A4 and automatic orientation by
//! default, and matching the page is an option (Preferences, and a switch in the dialog).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::print::{Layout, Orientation, PAPERS, SizeMode};
use pdfcraft_ui_egui::PdfCraftApp;

const SWITCH: &str = "Match paper to PDF page size when printing";

/// A 36 x 24 in page: no printer paper but ARCH-sized ones fits it.
fn arch_app(match_paper: bool) -> PdfCraftApp {
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.match_paper_to_page = match_paper;
    let bytes = app.session.create_blank(2592.0, 1728.0, 1).unwrap();
    app.open_bytes("architectural.pdf", None, bytes.as_ref().clone()).unwrap();
    app
}

#[test]
fn defaults_are_a4_automatic_orientation_and_no_matching() {
    let app = PdfCraftApp::new();
    assert!(!app.match_paper_to_page, "off by default");
    let d = app.print_draft;
    assert_eq!(d.paper_name(), "A4");
    assert_eq!(d.orientation, Orientation::Auto);
    assert!(!d.auto_rotate);
}

#[test]
fn opening_print_keeps_the_chosen_paper_unless_matching_is_on() {
    let mut app = arch_app(false);
    app.open_print();
    assert_eq!(app.print_draft.paper_name(), "A4", "a large page leaves the paper alone");
    assert_eq!(app.print_draft.orientation, Orientation::Auto);

    let mut app = arch_app(true);
    app.open_print();
    let settings = app.print_draft.settings(1, &[]).unwrap();
    let arch = PAPERS.iter().find(|(_, s)| (s.0 - 1728.0).abs() < 1.0 && (s.1 - 2592.0).abs() < 1.0).map(|p| p.1);
    assert_eq!(Some(settings.paper), arch);
    assert_eq!(settings.layout, Layout::Size(SizeMode::Actual));
}

#[test]
fn match_page_leaves_the_draft_alone_when_nothing_fits() {
    let mut d = PdfCraftApp::new().print_draft;
    let before = d.paper;
    d.match_page((123.0, 77.0));
    assert_eq!(d.paper, before);
    assert_eq!(d.orientation, Orientation::Auto);
}

#[test]
fn match_paper_preference_round_trips() {
    let mut app = PdfCraftApp::new();
    app.match_paper_to_page = true;
    let json = app.persist();
    let mut fresh = PdfCraftApp::new();
    assert!(!fresh.match_paper_to_page);
    fresh.restore(&json);
    assert!(fresh.match_paper_to_page);
    fresh.restore("{ not json");
    assert!(fresh.match_paper_to_page, "bad data leaves the setting alone");
}

#[test]
fn the_dialog_switch_matches_the_page_when_turned_on() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe(|_cc| arch_app(false));
    h.run_steps(4);
    assert!(h.state_mut().execute("print.dialog"));
    h.run_steps(3);
    assert!(!h.state().match_paper_to_page);
    h.get_by_label(SWITCH).click();
    h.run_steps(3);
    assert!(h.state().match_paper_to_page);
    assert_ne!(h.state().print_draft.paper_name(), "A4", "turning it on matches the page");
    h.get_by_label(SWITCH).click();
    h.run_steps(3);
    assert!(!h.state().match_paper_to_page);
}
