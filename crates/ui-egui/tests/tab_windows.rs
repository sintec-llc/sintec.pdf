//! Sintec.PDF: tabs drag along the strip to reorder, out of it into a window of their own, and
//! onto another window's strip to join it; pages drag between documents in different windows.
//!
//! The test harness has no native windows (egui would draw extra windows inside the main one,
//! where their tools clash), so pulling a tab out does nothing here, as on the web; the window
//! logic is tested through the same functions the drags call.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::DocId;
use pdfcraft_ui_egui::PdfCraftApp;

fn three_documents() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        for (name, pages) in [("a.pdf", 1), ("b.pdf", 2), ("c.pdf", 3)] {
            let bytes = app.session.create_blank(612.0, 792.0, pages).unwrap();
            app.open_bytes(name, None, bytes.as_ref().clone()).unwrap();
        }
        app.active = Some(0);
        app
    });
    h.run_steps(4);
    h
}

fn names(app: &PdfCraftApp, window: u64) -> Vec<String> {
    app.views.iter().filter(|v| v.window == window).filter_map(|v| app.session.get(v.id).map(|d| d.name.clone())).collect()
}

fn id_of(app: &PdfCraftApp, name: &str) -> DocId {
    app.views.iter().find(|v| app.session.get(v.id).is_some_and(|d| d.name == name)).map(|v| v.id).unwrap()
}

fn pages(app: &PdfCraftApp, name: &str) -> usize {
    app.session.get(id_of(app, name)).map_or(0, |d| d.info.pages.len())
}

fn drag(h: &mut Harness<'static, PdfCraftApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for k in 1..=10 {
        h.hover_at(from + (to - from) * (k as f32 / 10.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(4);
}

#[test]
fn a_tab_dragged_along_the_strip_moves_there() {
    let mut h = three_documents();
    let a = h.get_by_label("a.pdf").rect();
    let c = h.get_by_label("c.pdf").rect().center();
    // c.pdf onto the left half of a.pdf: first.
    drag(&mut h, c, a.left_center() + egui::vec2(6.0, 0.0));
    assert_eq!(names(h.state(), 0), ["c.pdf", "a.pdf", "b.pdf"]);
    assert_eq!(h.state().active.and_then(|i| h.state().views.get(i)).map(|v| v.id), Some(id_of(h.state(), "c.pdf")), "the dragged tab shows");
    // And to the end.
    let c = h.get_by_label("c.pdf").rect().center();
    let b = h.get_by_label("b.pdf").rect();
    drag(&mut h, c, b.right_center() + egui::vec2(10.0, 0.0));
    assert_eq!(names(h.state(), 0), ["a.pdf", "b.pdf", "c.pdf"]);
    assert!(h.state().tab_drag.is_none());
}

#[test]
fn without_native_windows_a_tab_pulled_out_stays() {
    let mut h = three_documents();
    let b = h.get_by_label("b.pdf").rect().center();
    drag(&mut h, b, b + egui::vec2(60.0, 260.0));
    assert!(h.state().doc_windows.is_empty());
    assert_eq!(names(h.state(), 0), ["a.pdf", "b.pdf", "c.pdf"]);
}

#[test]
fn a_torn_off_tab_has_its_own_window_and_comes_back_when_it_closes() {
    let mut h = three_documents();
    let b = id_of(h.state(), "b.pdf");
    h.state_mut().tear_off_tab(b, None);
    h.run_steps(3);
    assert_eq!(h.state().doc_windows.len(), 1);
    let w = h.state().doc_windows[0].id;
    assert_eq!(names(h.state(), w), ["b.pdf"]);
    assert_eq!(h.state().doc_windows[0].active, Some(b), "it shows there");
    assert_eq!(names(h.state(), 0), ["a.pdf", "c.pdf"]);
    let main = h.state().active.and_then(|i| h.state().views.get(i));
    assert_eq!(main.map(|v| v.window), Some(0), "the main window shows one of its own tabs");
    // A second tab joins that window, before b.pdf.
    let c = id_of(h.state(), "c.pdf");
    h.state_mut().move_tab(c, w, Some(b));
    assert_eq!(names(h.state(), w), ["c.pdf", "b.pdf"]);
    assert_eq!(h.state().doc_windows[0].active, Some(c));
    assert_eq!(names(h.state(), 0), ["a.pdf"]);
    // Closing the window brings its tabs back to the main window: nothing is lost.
    h.state_mut().close_doc_window(w);
    h.run_steps(3);
    assert!(h.state().doc_windows.is_empty());
    assert_eq!(names(h.state(), 0), ["a.pdf", "c.pdf", "b.pdf"]);
}

#[test]
fn a_window_emptied_by_its_last_tab_leaving_closes() {
    let mut h = three_documents();
    let c = id_of(h.state(), "c.pdf");
    h.state_mut().tear_off_tab(c, None);
    let w = h.state().doc_windows[0].id;
    let a = id_of(h.state(), "a.pdf");
    h.state_mut().move_tab(c, 0, Some(a));
    h.run_steps(3);
    assert_eq!(names(h.state(), w), Vec::<String>::new());
    assert!(h.state().doc_windows.is_empty(), "the emptied window closed");
    assert_eq!(names(h.state(), 0), ["c.pdf", "a.pdf", "b.pdf"]);
    assert_eq!(h.state().active.and_then(|i| h.state().views.get(i)).map(|v| v.id), Some(c), "it shows in the main window");
}

#[test]
fn pages_dropped_onto_a_document_in_another_window() {
    let mut h = three_documents();
    let c = id_of(h.state(), "c.pdf");
    h.state_mut().tear_off_tab(c, None);
    let w = h.state().doc_windows[0].id;
    // a.pdf shows in the main window; its page goes to c.pdf in the other window.
    let a = h.state().views.iter().position(|v| v.id == id_of(h.state(), "a.pdf")).unwrap();
    h.state_mut().active = Some(a);
    h.state_mut().begin_page_transfer(vec![0]);
    let target = h.state().views.iter().position(|v| v.id == c).unwrap();
    h.state_mut().drop_page_transfer(target, Some(1), false);
    assert_eq!((pages(h.state(), "a.pdf"), pages(h.state(), "c.pdf")), (1, 4));
    assert_eq!(h.state().active, Some(a), "the main window keeps its tab");
    assert_eq!(h.state().doc_windows.iter().find(|x| x.id == w).and_then(|x| x.active), Some(c), "the other window shows c.pdf");
}
