//! `cargo xtask rebrand`: after merging upstream PdfCraft (docs/upstream-sync.md), name the
//! product Sintec.PDF wherever upstream's new code strings and translation catalogs say
//! "PdfCraft". It follows the rule `crates/ui-egui/tests/sintec_guard.rs` enforces: identifiers
//! (`PdfCraftApp`), PDF markers (`%PdfCraft`, `PdfCraft-Identity-UCS`), paths and URLs, comments
//! and credit lines ("PdfCraft contributors", "Based on PdfCraft") are left alone.
//! `--check` lists what would change without writing.

use std::path::{Path, PathBuf};

const UPSTREAM: &str = "PdfCraft";
const OURS: &str = "Sintec.PDF";

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p.ends_with("target") {
                files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs" || x == "tsv") && !p.ends_with("sintec_guard.rs") && !p.ends_with("rebrand.rs") {
            out.push(p);
        }
    }
}

fn allowed(line: &str) -> bool {
    ["contributors", "Based on PdfCraft", "based on PdfCraft", "ArtCraft team", "storytold"].iter().any(|k| line.contains(k))
}

/// `line` with every standalone product-name word replaced.
pub fn rebrand_line(line: &str) -> String {
    let b = line.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = String::with_capacity(line.len());
    let mut last = 0;
    for (i, _) in line.match_indices(UPSTREAM) {
        let before = i.checked_sub(1).and_then(|j| b.get(j)).copied();
        let after = b.get(i + UPSTREAM.len()).copied();
        if before.is_some_and(|c| word(c) || c == b'%' || c == b'/') || after.is_some_and(|c| word(c) || c == b'-') {
            continue;
        }
        out.push_str(line.get(last..i).unwrap_or_default());
        out.push_str(OURS);
        last = i + UPSTREAM.len();
    }
    out.push_str(line.get(last..).unwrap_or_default());
    out
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let check = args.iter().any(|a| a == "--check");
    let root = crate::gates::root();
    let mut all = Vec::new();
    for dir in ["crates", "apps"] {
        files(&root.join(dir), &mut all);
    }
    let mut changed = 0;
    for f in all {
        let text = std::fs::read_to_string(&f)?;
        let tsv = f.extension().is_some_and(|x| x == "tsv");
        let mut touched = false;
        let lines: Vec<String> = text
            .split('\n')
            .map(|l| {
                let comment = if tsv { l.starts_with('#') } else { l.trim_start().starts_with("//") };
                if comment || allowed(l) {
                    return l.to_string();
                }
                let new = rebrand_line(l);
                if new != l {
                    touched = true;
                    changed += 1;
                    println!("{}: {}", f.strip_prefix(&root).unwrap_or(&f).display(), new.trim());
                }
                new
            })
            .collect();
        if touched && !check {
            std::fs::write(&f, lines.join("\n"))?;
        }
    }
    println!("rebrand: {changed} line(s) {}", if check { "would change" } else { "changed" });
    if check && changed > 0 {
        anyhow::bail!("upstream branding found; run `cargo xtask rebrand`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::rebrand_line;

    #[test]
    fn only_the_product_name_changes() {
        assert_eq!(rebrand_line(r#"tl!("Welcome to PdfCraft")"#), r#"tl!("Welcome to Sintec.PDF")"#);
        assert_eq!(rebrand_line("\tAbout PdfCraft\tAcerca de PdfCraft"), "\tAbout Sintec.PDF\tAcerca de Sintec.PDF");
        for keep in ["PdfCraftApp::new()", "q %PdfCraft\\n", "/CMapName /PdfCraft-Identity-UCS", "https://github.com/storytold/PdfCraft"] {
            assert_eq!(rebrand_line(keep), keep);
        }
    }
}
