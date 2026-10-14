# Localization parity

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** major (English and Russian only) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)

Status of Sintec.PDF's interface languages. How translations work: [localization.md](localization.md).
Summary in [ROADMAP.md](../ROADMAP.md).

Sintec.PDF ships two interface languages, English and Russian. The other catalogs inherited from
PdfCraft (Japanese, Chinese, Czech, Portuguese, German, Spanish, French, Bulgarian, Telugu,
Hungarian, Ukrainian, Italian, Arabic, Polish) were removed on 2026-10-13: the product's users work
in Russian and English.

## How this was measured

- **Strings translated** is measured from `crates/ui-egui/src/i18n/ru.tsv`. The tests check that it
  covers every registered command, every All tools label and every `tl!("…")` literal in the UI
  source, so a new English label can't ship untranslated.
- **Not counted:** error details from the engine and the operating system, which are shown in
  English inside a translated frame; there is no translated user documentation or help yet (Acrobat
  ships localized help).

## Languages

| Language | Code | UI strings translated | Dialogs / tooltips / help | Script support | Status | Acrobat ships it |
|---|---|---|---|---|---|---|
| English | `en` | built in (100%) | all; README and docs | Latin | **full** | yes |
| Russian | `ru` | ≈ 2,190 entries; every command, All tools label and `tl!` literal | dialogs and tooltips through `tl!`; no help | Cyrillic, drawn by the bundled Inter and JetBrains Mono | partial (engine error details in English, no help) | yes |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | major | English and Russian only: the other 15 catalogs, the Arabic right-to-left label support and the Chinese-first font order were removed |
| 2026-10-10 | major | Created. Catalog counts measured from `crates/ui-egui/src/i18n/*.tsv` against the 2,143-key union; Acrobat's localizations from its bundle's `.lproj` and Sequences folder names |
