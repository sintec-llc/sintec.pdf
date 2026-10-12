# Sintec.PDF app icon

<img src="hicolor/128x128/apps/ai.storyteller.pdfcraft.png" alt="Sintec.PDF app icon: an orange S over a blue PDF on a dark rounded square" width="128">

**Design:** Sintec LLC's Sintec.PDF logo (2026-10): a wide, heavy orange **S** over a blue **PDF** on a
dark-grey rounded square with a soft diagonal gradient.

**Master:** `sintec-pdf-logo.png`, Sintec LLC's artwork as supplied (2133 px), resampled to 1024 px,
full bleed. Windows and Linux icons use the full-bleed tile. macOS icons put it on Apple's grid (an 824 px
body centred on a transparent 1024 px canvas). The file names keep the upstream `pdfcraft` stem because
the build, the MSI and the app refer to them.

**PDF file icon:** `pdfcraft-document.png`, composed by `packaging/icons.py --document`: an orange sheet
behind a white page with Lucide's file-text outline and fold (ISC), text lines, and the app logo as the
badge in the lower right corner, with a thin transparent gap around it.

Licence: [LICENSE.txt](LICENSE.txt) and [LICENSE-document.txt](LICENSE-document.txt) (`MIT OR Apache-2.0`
for the files, plus ISC for the Lucide outline; the Sintec names and logos remain Sintec LLC's trademarks).

## Files

| File | What it is |
|---|---|
| `sintec-pdf-logo.png` | the master; every app-icon PNG, `.ico` and `.icns` is resampled from it |
| `pdfcraft-1024.png` | 1024 px on Apple's grid; also the runtime Dock icon on macOS |
| `pdfcraft.icns` | macOS icon (16–1024 px) |
| `pdfcraft.ico` | Windows icon (16–256 px), embedded in the app exe (shipped as `sintec-pdf.exe`) by `apps/pdfcraft/build.rs` |
| `hicolor/<n>x<n>/apps/ai.storyteller.pdfcraft.png` | Linux hicolor theme, 16–512 px; the 256 px one is the runtime icon on Windows and Linux, the 512 px one the in-app logo |
| `pdfcraft-document.png` | the PDF file icon's master, 1024 px |
| `pdfcraft-document.ico` | Windows PDF file icon (16–256 px), resource 2 of the exe; the installer's PDF file association uses it |

Where it shows: `apps/pdfcraft/src/main.rs` sets the window icon (Dock, taskbar, Alt-Tab, launcher) and the
Wayland app id `ai.storyteller.pdfcraft`; `packaging/linux/ai.storyteller.pdfcraft.desktop` names the
hicolor icon; `crates/ui-egui/src/widgets.rs` shows the logo on the home screen and in About.

## Regenerate

```sh
python3 packaging/icons.py [--document]   # needs Pillow; --document recomposes the PDF file icon
cargo xtask assets                        # then update the sha256 values in ATTRIBUTION.toml and run with --write
```
