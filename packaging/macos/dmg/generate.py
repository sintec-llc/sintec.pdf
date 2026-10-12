#!/usr/bin/env python3
"""Regenerate background.tiff and dmg-layout.DS_Store for the macOS DMG window (see README.md here).

Draws the background with Pillow from the app logo and Inter (both in assets/). Needs:
    pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3

Runs on any OS, no Mac needed. Both outputs come from this script alone, so they carry nothing
from the machine that made them: no local paths, user or disk names, dates or volume UUIDs.

    python3 packaging/macos/dmg/generate.py
"""

import datetime
import os
import sys

from ds_store import DSStore
from mac_alias import Alias, TargetInfo, VolumeInfo
from PIL import Image, ImageCms, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))

# Keep in sync with package.sh (volume name) and README.md (layout).
VOLUME = "Sintec.PDF"
WIDTH, HEIGHT = 660, 400  # window content, pt; the background's 1x size
TITLE_BAR = 32
ICON_SIZE = 128
PALETTE = 128  # colours in background.tiff (the logo tile has a gradient)
ICONS = {"Sintec.PDF.app": (326, 205), "Applications": (574, 205)}
# A fixed date for the alias and the colour profile. Finder never matches it against the image
# (each build is a new volume): it finds the background by volume name and path.
FIXED_DATE = datetime.datetime(2026, 10, 8, tzinfo=datetime.timezone.utc)


PAPER = (247, 245, 241)
INK = (38, 38, 38)  # the logo tile's dark grey, for text
SHELF = (236, 232, 225)  # the left panel, a shade darker than the paper
ORANGE = (252, 154, 26)  # Sintec orange, as the logo's S
MUTED = (110, 106, 100)
PANEL = 232  # width of the dark panel on the left, pt


def font(name, size):
    return ImageFont.truetype(os.path.join(ROOT, "assets", "fonts", name), size)


