//! Sintec.PDF: a tab torn off the tab strip opens in a Sintec.PDF process of its own.
//!
//! The new process owns the document from then on, so each window closes on its own: closing one
//! neither closes the other nor sends its tabs back. The tab leaves this window once the process
//! has started; when it can't start, the tab stays and a notice says why.
//!
//! What the new process opens:
//! - a document saved without changes: its file;
//! - unsaved work (or a document never saved, or whose file has gone): a crash-recovery entry
//!   ([`crate::RecoveryStore`]) written from what Save would write, under the original name and
//!   path. The new process recovers it as unsaved changes, so closing it still asks to save, and
//!   the entry stays its recovery copy until then. The original file is never written.
//!
//! The arguments are `--new-window` (so a running instance doesn't take the launch), `--torn-off`
//! (no recovery offer, no reopened session), `--window-pos X,Y` (where the tab was let go),
//! `--page N`, then the file or `--recover-key KEY`.
//!
//! Where no process can be started (the web, and tests, which install no spawner), the tab opens
//! in a window of this process instead ([`PdfCraftApp::tear_off_tab`]).
//!
//! A tab dropped on the strip of a window of another process goes the same way, file or recovery
//! entry, through [`crate::tab_transfer`] ([`PdfCraftApp::send_tab`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Pos2, Vec2, vec2};
use pdfcraft_engine::DocId;

use crate::PdfCraftApp;
use crate::recovery::{RecoveryMeta, RecoveryStore};

/// Starts a Sintec.PDF process with these arguments, not waiting for it. The desktop app sets it
/// ([`spawn_self`]); without one, torn-off tabs open in windows of this process.
pub type WindowSpawner = Arc<dyn Fn(&[OsString]) -> std::io::Result<()> + Send + Sync>;

/// The furthest a window position may be from the origin, in points: well beyond any desktop,
/// so a position from the command line can't ask the platform for something absurd.
pub const MAX_WINDOW_POS: f32 = 32_768.0;

/// The new window's corner relative to the pointer: under it, as if carried there by its tab.
const GRAB: Vec2 = vec2(120.0, 18.0);

/// Longest recovery key accepted from the command line.
const MAX_KEY_LEN: usize = 128;

/// What the new process opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandOff {
    /// A document saved without changes: its file.
    File(String),
    /// Unsaved work: the key of the recovery entry holding it.
    Recovery(String),
}

/// `--window-pos X,Y`: two finite numbers, in points, each clamped to ±[`MAX_WINDOW_POS`].
pub fn parse_window_pos(value: &str) -> Result<Pos2, String> {
    let (x, y) = value.split_once(',').ok_or("the window position must be X,Y")?;
    let coordinate = |s: &str| -> Result<f32, String> {
        let v = s.trim().parse::<f32>().map_err(|e| format!("window position {s:?}: {e}"))?;
        if v.is_finite() { Ok(v.clamp(-MAX_WINDOW_POS, MAX_WINDOW_POS)) } else { Err(format!("window position {s:?} is not a finite number")) }
    };
    Ok(Pos2::new(coordinate(x)?, coordinate(y)?))
}

/// A recovery key named on the command line: letters, digits, `-` and `_` only, so it can only
/// name an entry inside the recovery folder.
pub fn valid_recovery_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_KEY_LEN && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The arguments for the new process: `what` opens at `page` (0-based), with the window's corner
/// placed for a tab let go at `drop` (screen points; skipped unless finite).
pub fn hand_off_args(what: &HandOff, drop: Option<Pos2>, page: usize) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--new-window".into(), "--torn-off".into()];
    if let Some(p) = drop.filter(|p| p.x.is_finite() && p.y.is_finite()) {
        let corner = p - GRAB;
        let clamp = |v: f32| v.clamp(-MAX_WINDOW_POS, MAX_WINDOW_POS).round();
        args.push("--window-pos".into());
        args.push(format!("{},{}", clamp(corner.x), clamp(corner.y)).into());
    }
    if page > 0 {
        args.push("--page".into());
        args.push(page.saturating_add(1).to_string().into());
    }
    match what {
        HandOff::File(path) => {
            // A path that reads as an option would be taken for one.
            let path = if path.starts_with('-') { Path::new(".").join(path) } else { PathBuf::from(path) };
            args.push(path.into_os_string());
        }
        HandOff::Recovery(key) => {
            args.push("--recover-key".into());
            args.push(key.into());
        }
    }
    args
}

