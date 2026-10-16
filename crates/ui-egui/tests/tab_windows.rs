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

/// While a tab is dragged its ghost follows the pointer: the card along the strip, the card and
/// the page once pulled out. It is gone when the button comes up.
#[test]
fn a_dragged_tab_shows_its_ghost_under_the_pointer() {
    let mut h = three_documents();
    let b = h.get_by_label("b.pdf").rect().center();
    h.hover_at(b);
    h.run_steps(2);
    h.drag_at(b);
    h.run_steps(2);
    let along = b + egui::vec2(60.0, 0.0);
    h.hover_at(along);
    h.run_steps(2);
    let ghost = h.state().drag_ghost().expect("a ghost while dragging");
    assert_eq!((ghost.title.as_str(), ghost.preview), ("b.pdf", false), "along the strip: the card only");
    let out = egui::pos2(700.0, 500.0);
    for k in 1..=6 {
        h.hover_at(along + (out - along) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    let ghost = h.state().drag_ghost().expect("a ghost while dragging");
    assert!(ghost.preview, "pulled out: the page too");
    if let Ok(dir) = std::env::var("SINTEC_SHOT_DIR") {
        h.render().unwrap().save(std::path::Path::new(&dir).join("tab-drag-ghost.png")).unwrap();
    }
    h.drop_at(out);
    h.run_steps(4);
    assert!(h.state().drag_ghost().is_none(), "gone after the drop");
}

/// Records each launch a torn-off tab asks for, or refuses it when `fail` is set.
#[derive(Clone, Default)]
struct Launches {
    calls: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}

impl Launches {
    fn install(&self, app: &mut PdfCraftApp, fail: bool) {
        let calls = self.calls.clone();
        app.window_spawner = Some(std::sync::Arc::new(move |args: &[std::ffi::OsString]| {
            if fail {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such program"));
            }
            calls.lock().unwrap().push(args.iter().map(|a| a.to_string_lossy().into_owned()).collect());
            Ok(())
        }));
    }

    fn all(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-tear-off-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `three_documents`, with a recovery folder in `dir` and a spawner.
fn spawning(dir: &std::path::Path, launches: &Launches, fail: bool) -> Harness<'static, PdfCraftApp> {
    let mut h = three_documents();
    h.state_mut().enable_recovery_without_offer(pdfcraft_ui_egui::RecoveryStore::new(dir.join("Recovery")));
    launches.install(h.state_mut(), fail);
    h
}

fn recover_key(args: &[String]) -> String {
    args.iter().position(|a| a == "--recover-key").and_then(|i| args.get(i + 1)).cloned().expect("a recovery key")
}

#[test]
fn a_tab_pulled_out_goes_to_a_process_of_its_own_and_leaves_this_window() {
    let dir = scratch("drag");
    let launches = Launches::default();
    let mut h = spawning(&dir, &launches, false);
    let b = h.get_by_label("b.pdf").rect().center();
    drag(&mut h, b, b + egui::vec2(60.0, 260.0));
    let calls = launches.all();
    assert_eq!(calls.len(), 1, "one process started");
    let args = &calls[0];
    assert_eq!(&args[..2], ["--new-window", "--torn-off"], "a launch of its own, not handed to this instance");
    assert_eq!(args.get(2).map(String::as_str), Some("--window-pos"), "placed where it was let go: {args:?}");
    // b.pdf was never saved: its working state goes as a recovery entry under its own name.
    let key = recover_key(args);
    let store = pdfcraft_ui_egui::RecoveryStore::new(dir.join("Recovery"));
    assert!(store.read(&key).unwrap().starts_with(b"%PDF"));
    let meta = store.list().into_iter().find(|m| m.key == key).unwrap();
    assert_eq!((meta.name.as_str(), meta.path.as_deref()), ("b.pdf", None));
    // The tab is gone from here: no window of this process holds it, so nothing comes back.
    h.run_steps(2);
    assert_eq!(names(h.state(), 0), ["a.pdf", "c.pdf"]);
    assert!(h.state().doc_windows.is_empty());
    assert!(h.state().session.docs().iter().all(|d| d.name != "b.pdf"), "this process no longer owns it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_document_goes_by_its_file_and_unsaved_changes_keep_their_path() {
    let dir = scratch("saved");
    let launches = Launches::default();
    let mut h = spawning(&dir, &launches, false);
    let file = dir.join("saved.pdf");
    let bytes = h.state_mut().session.create_blank(612.0, 792.0, 2).unwrap();
    std::fs::write(&file, bytes.as_ref()).unwrap();
    let path = file.to_string_lossy().into_owned();
    h.state_mut().open_bytes("saved.pdf", Some(path.clone()), bytes.as_ref().clone()).unwrap();
    h.run_steps(2);
    let saved = id_of(h.state(), "saved.pdf");
    h.state_mut().detach_tab(saved, None);
    assert_eq!(launches.all(), [vec!["--new-window".to_owned(), "--torn-off".to_owned(), path.clone()]]);
    assert!(pdfcraft_ui_egui::RecoveryStore::new(dir.join("Recovery")).list().is_empty(), "no copy of a saved file");
    assert_eq!(std::fs::read(&file).unwrap(), bytes.as_ref().clone(), "the original is untouched");
    // Changed since: a copy goes, belonging at the original path (closing it asks to save).
    h.state_mut().open_bytes("saved.pdf", Some(path.clone()), bytes.as_ref().clone()).unwrap();
    let again = h.state().views.last().map(|v| v.id).unwrap();
    h.state_mut().session.mark_recovered(again, Some(path.clone()));
    h.state_mut().detach_tab(again, None);
    let calls = launches.all();
    let key = recover_key(calls.last().unwrap());
    let meta = pdfcraft_ui_egui::RecoveryStore::new(dir.join("Recovery")).list().into_iter().find(|m| m.key == key).unwrap();
    assert_eq!((meta.name.as_str(), meta.path.as_deref()), ("saved.pdf", Some(path.as_str())));
    assert_eq!(std::fs::read(&file).unwrap(), bytes.as_ref().clone(), "the original is still untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tab_whose_process_cannot_start_stays_and_says_so() {
    let dir = scratch("fail");
    let launches = Launches::default();
    let mut h = spawning(&dir, &launches, true);
    let b = h.get_by_label("b.pdf").rect().center();
    drag(&mut h, b, b + egui::vec2(60.0, 260.0));
    assert_eq!(names(h.state(), 0), ["a.pdf", "b.pdf", "c.pdf"], "the tab stays");
    assert!(h.state().doc_windows.is_empty());
    let toast = h.state().toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.contains("Couldn't open b.pdf in a new window") && toast.contains("no such program"), "{toast}");
    assert!(pdfcraft_ui_egui::RecoveryStore::new(dir.join("Recovery")).list().is_empty(), "the unused copy is removed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsaved_work_without_a_recovery_folder_stays() {
    let launches = Launches::default();
    let mut h = three_documents();
    launches.install(h.state_mut(), false);
    let b = id_of(h.state(), "b.pdf");
    h.state_mut().detach_tab(b, None);
    assert!(launches.all().is_empty());
    assert_eq!(names(h.state(), 0), ["a.pdf", "b.pdf", "c.pdf"]);
    assert!(h.state().toast.is_some());
}

#[test]
fn the_new_process_opens_handed_over_work_as_unsaved_changes() {
    let dir = scratch("open");
    let store = pdfcraft_ui_egui::RecoveryStore::new(dir.join("Recovery"));
    let mut app = PdfCraftApp::new();
    let bytes = app.session.create_blank(612.0, 792.0, 1).unwrap();
    let key = pdfcraft_ui_egui::tear_off::write_hand_off(&store, DocId(1), "Report.pdf", Some("/docs/Report.pdf".into()), false, &bytes).unwrap();
    app.enable_recovery_without_offer(store.clone());
    assert!(app.dialog.is_none(), "no recovery offer in a torn-off tab's window");
    app.open_handed_off(&key);
    let doc = app.views.last().and_then(|v| app.session.get(v.id)).unwrap();
    assert_eq!((doc.name.as_str(), doc.path.as_deref(), doc.dirty), ("Report.pdf", Some("/docs/Report.pdf"), true));
    assert!(store.read(&key).is_ok(), "the entry stays this process's recovery copy until saved");
    // A key that could name a file outside the folder opens nothing.
    let before = app.views.len();
    app.open_handed_off("../Report");
    assert_eq!(app.views.len(), before);
    assert!(app.toast.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
