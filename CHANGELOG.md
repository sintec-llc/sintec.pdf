# Changelog

User-visible changes in Sintec.PDF, newest first. Our versions are independent of upstream
PdfCraft's; each entry says which upstream version the code is based on.

## 0.3.0 — based on upstream PdfCraft v0.9.0

This release was developed as 0.2.7; the user decided the post-sync release is 0.3.0, so the
0.2.7 work ships here.

### Ours
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
