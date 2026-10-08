//! Help ▸ Check for updates (issue #28): ask for the latest release, then update in place or
//! open its download page.
//!
//! The desktop app supplies how to ask ([`PdfCraftApp::update_source`]) and how to install
//! ([`PdfCraftApp::update_installer`]), so this crate has no network code; without a source (the
//! web build, tests) the command opens the releases page. Nothing is downloaded or installed
//! until the user chooses Update. It asks only when the user does: there is no check at start.

use std::sync::Arc;

use egui::{Align, Layout};

use crate::{PdfCraftApp, theme, widgets};

/// Where every PdfCraft release is listed.
pub const RELEASES_PAGE: &str = "https://github.com/sintec-llc/sintec.pdf/releases";

/// The latest published release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
    /// Its version tag, such as `v0.2.0`.
    pub version: String,
    /// Its page on [`RELEASES_PAGE`], where the downloads are.
    pub url: String,
    /// The installer for this machine, when the release has one (enables Update).
    pub installer: Option<String>,
    /// The release's SHA-256 checksum list, which the installer is checked against.
    pub checksums: Option<String>,
}

/// Asks for the latest release (blocking; it runs on its own thread).
pub type UpdateSource = Arc<dyn Fn() -> Result<Release, String> + Send + Sync>;

/// Downloads, checks and starts the installer of a release (blocking; it runs on its own
/// thread). `Ok` means the installer is on its way and the app should close.
pub type UpdateInstaller = Arc<dyn Fn(&Release) -> Result<(), String> + Send + Sync>;

/// Whether release `latest` (a tag such as `v0.2.0`) is newer than version `current` (`0.1.1`).
/// Pre-release and build suffixes are ignored; a version that doesn't parse is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok(),
        None if required => None,
        None => Some(0),
    };
    let version = (next(true)?, next(false)?, next(false)?);
    parts.next().is_none().then_some(version)
}

/// Where a check is.
#[derive(Default)]
pub(crate) enum Check {
    #[default]
    Idle,
    #[cfg(not(target_arch = "wasm32"))]
    Running(std::sync::mpsc::Receiver<Result<Release, String>>),
    Done(Result<Release, String>),
}

/// Where an Update is.
#[derive(Default)]
pub(crate) enum Install {
    #[default]
    Idle,
    #[cfg(not(target_arch = "wasm32"))]
    Running(std::sync::mpsc::Receiver<Result<(), String>>),
    Failed(String),
}

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) check: Check,
    pub(crate) install: Install,
    /// The Updates dialog is showing.
    pub(crate) open: bool,
}

