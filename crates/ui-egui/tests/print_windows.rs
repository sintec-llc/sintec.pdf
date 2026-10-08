//! Printing on Windows, end to end: the Print dialog's printer list comes from Windows, and a
//! document goes through the real spooler path (render the print-ready sheets, print them with
//! the .NET print system). The job is sent to "Microsoft Print to PDF" writing into a file, so no
//! paper is used; skipped when that printer isn't installed.

#![cfg(windows)]

use pdfcraft_ui_egui::PdfCraftApp;

const PDF_PRINTER: &str = "Microsoft Print to PDF";

#[test]
fn printers_come_from_windows() {
    let printers = pdfcraft_engine::print::spool::printers();
    if !printers.iter().any(|p| p.name == PDF_PRINTER) {
        eprintln!("skipped: {PDF_PRINTER} not installed ({printers:?})");
        return;
    }
    assert!(printers.iter().filter(|p| p.default).count() <= 1, "{printers:?}");
}

#[test]
fn a_document_prints_through_the_windows_spooler() {
    if !pdfcraft_engine::print::spool::printers().iter().any(|p| p.name == PDF_PRINTER) {
        eprintln!("skipped: {PDF_PRINTER} not installed");
        return;
    }
    let out = std::env::temp_dir().join(format!("sintec-print-test-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let mut app = PdfCraftApp::new();
    app.run_inline = true;
    // Three A4 pages.
    let bytes = app.session.create_blank(595.0, 842.0, 3).unwrap();
    app.open_bytes("three.pdf", None, bytes.as_ref().clone()).unwrap();
    app.open_print();
    assert!(app.print_draft.printers.iter().any(|p| p.name == PDF_PRINTER), "{:?}", app.print_draft.printers);
    app.print_draft.printer = Some(PDF_PRINTER.into());
    app.print_file_override = Some(out.to_string_lossy().into_owned());
    assert!(app.print_now(), "the job was accepted");
    assert!(app.print_run.is_none(), "inline: done and reported");
    let printed = std::fs::read(&out).expect("the driver wrote the printed file");
    assert!(printed.starts_with(b"%PDF"), "a PDF came out");
    let info = pdfcraft_render::inspect(std::sync::Arc::new(printed), None).expect("readable");
    assert_eq!(info.pages.len(), 3, "one printed page per sheet");
    let p = &info.pages[0];
    assert!((p.width - 595.0).abs() < 3.0 && (p.height - 842.0).abs() < 3.0, "A4 portrait paper: {}x{}", p.width, p.height);
    let _ = std::fs::remove_file(out);
}
