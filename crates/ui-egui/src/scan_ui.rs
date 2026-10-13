//! Sintec.PDF: Scan into document. The scanner's own Windows dialog scans a page
//! ([`pdfcraft_engine::scan`]); the page goes into the open document after the selection (or at
//! the gap a "+" in the page grid chose), or into a new document when none is open. Then
//! «Сканировать ещё» scans the next page in after it, until the user is done.

use std::sync::{Arc, Mutex};

use pdfcraft_engine::scan::ScanError;

use crate::{Dialog, PdfCraftApp};

/// One page from the scanner: `Ok(None)` when the user closed the scanner's dialog.
pub type ScanResult = Result<Option<Vec<u8>>, ScanError>;

/// A scan in progress: the worker's result once the scanner's dialog closes.
pub type ScanRun = Arc<Mutex<Option<ScanResult>>>;

impl PdfCraftApp {
    /// Scan into document: a page into the active document after its selection, or into a new
    /// document when none is open.
    pub fn start_scan(&mut self) {
        self.scan_target = self.active.and_then(|i| self.views.get_mut(i)).map(|view| {
            let after = view.target_pages().last().map_or(0, |p| p + 1);
            (view.id, view.insert_at.take().unwrap_or(after))
        });
        self.scan_next();
    }

    /// Scan the next page into the same place, after the last one scanned.
    pub fn scan_next(&mut self) {
        if self.scan_run.is_some() {
            self.notify_tr("The scanner is already in use");
            return;
        }
        let run: ScanRun = Arc::new(Mutex::new(None));
        if !self.scan_fakes.is_empty() {
            // Tests: the "scanner" hands over pages they queued.
            let page = self.scan_fakes.remove(0);
            if let Ok(mut slot) = run.lock() {
                *slot = Some(page);
            }
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let slot = run.clone();
                let dir = std::env::temp_dir().join(format!("sintec-scan-{}", std::process::id()));
                std::thread::Builder::new()
                    .name("sintec-scan".into())
                    .spawn(move || {
                        let page = pdfcraft_engine::scan::scan_page(&dir);
                        if let Ok(mut s) = slot.lock() {
                            *s = Some(page);
                        }
                    })
                    .ok();
                self.notify_tr("Waiting for the scanner…");
            }
            #[cfg(target_arch = "wasm32")]
            if let Ok(mut slot) = run.lock() {
                *slot = Some(Err(ScanError::NotSupported));
            }
        }
        self.scan_run = Some(run);
        self.poll_scan();
    }

    /// Put the scanned page in once the scanner's dialog closes, and offer the next one.
    pub(crate) fn poll_scan(&mut self) {
        let Some(run) = self.scan_run.as_ref() else { return };
        let done = run.lock().ok().and_then(|mut r| r.take());
        let Some(done) = done else {
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            return;
        };
        self.scan_run = None;
        let png = match done {
            Ok(Some(png)) => png,
            // Closed the scanner's dialog: done.
            Ok(None) => return,
            Err(e) => {
                let reason = scan_error_text(&e);
                self.notify_fmt("Couldn't scan: {e}", &[("e", &reason)]);
                return;
            }
        };
        let name = tl!("Scan").to_string();
        let target = self.scan_target.and_then(|(doc, at)| self.views.iter().position(|v| v.id == doc).map(|i| (i, doc, at)));
        let placed = match target {
            Some((index, doc, at)) => {
                // Into that document (perhaps not the shown one now): show it while inserting.
                let shown = self.active;
                self.active = Some(index);
                let before = self.session.get(doc).map_or(0, |d| d.info.pages.len());
                if let Some(view) = self.views.get_mut(index) {
                    view.insert_at = Some(at);
                }
                self.insert_files(vec![(format!("{name}.png"), png)]);
                let added = self.session.get(doc).map_or(0, |d| d.info.pages.len()).saturating_sub(before);
                if shown.is_some_and(|s| self.views.get(s).is_some_and(|v| v.window == self.current_window)) {
                    self.active = shown;
                }
                self.scan_target = Some((doc, at.saturating_add(added)));
                added > 0
            }
            None => {
                // No document: the page starts a new one, in the page grid.
                let created = self.session.convert_to_pdf(&format!("{name}.png"), &Arc::new(png)).map(|(_, pdf)| pdf).map_err(|e| e.to_string());
                match self.open_created_bytes(&format!("{name}.pdf"), created) {
                    Ok(()) => {
                        if let Some(view) = self.active.and_then(|i| self.views.get_mut(i)) {
                            view.organize = true;
                            self.scan_target = Some((view.id, 1));
                        }
                        true
                    }
                    Err(e) => {
                        self.notify_fmt("Couldn't scan: {e}", &[("e", &e)]);
                        false
                    }
                }
            }
        };
        if placed && self.dialog.is_none() {
            self.dialog = Some(Dialog::ScanMore);
        }
    }
}

/// The reason, in the interface's language.
fn scan_error_text(e: &ScanError) -> String {
    match e {
        ScanError::NotSupported => tl!("Scanning works on Windows, through its built-in scanner support").to_string(),
        ScanError::NoWia => tl!("Windows' scanner support (WIA) isn't available on this computer").to_string(),
        ScanError::NoScanner => tl!("No scanner was found. Check that it is connected and switched on").to_string(),
        ScanError::Busy => tl!("The scanner is busy").to_string(),
        ScanError::Offline => tl!("The scanner is offline. Check that it is connected and switched on").to_string(),
        ScanError::FeederEmpty => tl!("The document feeder is empty").to_string(),
        ScanError::PaperJam => tl!("The scanner has a paper jam").to_string(),
        ScanError::CoverOpen => tl!("The scanner's cover is open").to_string(),
        ScanError::LampOff => tl!("The scanner's lamp is off").to_string(),
        ScanError::DeviceError => tl!("The scanner reported an error. Check it and try again").to_string(),
        ScanError::TooLarge => tl!("The scanned page is too large").to_string(),
        // The system's own message is not translated.
        ScanError::Failed(m) if m.is_empty() => tl!("The scan failed").to_string(),
        ScanError::Failed(m) => m.clone(),
    }
}
