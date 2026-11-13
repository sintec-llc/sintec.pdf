//! Sintec.PDF: pages between documents (dragged from one document's Pages panel into another's
//! panel or onto its tab; Shift moves), and Extract and save as (the chosen pages as a new file).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

fn pages(app: &PdfCraftApp, view: usize) -> usize {
    app.session.get(app.views[view].id).map_or(0, |d| d.info.pages.len())
}

fn widths(app: &PdfCraftApp, view: usize) -> Vec<f32> {
    app.session.get(app.views[view].id).map(|d| d.info.pages.iter().map(|p| p.width).collect()).unwrap_or_default()
}

/// `a.pdf` (three letter-size pages) and `b.pdf` (two 300 × 400 pages), `a.pdf` in front with
/// its Pages panel open.
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
    assert_eq!(widths(h.state(), 1), [300.0, 612.0, 612.0, 300.0]);
    let mut picked: Vec<usize> = h.state().views[1].selected.iter().copied().collect();
    picked.sort_unstable();
    assert_eq!(picked, [1, 2], "the pages added are selected");
    // Shift moves: page 1 leaves a.pdf.
    h.state_mut().active = Some(0);
    h.state_mut().begin_page_transfer(vec![0]);
    h.state_mut().drop_page_transfer(1, None, true);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (2, 5), "moved to the end");
    // A move never empties the original: it copies and says so.
    h.state_mut().active = Some(0);
    h.state_mut().begin_page_transfer(vec![0, 1]);
    h.state_mut().drop_page_transfer(1, None, true);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (2, 7), "copied: moving would empty a.pdf");
    // A document changed during the drag gives nothing: its page numbers may name other pages.
    h.state_mut().active = Some(0);
    h.state_mut().begin_page_transfer(vec![0]);
    h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![1], degrees: 90 });
    h.state_mut().drop_page_transfer(1, None, true);
    assert_eq!((pages(h.state(), 0), pages(h.state(), 1)), (2, 7), "a.pdf changed during the drag");
    // Each side is an ordinary edit: undo takes the last copy back out of b.pdf.
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
    assert_eq!(widths(h.state(), 1), [300.0, 300.0, 612.0]);
    assert_eq!(widths(h.state(), 0), [612.0, 612.0, 612.0], "nothing moved inside a.pdf");
    assert!(h.state().session.get(h.state().views[0].id).unwrap().can_undo().is_none(), "a copy: a.pdf wasn't edited");
    assert!(h.state().page_transfer.is_none(), "the drag is over");
}

#[test]
fn thumbnails_dropped_on_another_documents_pages_panel_go_to_the_gap() {
    let mut h = two_documents();
    let thumb = h.get_by_label("Page 3").rect().center();
    let tab = h.get_by_label("b.pdf").rect().center();
    h.hover_at(thumb);
    h.run_steps(2);
    h.drag_at(thumb);
    h.run_steps(2);
    for k in 1..=8 {
        h.hover_at(thumb + (tab - thumb) * (k as f32 / 8.0));
        h.run_steps(1);
    }
    // Resting on the tab brings b.pdf to the front, with its own thumbnails.
    for _ in 0..200 {
        if h.state().active == Some(1) {
            break;
        }
        h.hover_at(tab);
        h.run_steps(1);
    }
    assert_eq!(h.state().active, Some(1), "resting on the tab opens it");
    h.run_steps(2);
    // b.pdf's second thumbnail: just above its middle is the gap between its two pages.
    let second = h.get_by_label("Page 2").rect();
    let gap = second.center() - egui::vec2(0.0, second.height() / 4.0);
    for k in 1..=8 {
        h.hover_at(tab + (gap - tab) * (k as f32 / 8.0));
        h.run_steps(1);
    }
    h.drop_at(gap);
    h.run_steps(3);
    assert_eq!(widths(h.state(), 1), [300.0, 612.0, 300.0], "page 3 of a.pdf between b.pdf's pages");
    assert_eq!(pages(h.state(), 0), 3, "a copy: a.pdf keeps it");
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

/// A drop on another tab that can't be made (here the original withholds extracting pages) says
/// so and leaves both documents as they were: the original's own Pages panel doesn't take the same
/// release as a move to the gap nearest the tab.
#[test]
fn a_refused_drop_on_another_tab_leaves_the_original_as_it_was() {
    use pdfcraft_engine::{Changes, Edit, Protection, Session};
    let mut s = Session::new();
    let blank = s.create_blank(612.0, 792.0, 3).unwrap();
    let id = s.open("a.pdf", None, blank, None).unwrap();
    // Page changes allowed (the thumbnails drag), copying not: no extracting pages (#5081).
    let protection = Protection { permissions_password: Some("owner".into()), changes: Changes::Pages, copy: false, ..Default::default() };
    s.apply(id, Edit::Protect(protection)).unwrap();
    let restricted = s.save_bytes(id).unwrap().as_ref().clone();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("a.pdf", None, restricted).unwrap();
        let b = app.session.create_blank(300.0, 400.0, 2).unwrap();
        app.open_bytes("b.pdf", None, b.as_ref().clone()).unwrap();
        app.active = Some(0);
        app.right = Some(RightPanel::Pages);
        app
    });
    h.run_steps(4);
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
    assert_eq!(pages(h.state(), 1), 2, "nothing copied into b.pdf");
    let a = h.state().session.get(h.state().views[0].id).unwrap();
    assert!(a.can_undo().is_none() && !a.dirty, "a.pdf's pages didn't move inside it either");
    assert!(h.state().views[0].panel_drag.is_none(), "the drag is over in a.pdf too");
    assert!(h.state().page_transfer.is_none());
}