/// Write unsaved work (`bytes`, what Save would write) as a recovery entry for the new process to
/// open as document `name`, belonging at `path`. Returns its key.
pub fn write_hand_off(store: &RecoveryStore, doc: DocId, name: &str, path: Option<String>, encrypted: bool, bytes: &[u8]) -> std::io::Result<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let key = format!("{now}-{}-tear-{}", std::process::id(), doc.0);
    // No instance: the new process finds the entry by listing the store, and a listing leaves
    // out entries of a live instance, which this one still is.
    let meta = RecoveryMeta { key: key.clone(), name: name.to_owned(), path, saved_at: now, encrypted, instance: None };
    store.write(&meta, bytes)?;
    Ok(key)
}

/// Start this program again with `args`, detached: it outlives this process, and neither waits
/// for the other.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_self(args: &[OsString]) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Its own process group, so a console's Ctrl+C for this one leaves it alone.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: a terminal's Ctrl+C or hang-up for this one leaves it alone.
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    // Reap it when it exits first, so it doesn't linger as a zombie while this one runs.
    let reaper = std::thread::Builder::new().name("torn-off-window".into()).spawn(move || {
        let _ = child.wait();
    });
    if let Err(e) = reaper {
        log::warn!("not waiting for the torn-off window's process: {e}");
    }
    Ok(())
}

impl PdfCraftApp {
    /// Tab `doc` was pulled out of the tab strip and let go at `at` (screen points): open it in a
    /// process of its own and remove it here. Without a spawner it opens in a window of this
    /// process. When the process can't start, the tab stays and a notice says why.
    pub fn detach_tab(&mut self, doc: DocId, at: Option<Pos2>) {
        let Some(spawner) = self.window_spawner.clone() else {
            self.tear_off_tab(doc, at);
            return;
        };
        // What's typed in a field goes along (#166); added text that can't be saved stays here,
        // shown with its warning.
        if !self.commit_typing_in(doc) {
            return;
        }
        let Some(index) = self.views.iter().position(|v| v.id == doc) else { return };
        let name = self.session.get(doc).map(|d| d.display_name()).unwrap_or_default();
        match self.hand_off(index, at, &spawner) {
            Ok(()) => self.close_handed_off(index),
            Err(e) => {
                log::warn!("tearing off {name}: {e}");
                self.notify_fmt("Couldn't open {name} in a new window: {e}. It stays here.", &[("name", &name), ("e", &e)]);
            }
        }
    }

    /// Start the process that takes over tab `index`. On failure nothing it needed is left behind.
    fn hand_off(&mut self, index: usize, at: Option<Pos2>, spawner: &WindowSpawner) -> Result<(), String> {
        let (what, store, page) = self.hand_off_what(index)?;
        let args = hand_off_args(&what, at, page);
        if let Err(e) = spawner(&args) {
            if let (HandOff::Recovery(key), Some(store)) = (&what, store) {
                store.remove(key);
            }
            return Err(e.to_string());
        }
        log::info!("tab handed to a new process: {what:?}");
        Ok(())
    }

