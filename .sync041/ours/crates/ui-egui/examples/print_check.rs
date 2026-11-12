//! Manual check of Windows printing: print a document through the real spooler into a file
//! (Microsoft Print to PDF), then render page 1 of the original and of the printed file side by
//! side into a PNG for a visual comparison.
//!
//! cargo run -p pdfcraft-ui-egui --example print_check -- in.pdf out-dir

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("usage: print_check <in.pdf> <out-dir>")?;
    let dir = std::path::PathBuf::from(args.next().ok_or("usage: print_check <in.pdf> <out-dir>")?);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let printed = dir.join("printed.pdf");
    let _ = std::fs::remove_file(&printed);
    let bytes = std::fs::read(&input).map_err(|e| e.to_string())?;
    let mut app = pdfcraft_ui_egui::PdfCraftApp::new();
    app.run_inline = true;
    app.open_bytes("doc.pdf", None, bytes.clone()).map_err(|e| e.to_string())?;
    app.open_print();
    app.print_draft.printer = Some("Microsoft Print to PDF".into());
    app.print_file_override = Some(printed.to_string_lossy().into_owned());
    if !app.print_now() {
        return Err("print_now failed".into());
    }
    let shot = |b: Vec<u8>, name: &str| -> Result<(), String> {
        let mut r = pdfcraft_render::PageRenderer::new(std::sync::Arc::new(b), pdfcraft_render::RenderConfig::default());
        let p = r.render(pdfcraft_render::RenderRequest { page: 0, scale: 1.0, ..Default::default() });
        let png = pdfcraft_engine::export::encode_png(p.width, p.height, &p.rgba)?;
        std::fs::write(dir.join(name), png).map_err(|e| e.to_string())
    };
    shot(bytes, "original.png")?;
    shot(std::fs::read(&printed).map_err(|e| e.to_string())?, "printed.png")?;
    println!("wrote {}", dir.display());
    Ok(())
}
