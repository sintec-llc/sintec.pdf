#!/usr/bin/env bash
# Build and package Sintec.PDF for Linux (<arch> is x86_64 or aarch64):
#
#   $DIST/sintec-pdf-<version>-linux-<arch>.AppImage        any distro with glibc >= the build host's
#   $DIST/sintec-pdf-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/sintec-pdf-<version>-linux-<arch>.deb             Debian, Ubuntu, Mint, Astra, ...
#   $DIST/sintec-pdf-<version>-linux-<arch>.rpm             Fedora, openSUSE, RED OS, ALT, ...
#   $DIST/sintec-pdf-<version>-linux-<arch>.tar.gz          plain FHS-style tree (bin/, share/)
#   $DIST/sintec-pdf-cli-<version>-linux-<arch>.tar.gz      the headless CLI alone (servers, CI)
#
# Cargo builds `pdfcraft` and `pdfcraft-cli` (the upstream crate names); they ship as `sintec-pdf`
# and `sintec-pdf-cli`, as on Windows and macOS. The freedesktop app id (desktop entry, icon, Wayland
# app id / X11 WM_CLASS, AppStream id) is io.github.sintec_llc.sintec_pdf, and the shared files go
# to share/sintec-pdf, so nothing clashes with an upstream PdfCraft installed beside it.
#
# A local or patched build: set PDFCRAFT_BUILD_LABEL (e.g. `titlebar`). `--version` and About then
# show `0.3.0 (titlebar)`, file names carry `-titlebar`, and the .deb/.rpm release becomes
# `1.titlebar`, which upgrades over the release of the same version (and the next release over it).
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar cli"]
#
# Needs: cargo, curl and the network (OCR models and substitute fonts: cargo xtask models/fonts);
# nfpm for deb/rpm (https://nfpm.goreleaser.com); appimagetool for the AppImage (downloaded into
# $CARGO_TARGET_DIR if missing). Build on an old distro (CI: Ubuntu 22.04, glibc 2.35) so the
# binaries run on newer ones. Optional: desktop-file-validate, appstreamcli, zsyncmake (the zsync
# package) for the AppImage's .zsync.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=io.github.sintec_llc.sintec_pdf
# The hicolor icons keep the upstream file name in assets/ (assets/app-icon/README.md); they are
# installed under the app id.
ICON_STEM=ai.storyteller.pdfcraft
# Where the app looks for its OCR models and substitute fonts from <bin>:
# pdfcraft_ocr::Models::dirs_beside_exe and pdfcraft_engine::text_fonts::bundled_directories.
SHARE=sintec-pdf

SKIP_BUILD=0
FORMATS="appimage deb rpm tar cli"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,26p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export PDFCRAFT_MAINTAINER="${PDFCRAFT_MAINTAINER:-Sintec LLC <https://github.com/sintec-llc/sintec.pdf>}"
LABEL_SUFFIX="${PDFCRAFT_BUILD_LABEL:+-$PDFCRAFT_BUILD_LABEL}"
BASENAME="sintec-pdf-$VERSION$LABEL_SUFFIX-linux-$ARCH"

echo "==> Sintec.PDF $VERSION$LABEL_SUFFIX for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p pdfcraft -p pdfcraft-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# The licences the Windows and macOS packages ship: the project's, NOTICE, the PDF file icon's
# (ISC) and the fonts built into the binary (OFL); plus craft-fonts' through copy_docs.
copy_sintec_licences() {
  local dest="$1" f
  for f in LICENSE-MIT LICENSE-APACHE NOTICE assets/app-icon/LICENSE-document.txt \
    assets/fonts/OFL-Inter.txt assets/fonts/OFL-JetBrainsMono.txt \
    assets/fonts/OFL-DancingScript.txt assets/fonts/OFL-MarckScript.txt; do
    cp "$ROOT/$f" "$dest/"
  done
}

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/pdfcraft" "$STAGE/usr/bin/sintec-pdf"
install -Dm755 "$BIN/pdfcraft-cli" "$STAGE/usr/bin/sintec-pdf-cli"
strip "$STAGE/usr/bin/sintec-pdf" "$STAGE/usr/bin/sintec-pdf-cli" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$PDFCRAFT_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
for icon in "$ROOT"/assets/app-icon/hicolor/*/apps/"$ICON_STEM.png"; do
  size="$(basename "$(dirname "$(dirname "$icon")")")"
  install -Dm644 "$icon" "$STAGE/usr/share/icons/hicolor/$size/apps/$APP_ID.png"
done
mkdir -p "$STAGE/usr/share/doc/sintec-pdf"
copy_docs "$STAGE/usr/share/doc/sintec-pdf"
copy_sintec_licences "$STAGE/usr/share/doc/sintec-pdf"
# OCR models and substitute fonts: <bin>/../share/sintec-pdf/{models,fonts}, in every format below.
stage_models "$STAGE/usr/share/$SHARE/models"
stage_fonts "$STAGE/usr/share/$SHARE/fonts"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- CLI-only .tar.gz ---------------------------------------------------------------------------
# The stripped CLI (and its opt-in MCP server) with the licences, for machines that never open a
# window. Like the other formats it needs glibc >= the build host's.
if has cli; then
  CLI_NAME="sintec-pdf-cli-$VERSION$LABEL_SUFFIX-linux-$ARCH"
  CLI_DIR="$WORK/cli/$CLI_NAME"
  mkdir -p "$CLI_DIR"
  cp "$STAGE/usr/bin/sintec-pdf-cli" "$CLI_DIR/"
  copy_docs "$CLI_DIR"
  copy_sintec_licences "$CLI_DIR"
  tar -C "$WORK/cli" -czf "$DIST/$CLI_NAME.tar.gz" "$CLI_NAME"
  echo "wrote $DIST/$CLI_NAME.tar.gz"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export RELEASE="1${PDFCRAFT_BUILD_LABEL:+.$PDFCRAFT_BUILD_LABEL}"
  export NFPM_ARCH="$DEB_ARCH"
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPDIR="$WORK/Sintec.PDF.AppDir"
  cp -R "$STAGE" "$APPDIR"
  mv "$APPDIR/usr/share/doc" "$WORK/doc-unused"
  ln -s usr/bin/sintec-pdf "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$ICON_STEM.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [ -z "$TOOL" ]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool-$ARCH.AppImage"
    if [ ! -x "$TOOL" ]; then
      curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
      chmod +x "$TOOL"
    fi
  fi
  # Absolute, because appimagetool runs in $DIST below (CARGO_TARGET_DIR or APPIMAGETOOL may be
  # relative, e.g. target/agent-<name>).
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  # A .zsync left by an earlier run would hide a missing zsyncmake and describe another file.
  rm -f "$OUT.zsync"
  # Update information: AppImageUpdate, AppImageLauncher and the like read it from the file and
  # fetch only the blocks that changed in a newer release, through the .zsync published next to
  # each AppImage on GitHub Releases. `latest` is the newest published release that is not a
  # pre-release.
  REPO="${GITHUB_REPOSITORY:-sintec-llc/sintec.pdf}"
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|sintec-pdf-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run: works without FUSE (containers, CI). The output embeds the static runtime,
  # so users don't need libfuse2 either. With zsyncmake on the host (CI installs the zsync
  # package) appimagetool also writes the .zsync, into its working directory, hence the cd.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [ -s "$OUT.zsync" ]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

"$STAGE/usr/bin/sintec-pdf-cli" --version
echo "==> done"
ls -lh "$DIST"
