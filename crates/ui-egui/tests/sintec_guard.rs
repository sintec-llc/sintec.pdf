//! Sintec.PDF guards for the weekly upstream sync (docs/upstream-sync.md):
//! - the Russian catalog covers every command, tool and `tl!` UI label, so strings upstream adds
//!   don't silently show in English;
//! - the upstream product name and the ArtCraft marks stay out of what users see.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use pdfcraft_ui_egui::i18n::{Lang, has};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every `tl!("…")` literal in the UI crate's sources (outside the i18n module and test modules).
fn ui_literals() -> BTreeSet<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut out = BTreeSet::new();
    for f in files.iter().filter(|f| !f.components().any(|c| c.as_os_str() == "i18n")) {
        let text = std::fs::read_to_string(f).unwrap().replace("\r\n", "\n");
        let mut rest = text.split("#[cfg(test)]\nmod ").next().unwrap_or_default();
        while let Some((_, after)) = rest.split_once("tl!(\"") {
            let mut escaped = false;
            let Some(end) = after.char_indices().find_map(|(i, c)| {
                if c == '"' && !escaped {
                    return Some(i);
                }
                escaped = c == '\\' && !escaped;
                None
            }) else {
                break;
            };
            let (raw, tail) = after.split_at(end);
            if tail.starts_with("\")")
                && let Ok(label) = serde_json::from_str::<String>(&format!("\"{raw}\""))
            {
                out.insert(label);
            }
            rest = tail.get(1..).unwrap_or_default();
        }
    }
    out
}

#[test]
fn russian_covers_commands_tools_and_ui_labels() {
    let ru = Lang::from_code("ru").expect("ru registered");
    let mut missing = BTreeSet::new();
    for c in pdfcraft_engine::commands::COMMANDS {
        if !has(ru, c.label) {
            missing.insert(c.label.to_string());
        }
        if let Some(m) = c.menu.filter(|m| !has(ru, m)) {
            missing.insert(m.to_string());
        }
    }
    for g in pdfcraft_engine::catalog::TOOL_GROUPS {
        if !has(ru, g.label) {
            missing.insert(g.label.to_string());
        }
        for s in g.sections {
            if !has(ru, s.title) {
                missing.insert(s.title.to_string());
            }
            for i in s.items.iter().filter(|i| !has(ru, i.label)) {
                missing.insert(i.label.to_string());
            }
        }
    }
    let literals = ui_literals();
    assert!(literals.len() > 900, "the source scan found only {} labels", literals.len());
    missing.extend(literals.into_iter().filter(|l| !has(ru, l)));
    assert!(missing.is_empty(), "untranslated in ru.tsv (add them, see docs/upstream-sync.md): {missing:#?}");
}

/// The upstream product name as a word (not inside identifiers such as `PdfCraftApp`, the PDF
/// markers `%PdfCraft` / `PdfCraft-Identity-UCS`, or URLs and paths).
fn names_upstream(line: &str) -> bool {
    let b = line.as_bytes();
    line.match_indices("PdfCraft").any(|(i, _)| {
        let before = i.checked_sub(1).and_then(|j| b.get(j)).copied();
        let after = b.get(i + "PdfCraft".len()).copied();
        let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        !before.is_some_and(|c| word(c) || c == b'%' || c == b'/') && !after.is_some_and(|c| word(c) || c == b'-')
    })
}

/// Lines that may name upstream: credit, copyright and attribution.
fn allowed(line: &str) -> bool {
    ["contributors", "Based on PdfCraft", "based on PdfCraft", "ArtCraft team", "storytold"].iter().any(|k| line.contains(k))
}

#[test]
fn the_upstream_name_and_marks_stay_out_of_the_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates", "apps"] {
        rust_files(&root.join(dir), &mut files);
    }
    let mut found = Vec::new();
    // This guard spells the name to look for it.
    for f in files.iter().filter(|f| !f.ends_with("sintec_guard.rs")) {
        let text = std::fs::read_to_string(f).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || allowed(line) {
                continue;
            }
            if names_upstream(line) || line.contains("discord.gg/artcraft") || line.contains("getartcraft.com") {
                found.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
            }
        }
    }
    let i18n = root.join("crates/ui-egui/src/i18n");
    for e in std::fs::read_dir(&i18n).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "tsv")) {
        let text = std::fs::read_to_string(e.path()).unwrap();
        for (n, line) in text.lines().enumerate() {
            if !line.starts_with('#') && !allowed(line) && (names_upstream(line) || line.contains("ArtCraft")) {
                found.push(format!("{}:{}: {}", e.path().display(), n + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "upstream branding in the app (run the rebrand step, docs/upstream-sync.md):\n{}", found.join("\n"));
    assert!(!root.join("docs/brand").exists(), "the ArtCraft marks (docs/brand/) must not come back");
}