def render(scale):
    """The window background at `scale` (1 = 660 x 400 px): a panel with the logo and the
    name, and on the paper an orange arrow from the app to Applications under a hint. Nothing
    enters the icon boxes or the label strips under them."""
    k = 4 * scale  # draw at 4x and scale down for smooth edges
    im = Image.new("RGB", (WIDTH * k, HEIGHT * k), PAPER)
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, PANEL * k, HEIGHT * k], fill=SHELF)
    logo = Image.open(os.path.join(ROOT, "assets", "app-icon", "sintec-pdf-logo.png")).convert("RGBA")
    size = 132 * k
    im.paste(logo.resize((size, size), Image.LANCZOS), ((PANEL * k - size) // 2, 104 * k), logo.resize((size, size), Image.LANCZOS))
    d.text((PANEL * k / 2, 268 * k), "Sintec.PDF", font=font("Inter-SemiBold.ttf", 26 * k), fill=INK, anchor="mm")
    d.text((PANEL * k / 2, 300 * k), "PDF для работы", font=font("Inter-Regular.ttf", 14 * k), fill=MUTED, anchor="mm")
    centre = (PANEL + WIDTH) / 2
    d.text((centre * k, 62 * k), "Установка", font=font("Inter-SemiBold.ttf", 20 * k), fill=INK, anchor="mm")
    d.text((centre * k, 92 * k), "Перетащите Sintec.PDF в «Программы»", font=font("Inter-Regular.ttf", 15 * k), fill=MUTED, anchor="mm")
    # The arrow between the icon boxes (x 390-510), 12 pt clear of each.
    y, x0, x1 = 205 * k, 404 * k, 496 * k
    d.line([(x0, y), (x1 - 14 * k, y)], fill=ORANGE, width=6 * k)
    d.ellipse([x0 - 3 * k, y - 3 * k, x0 + 3 * k, y + 3 * k], fill=ORANGE)
    d.polygon([(x1, y), (x1 - 20 * k, y - 13 * k), (x1 - 20 * k, y + 13 * k)], fill=ORANGE)
    return im.resize((WIDTH * scale, HEIGHT * scale), Image.LANCZOS)


def quantize(im):
    # No dithering: dither noise compresses badly and the artwork has flat fills.
    return im.quantize(PALETTE, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)


def srgb_profile():
    # LittleCMS's built-in sRGB ("No copyright, use freely"), with its creation date fixed.
    icc = bytearray(ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes())
    d = FIXED_DATE
    icc[24:36] = b"".join(v.to_bytes(2, "big") for v in (d.year, d.month, d.day, 0, 0, 0))
    if icc[84:100] != bytes(16):
        sys.exit("error: the sRGB profile has an ID; recompute it after fixing the date")
    return bytes(icc)


def write_tiff(out):
    """1x (72 dpi) and 2x (144 dpi) in one TIFF, as `tiffutil -cathidpicheck` writes them."""
    one, two = (quantize(render(s)) for s in (1, 2))
    two.encoderinfo = {"dpi": (144, 144)}  # the rest as the first page
    # Opaque 16-colour palette, Deflate: the artwork is three inks plus antialiasing, and a palette
    # keeps the file small (RGB was ~230 KB; repository assets must stay small). As PhotoCraft's.
    one.save(
        out,
        "TIFF",
        save_all=True,
        append_images=[two],
        compression="tiff_adobe_deflate",  # brand-ok: Pillow's name for TIFF compression 8 (Deflate)
        dpi=(72, 72),
        icc_profile=srgb_profile(),
    )
    with Image.open(out) as im:
        im.seek(1)
        if im.info.get("dpi") != (144, 144):
            sys.exit("error: the 2x page isn't 144 dpi (needs Pillow 12 or later)")
        strips = []
        for page in (0, 1):
            im.seek(page)
            strips += zip(im.tag_v2[273], im.tag_v2[279])  # StripOffsets, StripByteCounts
    # TIFF keeps directories and tag data on even offsets. When a strip ends on an odd one, libtiff
    # leaves the padding byte after it uninitialised, so the file changed from run to run with
    # identical pixels. Zero it: nothing refers to that byte.
    starts = {o for o, _ in strips}
    with open(out, "r+b") as f:
        for o, n in strips:
            if (o + n) % 2 and o + n not in starts:
                f.seek(o + n)
                f.write(bytes(1))


def background_alias():
    # What Finder stores for /.background/background.tiff on the volume, minus anything about the
    # machine (dmgbuild writes the same kind of alias from a mounted image). Disk type 5 (ejectable),
    # the volume flags and the folder/file ids (25, 26) are what Finder wrote on a test image; the ids
    # are only hints, as the volume's creation date.
    volume = VolumeInfo(VOLUME, FIXED_DATE, b"H+", 5, 0x0D02, b"\0\0")
    volume.posix_path = f"/Volumes/{VOLUME}"
    target = TargetInfo(0, "background.tiff", 25, 26, FIXED_DATE, b"\0\0\0\0", b"\0\0\0\0")
    target.folder_name = ".background"
    target.cnid_path = [25]
    target.carbon_path = f"{VOLUME}:.background:\0background.tiff"
    target.posix_path = "/.background/background.tiff"
    return Alias(volume=volume, target=target).to_bytes()


def write_ds_store(out):
    if os.path.exists(out):
        os.remove(out)
    with DSStore.open(out, "w+") as d:
        d["."]["bwsp"] = {
            "ShowStatusBar": False,
            "ShowToolbar": False,
            "ShowTabView": False,
            "ContainerShowSidebar": False,
            "ShowSidebar": False,
            "WindowBounds": f"{{{{200, 528}}, {{{WIDTH}, {HEIGHT + TITLE_BAR}}}}}",
        }
        d["."]["icvp"] = {
            "viewOptionsVersion": 1,
            "backgroundType": 2,
            "backgroundImageAlias": background_alias(),
            "backgroundColorRed": 1.0,
            "backgroundColorGreen": 1.0,
            "backgroundColorBlue": 1.0,
            "gridOffsetX": 0.0,
            "gridOffsetY": 0.0,
            "gridSpacing": 100.0,
            "arrangeBy": "none",
            "showIconPreview": True,
            "showItemInfo": False,
            "labelOnBottom": True,
            "textSize": 16.0,
            "iconSize": float(ICON_SIZE),
        }
        d["."]["vSrn"] = ("long", 1)
        for name, pos in ICONS.items():
            d[name]["Iloc"] = pos


def main():
    write_tiff(os.path.join(HERE, "background.tiff"))
    write_ds_store(os.path.join(HERE, "dmg-layout.DS_Store"))


if __name__ == "__main__":
    main()
