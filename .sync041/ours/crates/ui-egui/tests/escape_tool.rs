//! Sintec.PDF: Esc leaves the current tool (placing a stamp or a signature, commenting…) for
//! the main view, unless a dialog or a text field has the key.

use egui::Key;
use egui_kittest::Harness;
use pdfcraft_engine::StampKind;
use pdfcraft_ui_egui::fill_sign::{FillTool, SavedSig};
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        let bytes = app.session.create_from_text("t", "Sign below").unwrap();
        app.open_bytes("contract.pdf", None, bytes.as_ref().clone()).unwrap();
        app.signature = Some(SavedSig::Typed("Ada L.".into()));
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn escape_leaves_the_stamp_and_signature_tools() {
    let mut h = harness();
    for tool in [QuickTool::Stamp(StampKind::Approved), QuickTool::Fill(FillTool::Signature), QuickTool::Hand] {
        h.state_mut().quick_tool = tool;
        h.run_steps(2);
        // The pointer over the page, showing the stamp or signature under the cursor.
        let page = h.state().views[0].page_screen_rect(0).expect("page on screen");
        h.hover_at(page.center());
        h.run_steps(2);
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().quick_tool, QuickTool::Select, "Esc leaves {tool:?}");
    }
    let id = h.state().views[0].id;
    assert!(h.state().session.get(id).unwrap().info.annotations.is_empty(), "nothing was placed");
}

#[test]
fn escape_closes_a_dialog_first() {
    let mut h = harness();
    h.state_mut().quick_tool = QuickTool::Stamp(StampKind::Approved);
    h.state_mut().execute("help.about");
    h.run_steps(3);
    assert!(h.state().dialog.is_some());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "Esc closes the dialog");
    assert_eq!(h.state().quick_tool, QuickTool::Stamp(StampKind::Approved), "and only the dialog");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().quick_tool, QuickTool::Select);
}
