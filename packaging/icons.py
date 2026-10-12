#!/usr/bin/env python3
"""Regenerate the app icons and the Windows PDF file icon from their masters.

  assets/app-icon/sintec-pdf-logo.png   Sintec LLC's Sintec.PDF logo (the app icon master)
  assets/app-icon/pdfcraft-document.png the PDF file icon, composed here (`--document`) from
                                        Lucide's file-text outline and the logo

Needs python3 with Pillow (pip install pillow). The outputs are committed, so building and
packaging never need it. After running it, update the sha256 values in ATTRIBUTION.toml, then
`cargo xtask assets --write`.

  python3 packaging/icons.py [--document]
"""

import io
import os
import struct
import sys

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIR = os.path.join(ROOT, "assets", "app-icon")
ID = "ai.storyteller.pdfcraft"
ORANGE = (252, 154, 26, 255)  # Sintec orange, as the logo's S
LOGO = os.path.join(DIR, "sintec-pdf-logo.png")
DOCUMENT = os.path.join(DIR, "pdfcraft-document.png")


def scaled(img, size):
    return img.resize((size, size), Image.LANCZOS)


def write_png(img, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.save(path, optimize=True)


def write_ico(img, path, sizes=(16, 20, 24, 32, 40, 48, 64, 128, 256)):
    """PNG-compressed .ico entries, each resampled from the master."""
    blobs = []
    for s in sizes:
        buf = io.BytesIO()
        scaled(img, s).save(buf, "PNG", optimize=True)
        blobs.append((s, buf.getvalue()))
    head = struct.pack("<HHH", 0, 1, len(blobs))
    entries, data, offset = b"", b"", 6 + 16 * len(blobs)
    for s, b in blobs:
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), offset)
        data += b
        offset += len(b)
    with open(path, "wb") as f:
        f.write(head + entries + data)


def mac_grid(img):
    """Apple's icon grid: an 824 px body centred on a transparent 1024 px canvas."""
    canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    canvas.alpha_composite(scaled(img, 824), (100, 100))
    return canvas


def arc(cx, cy, r, a0, a1, n=12):
    import math

    return [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)), cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]


def compose_document(logo):
    """The PDF file icon: an orange sheet behind a white page (Lucide file-text's outline and
    fold, in Lucide's 24-unit grid), text lines, and the Sintec.PDF logo as the badge."""
    k = 4  # supersampling
    W = 1024 * k
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = 42 * k  # one Lucide unit

    def page(dx, dy):
        # Lucide file-text: M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8 … 20 8v12a2 2 0 0 1-2 2z
        pts = arc(6, 20, 2, 90, 180) + arc(6, 4, 2, 180, 270) + [(14, 2), (20, 8)] + arc(18, 20, 2, 0, 90)
        return [(dx + x * u, dy + y * u) for x, y in pts]

    stroke = int(0.62 * u)
    # The page spans x 300-972, y 30-870 (Lucide's x 4-20, y 2-22); the sheet behind is the
    # same shape, offset down and to the left.
    fx, fy = 132 * k, -54 * k
    d.polygon(page(fx - 200 * k, fy + 120 * k), fill=ORANGE)
    front = page(fx, fy)
    d.polygon(front, fill=(255, 255, 255, 255))
    d.line(front + [front[0]], fill=ORANGE, width=stroke, joint="curve")
    fold = [(14, 2), (14, 7)] + arc(15, 7, 1, 180, 90) + [(20, 8)]
    d.line([(fx + x * u, fy + y * u) for x, y in fold], fill=ORANGE, width=stroke, joint="curve")
    # Text lines; the lower ones stop short of the badge.
    for y, x1 in ((330, 880), (440, 880), (550, 880), (660, 560), (770, 500)):
        x0, y, x1, r = 400 * k, y * k, x1 * k, int(0.25 * u)
        d.line([(x0, y), (x1, y)], fill=ORANGE, width=2 * r)
        for x in (x0, x1):
            d.ellipse([x - r, y - r, x + r, y + r], fill=ORANGE)
    img = img.resize((1024, 1024), Image.LANCZOS)
    # The badge: the app logo with a thin transparent gap around it.
    badge = 400
    at = 1024 - badge - 8
    gap = Image.new("L", (1024, 1024), 0)
    ImageDraw.Draw(gap).rounded_rectangle([at - 18, at - 18, at + badge + 18, at + badge + 18], radius=110, fill=255)
    img.putalpha(Image.composite(Image.new("L", (1024, 1024), 0), img.getchannel("A"), gap))
    img.alpha_composite(scaled(logo, badge), (at, at))
    return img


def main():
    logo = Image.open(LOGO).convert("RGBA")
    if "--document" in sys.argv:
        write_png(compose_document(logo), DOCUMENT)
    document = Image.open(DOCUMENT).convert("RGBA")
    mac = mac_grid(logo)
    write_png(mac, os.path.join(DIR, "pdfcraft-1024.png"))
    for s in (16, 24, 32, 48, 64, 128, 256, 512):
        write_png(scaled(logo, s), os.path.join(DIR, "hicolor", f"{s}x{s}", "apps", f"{ID}.png"))
    write_ico(logo, os.path.join(DIR, "pdfcraft.ico"))
    write_ico(document, os.path.join(DIR, "pdfcraft-document.ico"))
    mac.save(os.path.join(DIR, "pdfcraft.icns"), sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
    print("icons written to", DIR)


if __name__ == "__main__":
    main()
