# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with the app icon and the
`Applications` link side by side. `package.sh` copies these files into the image; nothing here is
generated at build time, so the DMG builds with a plain `hdiutil create -srcfolder` (no Finder
scripting on CI, nothing extra installed in the signing job).

| File | What |
|---|---|
| `background.tiff` | Sintec.PDF's background, drawn by `generate.py`: the logo and the name on a panel at the left, an orange arrow from the app to «Программы» under a Russian hint (Inter). 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (128-colour palette, for the logo's gradient; Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, Sintec PDF.app at (326, 205), `Applications` at (574, 205), and the background. Goes to `.DS_Store` in the image; named so it isn't mistaken for (or ignored like) a Finder-generated `.DS_Store`. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the logo, Inter and the layout above. |

## The volume name has no version

The mounted volume is called `Sintec.PDF`, not `Sintec.PDF <version>`. The layout points at the background
through an alias that includes the volume name, and Finder resolves it by that name: with a
versioned name the window keeps its size and icon positions but shows no background (tested).
The DMG file name (`sintec-pdf-<version>-macos-<arch>.dmg`) still carries the version.

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a background,
  so the area under both icons stays light (Paper).
- **Nothing goes inside the icon boxes:** artwork keeps 10 pt clear of each 128 pt icon box and of
  the label strip under it.

## Regenerate

Edit the drawing or the layout constants in `generate.py`, then run, on any OS:

```sh
pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
python3 packaging/macos/dmg/generate.py
```

Both outputs carry nothing from the machine that ran it: the background is drawn from files in
`assets/` only, and `dmg-layout.DS_Store` is written from scratch, its background alias holding
only the volume name and `/.background/background.tiff`. The window is 660 × 432 with Finder's
32 pt title bar; the content area is 660 × 400. After regenerating, update the TIFF's sha256 in
`ATTRIBUTION.toml` and run `cargo xtask assets --write`.

Based on VectorCraft's `packaging/macos/dmg/` (storytold/vectorcraft#493).
