//! Shared helpers for the ui-egui integration tests.
#![allow(dead_code)] // each test crate uses only some of these helpers

use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

/// The machine's own fonts must not decide what Bold draws in or whether Edit PDF suggests a
/// substitution: Arial, Times New Roman and Courier New "installed", as regular faces that are not
/// on disk (so a styled edit falls back to the standard PDF fonts).
pub fn standard_faces() -> Vec<pdfcraft_engine::text_fonts::FontFile> {
    ["Arial", "Times New Roman", "Courier New"]
        .map(|family| pdfcraft_engine::text_fonts::FontFile {
            path: std::path::PathBuf::from(format!("/nonexistent/{family}.ttf")),
            family: family.into(),
            face: family.replace(' ', ""),
            style: pdfcraft_fonts::embedding::FaceStyle {
                weight: 400,
                italic: false,
                monospace: family == "Courier New",
                serif: Some(family == "Times New Roman"),
                cyrillic: true,
            },
            widths: Vec::new(),
            bundled: false,
        })
        .to_vec()
}

/// State-based waits for native preparation, which completes between UI frames.
pub fn wait_until(h: &mut Harness<'static, PdfCraftApp>, what: &str, ready: impl Fn(&PdfCraftApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        h.step();
        if ready(h.state()) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}: views={}, active={:?}, password={:?}, toast={:?}",
            h.state().views.len(),
            h.state().active,
            h.state().password_prompt.as_ref().map(|p| (&p.name, &p.error)),
            h.state().toast
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
