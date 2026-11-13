//! Sintec.PDF guards for the weekly upstream sync (docs/upstream-sync.md):
//! - the Russian catalogs (`i18n/locales/ru/`) cover every command, tool and `tl!` UI label, so
//!   strings upstream adds don't silently show in English;
//! - the interface offers English and Russian only;
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

fn tsv_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            tsv_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "tsv") {
            out.push(p);
        }
    }
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
    // Upstream's ArtCraft community labels go away with its links (see
    // `the_artcraft_links_stay_out_of_the_app`); the catalogs never name ArtCraft.
    missing.retain(|l| !l.contains("ArtCraft"));
    assert!(missing.is_empty(), "untranslated in i18n/locales/ru/pdfcraft.tsv (add them, see docs/upstream-sync.md): {missing:#?}");
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

/// Code lines (not comments or credit lines) of `crates/` and `apps/` that `bad` matches.
fn code_lines(root: &Path, bad: impl Fn(&str) -> bool) -> Vec<String> {
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
            if bad(line) {
                found.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn the_upstream_name_and_marks_stay_out_of_the_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = code_lines(&root, names_upstream);
    let mut catalogs = Vec::new();
    tsv_files(&root.join("i18n"), &mut catalogs);
    assert!(!catalogs.is_empty(), "no catalogs under i18n/");
    for path in catalogs {
        let text = std::fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            if !line.starts_with('#') && !allowed(line) && (names_upstream(line) || line.contains("ArtCraft")) {
                found.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "upstream branding in the app (run the rebrand step, docs/upstream-sync.md):\n{}", found.join("\n"));
    assert!(!root.join("docs/brand").exists(), "the ArtCraft marks (docs/brand/) must not come back");
}

/// The ArtCraft community links (Discord, website) stay out of the Help menu and home screen.
#[test]
fn the_artcraft_links_stay_out_of_the_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let found = code_lines(&root, |line| line.contains("discord.gg/artcraft") || line.contains("getartcraft.com"));
    assert!(found.is_empty(), "ArtCraft links in the app:\n{}", found.join("\n"));
}

/// Sintec.PDF offers English and Russian, and Russian is embedded (`i18n/locales/ru/`).
#[test]
fn the_interface_is_english_and_russian() {
    let codes: Vec<&str> = Lang::all().map(Lang::code).collect();
    assert_eq!(codes, ["en", "ru"]);
    let ru = Lang::from_code("ru").expect("ru registered");
    assert_eq!(ru.name(), "Русский");
    assert_eq!(pdfcraft_ui_egui::i18n::tr(ru, "Open"), "Открыть");
    let dates: Vec<&str> = pdfcraft_engine::dates::DATE_LANGUAGES.iter().map(|l| l.code).collect();
    assert_eq!(dates, ["en", "ru"]);
    let locales: BTreeSet<String> = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../i18n/locales"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(locales.iter().all(|l| l == "en" || l == "ru"), "only English and Russian catalogs: {locales:?}");
}

/// No internal milestone codes ("M4", "(M6)", "этапе M7") in Russian text the app still uses;
/// the web-only attachment note is the one place left (the desktop app never shows it).
#[test]
fn russian_text_shows_no_internal_milestones() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let sources: Vec<String> = files.iter().filter_map(|f| std::fs::read_to_string(f).ok()).collect();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../i18n/locales/ru");
    let ru = ["common.tsv", "pdfcraft.tsv"].map(|f| std::fs::read_to_string(dir.join(f)).unwrap()).join("\n");
    let bad: Vec<&str> = ru
        .lines()
        .filter(|l| {
            let mut cols = l.split('\t');
            let (_context, english, russian) = (cols.next(), cols.next().unwrap_or(""), cols.next().unwrap_or(""));
            let milestone = russian
                .split(|c: char| !c.is_alphanumeric() && c != '.')
                .any(|w| w.strip_prefix('M').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '.')));
            milestone && !english.contains("on the web") && sources.iter().any(|s| s.contains(&format!("\"{english}\"")))
        })
        .collect();
    assert!(bad.is_empty(), "milestone codes in Russian text the app uses: {bad:#?}");
}
