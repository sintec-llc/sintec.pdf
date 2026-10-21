//! Shared helpers for the ui-egui integration tests.

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
