//! Manual check of «Преобразовать в PDF»: convert files into one PDF and report what was skipped.
//!
//! cargo run -p pdfcraft-engine --example convert_files -- out.pdf file1 file2 …

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let out = args.next().ok_or("usage: convert_files <out.pdf> <files…>")?;
    let files: Vec<std::path::PathBuf> = args.map(std::path::PathBuf::from).collect();
    let session = pdfcraft_engine::Session::new();
    let started = std::time::Instant::now();
    let done = pdfcraft_engine::convert::convert_files(&session, &files)?;
    std::fs::write(&out, done.bytes.as_ref()).map_err(|e| e.to_string())?;
    println!("{} file(s) in, {} skipped, {:.1} s -> {out}", done.used, done.skipped.len(), started.elapsed().as_secs_f32());
    for (name, why) in done.skipped {
        println!("  skipped {name}: {why}");
    }
    Ok(())
}
