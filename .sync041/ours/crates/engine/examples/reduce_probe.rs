//! Manual check of Reduce File Size on real files: what images a PDF has, how large each level
//! makes it and how close the estimate is. Nothing is written.
//!
//! cargo run --release -p pdfcraft-engine --example reduce_probe -- file1.pdf file2.pdf …

use std::sync::Arc;

use pdfcraft_cos::{Document, ObjRef, Object};
use pdfcraft_engine::optimizer::Level;

fn main() -> Result<(), String> {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        println!("== {path}: {} bytes", bytes.len());
        let bytes = Arc::new(bytes);
        images(&bytes);
        let mut s = pdfcraft_engine::Session::new();
        let id = match s.open("probe.pdf", None, bytes.clone(), None) {
            Ok(id) => id,
            Err(e) => {
                println!("  cannot open: {e}");
                continue;
            }
        };
        let mut estimated = Vec::new();
        let t = std::time::Instant::now();
        if let Ok(job) = s.estimate_job(id) {
            let _ = job.run(|| false, |l, b| estimated.push((l, b)));
        }
        println!("  estimate took {:.2} s", t.elapsed().as_secs_f32());
        for l in Level::ALL {
            let t = std::time::Instant::now();
            match s.reduced_bytes(id, l) {
                Ok((out, _)) => {
                    let est = estimated.iter().find(|(x, _)| *x == l).map_or(0, |(_, b)| *b);
                    println!(
                        "  {:<6} {:>10} ({:>5.1}% smaller)  estimate {:>10} ({:+.1}%)  {:.2} s",
                        l.name(),
                        out.len(),
                        100.0 * (1.0 - out.len() as f64 / bytes.len() as f64),
                        est,
                        100.0 * (est as f64 - out.len() as f64) / out.len().max(1) as f64,
                        t.elapsed().as_secs_f32()
                    );
                }
                Err(e) => println!("  {:<6} failed: {e}", l.name()),
            }
        }
    }
    Ok(())
}

/// Every image of the file: size, filter, colour space, bits, stored bytes and drawn resolution.
fn images(bytes: &Arc<Vec<u8>>) {
    let Ok(doc) = Document::open(bytes.clone()) else { return };
    let pages = pdfcraft_annot::page_refs(&doc).unwrap_or_default();
    let ppi = pdfcraft_optimize::effective_resolutions(&doc, &pages);
    for n in doc.object_numbers() {
        let r = ObjRef::new(n, doc.generation(n));
        let Object::Stream(s) = &*doc.get(r) else { continue };
        if s.dict.name(b"Subtype") != Some(b"Image") {
            continue;
        }
        let mut f = Vec::new();
        pdfcraft_cos::serialize(s.dict.get(b"Filter").unwrap_or(&Object::Null), &mut f);
        let mut cs = Vec::new();
        pdfcraft_cos::serialize(&doc.resolve(s.dict.get(b"ColorSpace").unwrap_or(&Object::Null)), &mut cs);
        let cs = String::from_utf8_lossy(&cs);
        println!(
            "  img {:>5}: {}x{} bpc {:?} {} cs {} {} bytes ppi {:.0}{}",
            n,
            s.dict.int(b"Width").unwrap_or(0),
            s.dict.int(b"Height").unwrap_or(0),
            s.dict.int(b"BitsPerComponent"),
            String::from_utf8_lossy(&f),
            cs.chars().take(40).collect::<String>(),
            s.raw.len(),
            ppi.get(&r).copied().unwrap_or(f64::NAN),
            if s.dict.contains(b"SMask") { " smask" } else { "" }
        );
    }
}
