//! Convert to PDF (Windows Explorer ▸ «Преобразовать в PDF»): turn a selection of files (images,
//! text files, Office documents, PDFs) into one PDF, in natural name order, opened as a new,
//! unsaved document for the user to arrange and save.
//! Office documents go through the installed office suite ([`crate::office`]). Files that can't be
//! converted are skipped and reported, not fatal.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::Session;

/// Image types converted to one page each.
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "gif", "bmp", "jp2", "j2k", "jpx"];
/// Text types set in the embedded monospaced font (any script it covers).
pub const TEXT_EXTS: &[&str] = &["txt", "text", "md", "csv", "log", "ini", "cfg", "json", "xml", "yml", "yaml"];
/// Every type the context-menu entry is offered for.
pub fn supported_exts() -> impl Iterator<Item = &'static str> {
    std::iter::once("pdf").chain(IMAGE_EXTS.iter().copied()).chain(TEXT_EXTS.iter().copied()).chain(crate::office::exts())
}

/// The result of a conversion.
#[derive(Debug)]
pub struct Converted {
    pub bytes: Arc<Vec<u8>>,
    /// Files left out, with why.
    pub skipped: Vec<(String, String)>,
    /// How many files went in.
    pub used: usize,
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Compare names the way people number files: "scan2" before "scan10", case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The name for the converted document: the file's when there is one, the folder's when there
/// are several ("Счета.pdf").
pub fn output_name(paths: &[PathBuf]) -> String {
    let first = paths.first();
    let stem = if paths.len() == 1 {
        first.and_then(|f| f.file_stem()).map(|s| s.to_string_lossy().into_owned())
    } else {
        first.and_then(|f| f.parent()).and_then(Path::file_name).map(|s| s.to_string_lossy().into_owned())
    }
    .filter(|s| !s.trim().is_empty())
    .unwrap_or_else(|| "Документ".to_string());
    format!("{stem}.pdf")
}

/// One file's bytes as a PDF, for Insert pages from a file: PDFs as they are, images, text files
/// and Office documents converted as [`convert_files`] does.
pub fn file_to_pdf(session: &Session, file_name: &str, bytes: Vec<u8>) -> Result<Arc<Vec<u8>>, String> {
    let path = Path::new(file_name);
    let e = ext(path);
    let head = bytes.get(..bytes.len().min(1024)).unwrap_or_default();
    if e == "pdf" || head.windows(5).any(|w| w == b"%PDF-") {
        return Ok(Arc::new(bytes));
    }
    if IMAGE_EXTS.contains(&e.as_str()) {
        return session.create_from_images(&[(file_name.to_string(), bytes)]).map_err(|e| e.to_string());
    }
    if TEXT_EXTS.contains(&e.as_str()) {
        let title = path.file_stem().map_or_else(|| file_name.to_string(), |s| s.to_string_lossy().into_owned());
        return session.create_from_text_file(&title, &bytes).map_err(|e| e.to_string());
    }
    if let Some(kind) = crate::office::kind_for(&e) {
        // Office converts files, not bytes: hand it a temporary copy under the same name.
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("sintec-pdf-insert-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let copy = dir.join(path.file_name().unwrap_or(path.as_os_str()));
        let result = std::fs::write(&copy, &bytes).map_err(|e| e.to_string()).and_then(|()| {
            crate::office::convert(&[(kind, copy.clone())]).into_iter().next().map_or_else(|| Err("not converted".to_string()), |(_, r)| r)
        });
        let _ = std::fs::remove_dir_all(&dir);
        return result.map(Arc::new);
    }
    Err("this file type can't be converted to PDF".to_string())
}

/// Convert `paths` (any order) into one PDF. Errors only when nothing could be converted.
pub fn convert_files(session: &Session, paths: &[PathBuf]) -> Result<Converted, String> {
    let mut sorted: Vec<&PathBuf> = paths.iter().collect();
    sorted.sort_by(|a, b| natural_cmp(&name(a), &name(b)));
    sorted.dedup();
    // Office documents first, in one batch (each Office application starts once).
    let office_inputs: Vec<(crate::office::Kind, PathBuf)> =
        sorted.iter().filter_map(|p| crate::office::kind_for(&ext(p)).map(|k| (k, (*p).clone()))).collect();
    let mut office_done = crate::office::convert(&office_inputs);
    let mut sources: Vec<(String, Arc<Vec<u8>>)> = Vec::new();
    let mut skipped = Vec::new();
    for p in sorted {
        if crate::office::kind_for(&ext(p)).is_some() {
            let n = name(p);
            match office_done.iter().position(|(q, _)| q == p).map(|i| office_done.swap_remove(i).1) {
                Some(Ok(b)) => sources.push((n, Arc::new(b))),
                Some(Err(e)) => skipped.push((n, e)),
                None => skipped.push((n, "not converted".to_string())),
            }
            continue;
        }
        let n = name(p);
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(e) => {
                skipped.push((n, e.to_string()));
                continue;
            }
        };
        let e = ext(p);
        let one = if e == "pdf" {
            Ok(Arc::new(bytes))
        } else if IMAGE_EXTS.contains(&e.as_str()) {
            session.create_from_images(&[(n.clone(), bytes)]).map_err(|e| e.to_string())
        } else if TEXT_EXTS.contains(&e.as_str()) {
            let title = p.file_stem().map_or_else(|| n.clone(), |s| s.to_string_lossy().into_owned());
            session.create_from_text_file(&title, &bytes).map_err(|e| e.to_string())
        } else {
            Err("this file type can't be converted to PDF".to_string())
        };
        match one {
            Ok(b) => sources.push((n, b)),
            Err(e) => skipped.push((n, e)),
        }
    }
    let used = sources.len();
    let bytes = match sources.len() {
        0 => return Err(skipped.iter().map(|(n, e)| format!("{n}: {e}")).collect::<Vec<_>>().join("; ")),
        1 => sources.into_iter().next().map(|(_, b)| b).ok_or("nothing to convert")?,
        _ => match session.combine(&sources) {
            Ok(b) => b,
            // One unreadable PDF shouldn't sink the rest: retry without the ones that fail alone.
            Err(_) => {
                let mut ok = Vec::new();
                for (n, b) in sources {
                    match session.combine(std::slice::from_ref(&(n.clone(), b.clone()))) {
                        Ok(_) => ok.push((n, b)),
                        Err(e) => skipped.push((n, e.to_string())),
                    }
                }
                if ok.is_empty() {
                    return Err(skipped.iter().map(|(n, e)| format!("{n}: {e}")).collect::<Vec<_>>().join("; "));
                }
                session.combine(&ok).map_err(|e| e.to_string())?
            }
        },
    };
    Ok(Converted { bytes, skipped, used })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installer offers «Преобразовать в PDF» for exactly the types the converter handles.
    #[test]
    fn the_installer_menu_matches_the_supported_types() {
        let wxs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/windows/pdfcraft.wxs")).unwrap();
        let mut menu: Vec<String> = wxs
            .lines()
            .filter(|l| l.contains("SintecPDF.ConvertToPdf\\command"))
            .filter_map(|l| l.split("SystemFileAssociations\\.").nth(1)?.split('\\').next().map(str::to_string))
            .collect();
        let mut supported: Vec<String> = supported_exts().map(str::to_string).collect();
        menu.sort();
        supported.sort();
        assert_eq!(menu, supported);
    }

    #[test]
    fn names_sort_like_people_number_them() {
        let mut v = vec!["scan10.jpg", "Scan2.jpg", "scan1.jpg", "notes.txt", "scan02.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["notes.txt", "scan1.jpg", "Scan2.jpg", "scan02.jpg", "scan10.jpg"]);
    }

    #[test]
    fn the_document_is_named_after_the_file_or_the_folder() {
        let folder = PathBuf::from("C:\\Документы").join("Счета");
        assert_eq!(output_name(&[folder.join("акт.jpg")]), "акт.pdf");
        assert_eq!(output_name(&[folder.join("a.jpg"), folder.join("b.txt")]), "Счета.pdf");
        assert_eq!(output_name(&[]), "Документ.pdf");
    }

    #[test]
    fn single_files_convert_for_insert_pages() {
        let s = Session::new();
        let png = crate::export::encode_png(2, 2, &[255; 16]).unwrap();
        let from_image = file_to_pdf(&s, "фото.png", png).unwrap();
        assert!(from_image.starts_with(b"%PDF"));
        let from_text = file_to_pdf(&s, "заметка.txt", "Привет".as_bytes().to_vec()).unwrap();
        assert!(from_text.starts_with(b"%PDF"));
        let pdf = s.create_blank(100.0, 100.0, 1).unwrap();
        assert_eq!(file_to_pdf(&s, "a.pdf", pdf.as_ref().clone()).unwrap(), pdf, "a PDF goes in as it is");
        assert!(file_to_pdf(&s, "archive.zip", b"PK".to_vec()).is_err());
    }

    #[test]
    fn mixed_files_become_one_pdf_in_name_order() {
        let dir = std::env::temp_dir().join(format!("sintec-convert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new();
        // A Russian text file in Windows-1251, an image, a two-page PDF and a broken Word file.
        let cp1251: Vec<u8> = "Привет, мир".chars().map(|c| if c.is_ascii() { c as u8 } else { (c as u32 - 0x410 + 0xC0) as u8 }).collect();
        std::fs::write(dir.join("1 заметка.txt"), &cp1251).unwrap();
        let png = crate::export::encode_png(2, 2, &[255; 16]).unwrap();
        std::fs::write(dir.join("2 фото.png"), png).unwrap();
        let pdf = s.create_blank(300.0, 300.0, 2).unwrap();
        std::fs::write(dir.join("10 договор.pdf"), pdf.as_ref()).unwrap();
        std::fs::write(dir.join("3 таблица.docx"), b"PK\x03\x04").unwrap();
        let paths: Vec<PathBuf> = ["10 договор.pdf", "3 таблица.docx", "2 фото.png", "1 заметка.txt"].iter().map(|n| dir.join(n)).collect();
        let out = convert_files(&s, &paths).unwrap();
        assert_eq!(out.used, 3);
        assert_eq!(out.skipped.len(), 1, "{:?}", out.skipped);
        assert_eq!(out.skipped[0].0, "3 таблица.docx");
        let info = pdfcraft_render::inspect(out.bytes.clone(), None).unwrap();
        assert_eq!(info.pages.len(), 4, "text (1) + image (1) + PDF (2)");
        // The text page reads back as Cyrillic, first (name order: 1, 2, 10).
        let mut r = pdfcraft_render::PageRenderer::new(out.bytes.clone(), pdfcraft_render::RenderConfig::default());
        let t = r.render(pdfcraft_render::RenderRequest { page: 0, kind: pdfcraft_render::RequestKind::Text, scale: 1.0, ..Default::default() });
        let text = t.text.map(|t| t.plain_text()).unwrap_or_default();
        assert!(text.contains("Привет, мир"), "{text:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
