//! Sintec.PDF: pages between documents (dragged from one document's Pages panel into another's
//! panel or tab; Shift moves), and Extract and save as (the chosen pages as a new PDF file).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

fn pages(app: &PdfCraftApp, view: usize) -> usize {
    app.session.get(app.views[view].id).map_or(0, |d| d.info.pages.len())
}

fn two_documents() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        let a = app.session.create_blank(612.0, 792.0, 3).unwrap();
        app.open_bytes("a.pdf", None, a.as_ref().clone()).unwrap();
        let b = app.session.create_blank(300.0, 400.0, 2).unwrap();
        app.open_bytes("b.pdf", None, b.as_ref().clone()).unwrap();
        app.active = Some(0);
        app.right = Some(RightPanel::Pages);
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn pages_copy_and_move_into_another_document() {
    let mut h = two_documents();
    // Copy pages 2-3 of a.pdf between b.pdf's pages 1 and 2.
    h.state_mut().begin_page_transfer(vec![1, 2]);
    h.state_mut().drop_page_transfer(1, Some(1), false);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (3, 4), "copied, the original kept them");
    assert_eq!(h.state().active, Some(1), "the document they went to is shown");
    // b.pdf's new pages 2-3 are a.pdf's (letter-sized), its own (300 × 400) around them.
    let b = h.state().session.get(h.state().views[1].id).unwrap();
    let widths: Vec<f32> = b.info.pages.iter().map(|p| p.width).collect();
    assert_eq!(widths, vec![300.0, 612.0, 612.0, 300.0]);
    // Shift moves: page 1 leaves a.pdf.
    h.state_mut().active = Some(0);
    h.state_mut().begin_page_transfer(vec![0]);
    h.state_mut().drop_page_transfer(1, None, true);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (2, 5), "moved to the end");
    // A move never empties the original, and never removes pages it can't be sure of.
    h.state_mut().active = Some(0);
    h.state_mut().begin_page_transfer(vec![0, 1]);
    h.state_mut().drop_page_transfer(1, None, true);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (2, 7), "copied: moving would empty a.pdf");
    h.state_mut().active = Some(0);
    h.state_mut().begin_page_transfer(vec![0]);
    h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![1], degrees: 90 });
    h.state_mut().drop_page_transfer(1, None, true);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (2, 8), "copied: a.pdf changed during the drag");
    // Each step is an ordinary edit: undo takes the last copy back out.
    assert!(h.state().session.get(h.state().views[1].id).unwrap().can_undo().is_some());
}

#[test]
fn dragging_thumbnails_onto_another_tab_adds_them_at_its_end() {
    let mut h = two_documents();
    let thumb = h.get_by_label("Page 2").rect().center();
    let tab = h.get_by_label("b.pdf").rect().center();
    h.hover_at(thumb);
    h.run_steps(2);
    h.drag_at(thumb);
    h.run_steps(2);
    for k in 1..=8 {
        h.hover_at(thumb + (tab - thumb) * (k as f32 / 8.0));
        h.run_steps(1);
    }
    assert!(h.state().page_transfer.is_some(), "the drag can leave the document");
    h.drop_at(tab);
    h.run_steps(3);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (3, 3), "page 2 of a.pdf added to b.pdf");
    assert!(h.state().page_transfer.is_none(), "the drag is over");
}

#[test]
fn extract_and_save_as_writes_the_chosen_pages_to_a_new_file() {
    let mut h = two_documents();
    let out = std::env::temp_dir().join(format!("sintec-extract-save-as-{}.pdf", std::process::id()));
    h.state_mut().save_override = Some(out.to_string_lossy().into_owned());
    h.state_mut().views[0].select_pages(&[0, 2]);
    assert!(h.state_mut().execute("page.extract_save_as"));
    h.run_steps(2);
    let bytes = std::fs::read(&out).expect("written");
    std::fs::remove_file(&out).unwrap();
    let mut app = PdfCraftApp::new();
    app.open_bytes("x.pdf", None, bytes).unwrap();
    assert_eq!(pages(&app, 0), 2, "pages 1 and 3");
    assert_eq!(pages(h.state(), 0), 3, "the document itself is unchanged");
    assert!(!h.state().session.get(h.state().views[0].id).unwrap().dirty);
}
