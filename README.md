<h1 align="center">Sintec.PDF</h1>

<p align="center">
  <b>A fast, private PDF workbench for Windows — with a Russian interface and Russian OCR built in.</b><br>
  Read, annotate, fill, sign, organize, combine, split, secure and OCR PDFs. Everything runs locally:
  no account, no telemetry, no cloud.
</p>

<p align="center">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-12a58a">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-0a7563">
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/runs%20on-Windows%2010%20%C2%B7%2011-12a58a">
</p>

Sintec.PDF is maintained by Sintec LLC and based on the open-source
[PdfCraft](https://github.com/storytold/pdfcraft) by the ArtCraft team.

## Install

Download the latest `sintec-pdf-<version>-windows-x64.msi` from
[Releases](https://github.com/sintec-llc/sintec.pdf/releases) and run it. One file, nothing else to
download: no Rust, no Visual C++ redistributable, no .NET. The OCR models are inside the installer,
so text recognition works offline straight away.

- **Silent install** (from an elevated prompt, or through your deployment tool):
  `msiexec /i sintec-pdf-<version>-windows-x64.msi /qn`
  Add `INSTALLDESKTOPSHORTCUT=0` to skip the desktop shortcut.
- **Silent uninstall:** `msiexec /x sintec-pdf-<version>-windows-x64.msi /qn` (also elevated).
- **No install at all:** unzip `sintec-pdf-<version>-windows-x64-portable.zip` and run
  `sintec-pdf.exe`.

The installer is per-machine, so installing and removing it need administrator rights. Until
releases are code-signed, Windows SmartScreen shows a warning the first time.

**Requirements:** Windows 10 or 11 (x64; x86 and ARM64 builds can be made too). The app draws with
DirectX 12, falling back to OpenGL; on machines without a GPU, Windows' software renderer is used.

## What it does

- **Read and review:** fast rendering, search, bookmarks, thumbnails, comments and markup, stamps,
  measurement tools, light and dark themes.
- **Organize:** insert, delete, rotate, reorder, extract, replace and split pages; combine files.
- **Forms and signing:** fill and create forms (including scripted ones), sign and certify with
  digital IDs, validate signatures.
- **Protect:** passwords and permissions, redaction, sanitising.
- **Create and export:** PDFs from images, text or the clipboard; pages as PNG, JPEG or TIFF;
  text, Word, HTML and RTF export; optimise file size.
- **Scan & OCR:** turn scanned pages into searchable, copyable PDFs.
  - **Russian, Ukrainian and Belarusian** (PaddleOCR PP-OCRv5 East Slavic models), with Latin
    letters and digits mixed in.
  - **English** and other Latin-alphabet text (ocrs models).
- **Interface languages:** Russian, English, Japanese, Simplified and Traditional Chinese, Czech
  and Brazilian Portuguese. A Russian Windows starts in Russian automatically; change it under
  Preferences.
- **Automation:** `sintec-pdf-cli` scripts every tool from the command line or as an MCP server.

## Build from source

You need a recent stable [Rust](https://rustup.rs/) toolchain.

```
cargo xtask models        # fetch the OCR models (SHA-256 verified) into assets/models/
cargo run -p pdfcraft     # run the app
cargo test --workspace    # run the tests
```

To build the installer: install the WiX Toolset v5 (`dotnet tool install --global wix --version 5.0.2`)
and PowerShell 7, then run `pwsh packaging/windows/package.ps1 -Arch x64`. The MSI and the portable
zip land in `dist/release/`.

The Rust crates keep their upstream `pdfcraft-*` names; only what users see is called Sintec.PDF.

## License and credits

Sintec.PDF is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Copyright (c) 2026 Sintec LLC; based on PdfCraft, Copyright (c) 2026 ArtCraft Team and the
PdfCraft contributors. Required notices are in [NOTICE](NOTICE).

Bundled fonts, icons, OCR models and other assets keep their own open licences; each one is listed
with its author, source and licence in [ATTRIBUTION.md](ATTRIBUTION.md).

Sintec.PDF is not made, sponsored or endorsed by the ArtCraft Team.

<sub>Adobe and Acrobat are trademarks or registered trademarks of Adobe Inc. in the United States
and/or other countries. Sintec.PDF is not affiliated with, sponsored by or endorsed by Adobe Inc.</sub>
