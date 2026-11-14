# Changelog

User-visible changes in Sintec.PDF, newest first. Our versions are independent of upstream
PdfCraft's; each entry says which upstream version the code is based on.

## 0.4.0 — based on upstream PdfCraft v0.21.1 (main `d749fd806`, one commit after the tag)

This release moves the base from upstream v0.9.0 to v0.21.1 and carries our features onto it.

### Upstream (v0.10 – v0.21.1)
- Everything upstream PdfCraft added since v0.9.0, among it printing with the printer's colour
  profile, black-and-white images compressed with CCITT Group 4 or JBIG2, and the reorganised
  interface.

### Ours, carried onto the new base
- **Updates** come only from our releases (`github.com/sintec-llc/sintec.pdf`) through the shared
  updater: the installer is checked against the release's `SHA256SUMS.txt` and the Windows MSI is
  installed with Windows Installer (`msiexec`, with a log), never through PowerShell. The update
  prompt names Sintec.PDF and Sintec LLC.
- **Settings, crash recovery and the clipboard folder** stay under `Sintec.PDF` (on Windows
  `%APPDATA%\Sintec.PDF` and `%LOCALAPPDATA%\Sintec.PDF`), so unsaved work and preferences from
  0.3.x are found after the update.
- **Text editing** (font substitution, the floating text panel, text frames and grips),
  **compression levels** (Strong / Medium / Light on top of upstream's optimizer), **Excel and Word
  export**, **Convert to PDF** from Office, **tabs between windows** and **printing** (A4 by
  default, automatic orientation) work as in 0.3.1.
- **Pages between documents:** drag thumbnails from the Pages panel onto another document's tab;
  resting there opens it, and its Pages panel shows where they go ("Copy here", Shift: "Move
  here"). Releasing on the tab adds them at the end. A move never empties the original. Each side
  is undone on its own. **Extract and save as…** (Pages menu, the Pages panel and page grid menus)
  saves the selected pages as a new PDF named "<document> (p. 2-4, 7).pdf". Both are also
  automation tools (`page_transfer`, `page_extract_save_as`).
- **Interface languages:** English and Russian; the translations live in this repository.
- **Installers:** the Windows installer says "Edit with Sintec.PDF" in the PDF right-click menu
  (it said "Edit with PdfCraft"); `--help` names `sintec-pdf` and `sintec-pdf-cli`.

### Also carried over from 0.3.1
- **Close in white** (permanent white masking, `redact.mask`) is back.
- **Scan into document…** is back in the Pages menu: scan from the scanner into the open document
  or a new one.
- **Explorer ▸ Convert to PDF** works again for several selected files at once
  (`--convert-to-pdf`).
- **Linux Flatpak** files use the Sintec id `io.github.sintec_llc.sintec_pdf`.

## 0.3.1 — based on upstream PdfCraft v0.9.0

A maintenance release; no feature changes.

### Fixed
- **Tab tear-off:** a window that closes now reliably removes its tab-strip entry, so other windows
  no longer see a stale peer to drop tabs onto.
- **Security:** `crypto-bigint` updated to 0.7.10 (RUSTSEC-2026-0387).
- **Tests and CI:** the Linux, macOS and cargo-deny checks that were red after 0.3.0 are green again.

## 0.3.0 — based on upstream PdfCraft v0.9.0

This release was developed as 0.2.7; the user decided the post-sync release is 0.3.0, so the
0.2.7 work ships here.

### Ours
- **Linux x86_64:** CI tests on Linux and releases ship `sintec-pdf-<version>-linux-x86_64.AppImage`,
  a `.deb` and a CLI tarball. The app installs as `sintec-pdf` with its own desktop entry and icon
  (`io.github.sintec_llc.sintec_pdf`, English and Russian) and opens PDFs from the file manager.
  Scanning uses SANE (`scanimage`) and printing CUPS. On Linux the update check opens the release
  page instead of downloading a Windows installer.
- **Reduce file size:** choose Strong, Medium or Light, with each level's estimated size shown
  before you pick (`~2.1 MB (64% smaller)`); Cancel writes nothing and the notice gives the real
  sizes. Light is lossless; Medium takes images to 150 ppi at JPEG quality 75; Strong to 100 ppi
  at quality 55 and drops metadata and other extras (`docs/file-size-levels.md`).
- Every optimization now recompresses streams losslessly at the best Flate level with PNG
  predictors for images, packs objects tighter, and keeps a chart or screenshot lossless when
  that is smaller than JPEG. Optimize PDF with a Discard User Data choice no longer fails on a
  file that has none of that data.
- **Text panel:** the floating text panel shows the paragraph's font, how much of it the document
  carries and whether it is installed, and judges "installed" from the same faces the Font
  substitution dialog uses. Replace font… lists installed faces in order of match.
- **Handles:** text boxes have resize and move grips in the editor.
- **Tabs:** tear a tab off into its own window and drag it back; middle-click closes a tab, also
  in a torn-off window; a ghost of the dragged tab follows the pointer. Upstream's in-strip tab
  reordering uses the same drag.
- **Bundled fonts:** the installer carries the open-licence fonts used for editing and
  substitution.
- **Print:** a match-paper-size option, and label/A6 paper still prints correctly beside it.
  Printing on Windows keeps our own path (sheets rendered to images, printed through .NET).
- **Scanning:** «Scan another page» keeps its dialog when the next page arrives at once.
- English and Russian only; 253 more Russian strings for the new upstream interface.

### From upstream 0.8.0 and 0.9.0
- Batch CLI, text-only auto-tag, opening documents in the background, standard-14 font metrics,
  GNOME header bar, a build label.
- 0.9.0: new text-editing interface (edit text, canvas, panels), print, forms, files, window
  chrome and dialogs, signature validation, accessibility checker, credits and calendar contexts.
- Fixes from both releases, including the security fix #1631 (synced earlier with 0.7.1).
- The installer keeps the desktop shortcut on update only while it is still there (#1058).
