# Syncing with upstream PdfCraft

Sintec.PDF is a fork of [PdfCraft](https://github.com/storytold/pdfcraft). Upstream moves very fast
(about 40 commits a day in October 2026, a release every few days), so we merge it **every week**.

## Cadence

- **Weekly (Mondays):** `.github/workflows/upstream-sync.yml` merges upstream's **newest release
  tag**, not its `main`: tags are upstream's tested points, and its `main` has had long red-CI
  stretches. A clean merge becomes a draft pull request `sync/upstream-<tag>` (CI is started on it);
  a conflicting one becomes an issue listing the files. It can also be run by hand (Actions ▸
  Upstream sync ▸ Run workflow).
- **Right away, out of cycle:** upstream crash or security fixes (cherry-pick them), and the
  milestones that touch what we changed, which get their own sync branch and extra testing:
  - **M10** (OCR beyond Latin, Office import, Windows printing): see [M10](#m10-compare-and-keep-the-better-one).
  - **M2** (upstream's own renderer replacing `hayro`): our print rasterising and page preview call
    the renderer.

## Merging by hand

```sh
git fetch upstream --tags
git switch -c sync/upstream-vX.Y.Z main
git merge --no-ff vX.Y.Z
```

Resolve conflicts by these rules (they cover most of them):

| File | Rule |
|---|---|
| `Cargo.toml` | keep **our** version numbers (workspace and internal crates); take their other changes |
| `Cargo.lock` | take theirs, then `cargo update --workspace` |
| `README.md`, `NOTICE`, `.github/` | keep ours |
| `ATTRIBUTION.toml` | keep both sides' entries as separate records; then refresh the sha256 values and run `cargo xtask assets --write` |
| `crates/ui-egui/src/i18n/*.tsv`, the `LANGUAGES` list | keep **both** sides' entries (and fix the array length) |
| code | merge by hand; prefer upstream's design and re-apply our behaviour on top |

Then:

1. `cargo xtask rebrand`: names the product Sintec.PDF in upstream's new strings and catalogs.
2. Translate upstream's new UI strings into Russian (`ru.tsv`): `cargo test -p pdfcraft-ui-egui
   --test sintec_guard` lists every one that is missing.
3. Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `cargo xtask assets`.
4. Set our next version (`cargo xtask version set 0.x.y`); our versions are independent of
   upstream's. Release notes say which upstream version we are based on.
5. Build the installer (`pwsh packaging/windows/package.ps1 -Arch x64`), install it and try it.
6. Pull request into `main`; release from the `release` branch.

## What the guards check (`crates/ui-egui/tests/sintec_guard.rs`)

- Every command, tool and `tl!` label has a Russian translation.
- "PdfCraft" doesn't appear as the product name in code strings or catalogs (identifiers, PDF
  markers, comments and credit lines are fine), no ArtCraft/Discord links, and `docs/brand/` (the
  ArtCraft marks, which forks must not ship) stays gone.

## M10: compare and keep the better one

When upstream ships its own Windows printing, non-Latin OCR or Office import, **compare both
implementations and keep the one that is more polished and effective** (owner's decision,
2026-10-08). Compare on:

- **Results:** Windows printing on real printers (paper size, duplex, colour, page placement);
  OCR accuracy on Russian scans (character error rate, word boxes, search/copy in the result);
  Office documents (formatting, tables, images; Word, Excel, PowerPoint).
- **Robustness:** password-protected and broken files, machines without Office, no network.
- **Speed and resources:** time per page or document, memory, installer size.
- **Code:** tests, maintainability, and how much it costs us to keep differing from upstream.

Ours, for reference: printing renders sheets and prints them through .NET (`crates/print/src/spool.rs`);
OCR uses PaddleOCR PP-OCRv5 for Cyrillic (`crates/ocr/src/paddle.rs`); Office documents are
converted by the installed Office (`crates/engine/src/office.rs`). If upstream's version wins,
take it and drop ours; if ours wins, keep it (and consider offering it upstream).