impl PdfCraftApp {
    /// Help ▸ Check for updates: ask for the latest release and show the outcome.
    pub fn check_for_updates(&mut self) {
        let Some(source) = self.update_source.clone() else {
            self.open_url(RELEASES_PAGE);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.updates.open = true;
            if matches!(self.updates.check, Check::Running(_)) {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                // The receiver may be gone (the app quit): nothing to report to then.
                let _ = tx.send(source());
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.check = Check::Running(rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = source;
            self.open_url(RELEASES_PAGE);
        }
    }

    /// Update: download, check and start the new version's installer, then close (the installer
    /// restarts Sintec.PDF on the new version). Refused while documents have unsaved changes.
    pub fn start_update(&mut self, release: Release) {
        let Some(installer) = self.update_installer.clone() else {
            self.open_url(&release.url);
            return;
        };
        if self.views.iter().any(|v| self.session.get(v.id).is_some_and(|d| d.dirty)) {
            self.updates.install = Install::Failed(tl!("Save your documents first: Sintec.PDF closes to install the new version.").to_string());
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if matches!(self.updates.install, Install::Running(_)) {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(installer(&release));
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.install = Install::Running(rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = installer;
            self.open_url(&release.url);
        }
    }

    /// Pick up a finished check or update (each frame).
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Check::Running(rx) = &self.updates.check {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the update check stopped unexpectedly".into()),
            };
            self.updates.check = Check::Done(result);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Install::Running(rx) = &self.updates.install {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the update stopped unexpectedly".into()),
            };
            match result {
                // The installer waits for this window to close, then installs and restarts it.
                Ok(()) => {
                    self.updates.install = Install::Idle;
                    self.update_started = true;
                    if let Some(ctx) = &self.ctx {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Err(e) => self.updates.install = Install::Failed(e),
            }
        }
    }
}

/// The Updates dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    if !app.updates.open {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let current = env!("CARGO_PKG_VERSION");
    let can_install = app.update_installer.is_some();
    let mut close = false;
    let mut download: Option<String> = None;
    let mut update: Option<Release> = None;
    let modal = egui::Modal::new(egui::Id::new("updates")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(egui::RichText::new(tl!("Check for updates")).font(theme::semibold(16.0)));
        ui.add_space(8.0);
        let mut offer: Option<Release> = None;
        match &app.updates.check {
            Check::Idle => {
                ui.label(tl!("No check has run yet."));
            }
            #[cfg(not(target_arch = "wasm32"))]
            Check::Running(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tl!("Checking for a newer version…"));
                });
            }
            Check::Done(Ok(r)) if is_newer(&r.version, current) => {
                let version = r.version.trim_start_matches(['v', 'V']);
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Sintec.PDF {v} is available."), &[("v", version)])).strong());
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("You have version {c}."), &[("c", current)])).color(t.text_muted));
                offer = Some(r.clone());
            }
            Check::Done(Ok(_)) => {
                ui.label(crate::i18n::fmt(tl!("Sintec.PDF {c} is up to date."), &[("c", current)]));
            }
            Check::Done(Err(e)) => {
                ui.label(crate::i18n::fmt(tl!("Couldn't check for updates: {e}"), &[("e", &e.to_string())]));
                ui.label(
                    egui::RichText::new(crate::i18n::fmt(
                        tl!("You have version {c}. All releases are listed at {page}."),
                        &[("c", current), ("page", RELEASES_PAGE)],
                    ))
                    .color(t.text_muted),
                );
            }
        }
        let installing = match &app.updates.install {
            #[cfg(not(target_arch = "wasm32"))]
            Install::Running(_) => {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tl!("Downloading and checking the update…"));
                });
                true
            }
            Install::Failed(e) => {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Couldn't update: {e}"), &[("e", e)])).color(ui.visuals().error_fg_color));
                false
            }
            Install::Idle => false,
        };
        ui.add_space(10.0);
        let installable = can_install && offer.as_ref().is_some_and(|r| r.installer.is_some());
        ui.label(
            egui::RichText::new(if installable {
                tl!("Update downloads the new installer, checks it and restarts Sintec.PDF on the new version. Download opens the release page.")
            } else {
                tl!("Asks GitHub for the latest release. Nothing is downloaded or installed until you choose.")
            })
            .color(t.text_muted)
            .small(),
        );
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(r) = offer {
                ui.add_enabled_ui(!installing, |ui| {
                    if installable && widgets::pill_button(ui, tl!("Update"), true).clicked() {
                        update = Some(r.clone());
                    }
                    if widgets::pill_button(ui, tl!("Download"), !installable).clicked() {
                        download = Some(r.url.clone());
                        close = true;
                    }
                    if widgets::pill_button(ui, tl!("Later"), false).clicked() {
                        close = true;
                    }
                });
            } else if widgets::pill_button(ui, tl!("Close"), true).clicked() {
                close = true;
            }
        });
    });
    if modal.should_close() && !matches!(app.updates.install, Install::Failed(_)) {
        close = true;
        download = None;
    }
    if let Some(r) = update {
        app.start_update(r);
    }
    if close {
        app.updates.open = false;
        if matches!(app.updates.install, Install::Failed(_)) {
            app.updates.install = Install::Idle;
        }
        if let Some(url) = download {
            app.open_url(&url);
        }
    }
}