    /// Tab `doc` was let go over the tab strip of a window of another process: hand it over
    /// ([`crate::tab_transfer`]) and, once that process has confirmed it took it, remove it here.
    /// Otherwise the tab stays and a notice says why.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_tab(&mut self, doc: DocId, target: crate::tab_transfer::Target) {
        if !self.commit_typing_in(doc) {
            return;
        }
        let Some(index) = self.views.iter().position(|v| v.id == doc) else { return };
        let name = self.session.get(doc).map(|d| d.display_name()).unwrap_or_default();
        let result = self.hand_off_what(index).and_then(|(what, store, page)| {
            // The other process has a working folder of its own.
            let what = match what {
                HandOff::File(p) => HandOff::File(std::path::absolute(&p).map_or(p, |a| a.to_string_lossy().into_owned())),
                other => other,
            };
            let take = crate::tab_transfer::Take { what, window: target.window, before: target.before, page };
            crate::tab_transfer::send(&target, &take).map_err(|code| {
                // Not taken: the copy made for it isn't needed.
                if let (HandOff::Recovery(key), Some(store)) = (&take.what, store) {
                    store.remove(key);
                }
                crate::tab_transfer::reason_text(&code)
            })
        });
        match result {
            Ok(()) => {
                log::info!("tab {name} moved to a window of process {}", target.pid);
                self.close_handed_off(index);
            }
            Err(e) => {
                log::warn!("moving {name} to another window: {e}");
                self.notify_fmt("Couldn't move {name} to that window: {e}. It stays here.", &[("name", &name), ("e", &e)]);
            }
        }
    }

    /// What another process opens for tab `index`: its file when saved without changes,
    /// otherwise a recovery entry written now from what Save would write (with the store, to
    /// remove it if the hand-off fails). And the page it shows.
    fn hand_off_what(&mut self, index: usize) -> Result<(HandOff, Option<RecoveryStore>, usize), String> {
        let unsaved = self.has_unsaved_work(index);
        let view = self.views.get(index).ok_or("the tab was closed")?;
        let page = view.current;
        let doc = self.session.get(view.id).ok_or("the tab was closed")?;
        let saved_file = doc.path.clone().filter(|p| !unsaved && Path::new(p).is_file());
        let (what, store) = match saved_file {
            Some(path) => (HandOff::File(path), None),
            None => {
                let store = self.recovery.clone().ok_or_else(|| tl!("there is no folder to keep a copy of its unsaved changes in").to_string())?;
                let bytes = self.session.save_bytes(doc.id).map_err(|e| e.to_string())?;
                // Encrypted documents stay encrypted in the copy (the working file is); the new
                // process asks for the password, as recovery does.
                let key = write_hand_off(&store, doc.id, &doc.name, doc.path.clone(), doc.info.encrypted, &bytes).map_err(|e| e.to_string())?;
                (HandOff::Recovery(key), Some(store))
            }
        };
        Ok((what, store, page))
    }

    /// Tab `index` now lives in another process: close it here without asking (its unsaved work
    /// went along) and without offering to reopen it, which would open it twice.
    fn close_handed_off(&mut self, index: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        let closed_before = self.closed_tabs.len();
        self.close_tab(index);
        #[cfg(not(target_arch = "wasm32"))]
        if self.closed_tabs.len() > closed_before {
            self.closed_tabs.pop();
        }
    }

    /// Open the unsaved work a torn-off tab handed over (`--recover-key`): as unsaved changes,
    /// with its own name and path, keeping its recovery entry as this process's copy.
    pub fn open_handed_off(&mut self, key: &str) {
        let store = self.recovery.clone();
        let meta = store.filter(|_| valid_recovery_key(key)).and_then(|s| s.list().into_iter().find(|m| m.key == key));
        let Some(meta) = meta else {
            log::warn!("no handed-over document {key:?}");
            self.notify_tr("Couldn't open the document handed over from another window.");
            return;
        };
        self.recoverable.retain(|m| m.key != meta.key);
        self.recoverable.push(meta);
        self.recover(&[key.to_owned()]);
    }

    /// Autosave into `store` without offering what earlier sessions left behind: for a torn-off
    /// tab's window, while the window it came from still runs and owns its own entries.
    pub fn enable_recovery_without_offer(&mut self, store: RecoveryStore) {
        self.recovery = Some(store);
    }

    /// A torn-off window ends with its last document: closing it, or sending it to another
    /// window, quits this process rather than leaving an empty start page behind.
    pub(crate) fn close_when_last_document_gone(&mut self, ctx: &egui::Context) {
        if !self.quit_when_empty {
            return;
        }
        if !self.views.is_empty() {
            self.had_document = true;
        } else if self.had_document && self.close_request.is_none() && self.password_prompt.is_none() && self.dialog.is_none() {
            self.request_quit(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_saved_document_opens_its_file_in_a_new_window_where_it_was_dropped() {
        let args = hand_off_args(&HandOff::File("/docs/a.pdf".into()), Some(Pos2::new(500.0, 300.5)), 4);
        assert_eq!(strings(&args), ["--new-window", "--torn-off", "--window-pos", "380,283", "--page", "5", "/docs/a.pdf"]);
    }

    #[test]
    fn without_a_drop_point_or_page_only_the_file_and_new_window() {
        let args = hand_off_args(&HandOff::File("/docs/a.pdf".into()), None, 0);
        assert_eq!(strings(&args), ["--new-window", "--torn-off", "/docs/a.pdf"]);
        let odd = hand_off_args(&HandOff::File("/d.pdf".into()), Some(Pos2::new(f32::NAN, 1.0)), 0);
        assert!(!strings(&odd).contains(&"--window-pos".to_owned()), "a non-finite drop point is not passed on");
        let far = hand_off_args(&HandOff::File("/d.pdf".into()), Some(Pos2::new(1e30, -1e30)), 0);
        assert_eq!(strings(&far)[3], "32768,-32768");
    }

    #[test]
    fn a_file_named_like_an_option_is_not_taken_for_one() {
        let args = strings(&hand_off_args(&HandOff::File("--version".into()), None, 0));
        assert_eq!(args.last().map(|s| s.replace('\\', "/")), Some("./--version".to_owned()));
    }

    #[test]
    fn unsaved_work_goes_as_a_recovery_entry_with_its_name_and_path() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-tear-off-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = RecoveryStore::new(&dir);
        let bytes = b"%PDF-1.7\n% unsaved edits\n%%EOF\n";
        let key = write_hand_off(&store, DocId(7), "Report.pdf", Some("/docs/Report.pdf".into()), false, bytes).unwrap();
        assert!(valid_recovery_key(&key), "{key}");
        assert_eq!(store.read(&key).unwrap(), bytes, "the copy holds the working state");
        let meta = store.list().into_iter().find(|m| m.key == key).unwrap();
        assert_eq!(meta.name, "Report.pdf");
        assert_eq!(meta.path.as_deref(), Some("/docs/Report.pdf"));
        assert!(!meta.encrypted);
        let args = hand_off_args(&HandOff::Recovery(key.clone()), Some(Pos2::new(120.0, 18.0)), 0);
        assert_eq!(strings(&args), ["--new-window", "--torn-off", "--window-pos", "0,0", "--recover-key", key.as_str()]);
        // An untitled document has no path to go back to.
        let untitled = write_hand_off(&store, DocId(8), "Untitled 1", None, false, bytes).unwrap();
        assert_ne!(untitled, key);
        assert_eq!(store.list().into_iter().find(|m| m.key == untitled).and_then(|m| m.path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_positions_must_be_two_finite_numbers() {
        assert_eq!(parse_window_pos("10,-20.5"), Ok(Pos2::new(10.0, -20.5)));
        assert_eq!(parse_window_pos(" 3 , 4 "), Ok(Pos2::new(3.0, 4.0)));
        assert_eq!(parse_window_pos("1e9,2"), Ok(Pos2::new(MAX_WINDOW_POS, 2.0)), "clamped");
        for bad in ["", "10", "10;20", "x,1", "1,", "NaN,1", "1,inf", "-inf,0", "1,2,3"] {
            assert!(parse_window_pos(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn recovery_keys_name_only_entries_in_the_folder() {
        assert!(valid_recovery_key("1760000000-42-tear-3"));
        for bad in ["", "../x", "a/b", "a\\b", "a.pdf", "a b", &"k".repeat(MAX_KEY_LEN + 1)] {
            assert!(!valid_recovery_key(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_torn_off_window_quits_when_its_last_document_is_closed() {
        let ctx = egui::Context::default();
        let mut app = PdfCraftApp::new();
        app.quit_when_empty = true;
        // Nothing opened yet (the file may still be loading): the window stays.
        app.close_when_last_document_gone(&ctx);
        assert!(!app.allow_quit);
        let bytes = app.session.create_blank(612.0, 792.0, 1).unwrap();
        app.open_bytes("a.pdf", None, bytes.as_ref().clone()).unwrap();
        app.close_when_last_document_gone(&ctx);
        assert!(!app.allow_quit, "a document is open");
        app.close_tab(0);
        app.close_when_last_document_gone(&ctx);
        assert!(app.allow_quit, "the last document is closed");
    }

    #[test]
    fn the_main_window_stays_on_the_start_page_after_its_last_document() {
        let ctx = egui::Context::default();
        let mut app = PdfCraftApp::new();
        let bytes = app.session.create_blank(612.0, 792.0, 1).unwrap();
        app.open_bytes("a.pdf", None, bytes.as_ref().clone()).unwrap();
        app.close_when_last_document_gone(&ctx);
        app.close_tab(0);
        app.close_when_last_document_gone(&ctx);
        assert!(!app.allow_quit);
    }
}
