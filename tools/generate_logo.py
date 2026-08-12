#!/usr/bin/env python3
"""Generates the Carbon app logo in every format the bundlers need.

The geometry below is the single source of truth: this script emits both the
SVG and every raster size from it, so the vector art and the icons cannot drift
apart. Rendering is stdlib-only (`zlib` for PNG, `iconutil` for `.icns`) because
the project has no image toolchain and adding one for a nine-shape logo is not
worth the dependency.

    python3 tools/generate_logo.py

Writes `src-tauri/icons/` and `public/logo.svg` (served at `/logo.svg`, so the
favicon and the in-app mark share one file with no bundler plumbing).

The mark: a speech bubble with a download arrow knocked out of it — a
conversation being written to disk. Three shapes, so it survives 16x16.
"""

from __future__ import annotations

import struct
import subprocess
import sys
import zlib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ICONS = REPO / "src-tauri" / "icons"
SVG_OUT = REPO / "public" / "logo.svg"

# Design space. Every coordinate below is in a 1024x1024 canvas.
CANVAS = 1024

# The tile is inset rather than bleeding to the edge: macOS renders app icons
# with their own shape and expects that margin.
TILE = (48.0, 48.0, 976.0, 976.0)
TILE_R = 208.0

BUBBLE = (232.0, 228.0, 792.0, 660.0)
BUBBLE_R = 116.0
TAIL = ((352.0, 632.0), (352.0, 798.0), (474.0, 648.0))

# Arrow, knocked out of the bubble. The head's base sits above the stem's foot
# so the two merge into one shape instead of meeting at a seam.
STEM = (476.0, 292.0, 548.0, 492.0)
HEAD = ((398.0, 462.0), (626.0, 462.0), (512.0, 598.0))

CLAY_TOP = (216, 124, 94)
CLAY_BOTTOM = (176, 79, 47)
BONE = (255, 253, 250)

SAMPLES = 4  # per axis, so 16 samples per pixel


def lerp(a: float, b: float, t: float) -> float:
    return a + (b - a) * t


def tile_color(y: float) -> tuple[int, int, int]:
    """Vertical gradient across the tile, clamped outside it."""
    span = TILE[3] - TILE[1]
    t = min(1.0, max(0.0, (y - TILE[1]) / span))
    return tuple(round(lerp(CLAY_TOP[i], CLAY_BOTTOM[i], t)) for i in range(3))


def in_rounded_rect(x: float, y: float, rect, r: float) -> bool:
    x0, y0, x1, y1 = rect
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    # Only the four corner boxes need the radius test.
    cx = x0 + r if x < x0 + r else (x1 - r if x > x1 - r else x)
    cy = y0 + r if y < y0 + r else (y1 - r if y > y1 - r else y)
    if cx == x or cy == y:
        return True
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def in_triangle(x: float, y: float, tri) -> bool:
    (ax, ay), (bx, by), (cx, cy) = tri

    def side(px, py, qx, qy):
        return (qx - px) * (y - py) - (qy - py) * (x - px)

    d1, d2, d3 = side(ax, ay, bx, by), side(bx, by, cx, cy), side(cx, cy, ax, ay)
    has_neg = d1 < 0 or d2 < 0 or d3 < 0
    has_pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (has_neg and has_pos)


def in_rect(x: float, y: float, rect) -> bool:
    x0, y0, x1, y1 = rect
    return x0 <= x <= x1 and y0 <= y <= y1


def sample(x: float, y: float):
    """Colour of one sample point, or None where the icon is transparent."""
    if not in_rounded_rect(x, y, TILE, TILE_R):
        return None
    on_bubble = in_rounded_rect(x, y, BUBBLE, BUBBLE_R) or in_triangle(x, y, TAIL)
    if on_bubble and not (in_rect(x, y, STEM) or in_triangle(x, y, HEAD)):
        return BONE
    return tile_color(y)


def render(size: int) -> bytes:
    """Renders RGBA bytes at `size`, supersampled for clean edges."""
    scale = CANVAS / size
    step = scale / SAMPLES
    offset = step / 2.0
    total = SAMPLES * SAMPLES
    out = bytearray(size * size * 4)

    for py in range(size):
        base_y = py * scale
        row = py * size * 4
        for px in range(size):
            base_x = px * scale
            r = g = b = a = 0
            for sy in range(SAMPLES):
                y = base_y + offset + sy * step
                for sx in range(SAMPLES):
                    hit = sample(base_x + offset + sx * step, y)
                    if hit is not None:
                        r += hit[0]
                        g += hit[1]
                        b += hit[2]
                        a += 255
            i = row + px * 4
            if a:
                # Average over covered samples only, so edge pixels keep the
                # shape's colour instead of darkening toward transparent black.
                covered = a / 255
                out[i] = round(r / covered)
                out[i + 1] = round(g / covered)
                out[i + 2] = round(b / covered)
                out[i + 3] = round(a / total)
    return bytes(out)


def png_bytes(size: int, rgba: bytes) -> bytes:
    raw = b"".join(
        b"\x00" + rgba[y * size * 4 : (y + 1) * size * 4] for y in range(size)
    )

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def bmp_entry(size: int, rgba: bytes) -> bytes:
    """32-bit BMP DIB for an ICO entry.

    Small sizes use BMP rather than PNG because some Windows shell paths still
    read only the DIB form; 128px and up use PNG to keep the file small.
    """
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    pixels = bytearray()
    for y in range(size - 1, -1, -1):  # BMP rows run bottom-up
        for x in range(size):
            i = (y * size + x) * 4
            pixels += bytes(
                (rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3])  # BGRA
            )
    mask_row = ((size + 31) // 32) * 4  # AND mask padded to 4 bytes
    return header + bytes(pixels) + b"\x00" * (mask_row * size)


def write_ico(path: Path, images: dict[int, bytes]) -> None:
    entries, blobs = [], []
    offset = 6 + 16 * len(images)
    for size in sorted(images):
        rgba = images[size]
        data = png_bytes(size, rgba) if size >= 128 else bmp_entry(size, rgba)
        entries.append(
            struct.pack(
                "<BBBBHHII",
                size if size < 256 else 0,  # 0 means 256 in the ICO header
                size if size < 256 else 0,
                0,
                0,
                1,
                32,
                len(data),
                offset,
            )
        )
        blobs.append(data)
        offset += len(data)
    path.write_bytes(
        struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries) + b"".join(blobs)
    )


def svg_source() -> str:
    x0, y0, x1, y1 = TILE
    bx0, by0, bx1, by1 = BUBBLE
    sx0, sy0, sx1, sy1 = STEM
    tail = " ".join(f"{x:g},{y:g}" for x, y in TAIL)
    head = " ".join(f"{x:g},{y:g}" for x, y in HEAD)
    top = "#%02X%02X%02X" % CLAY_TOP
    bottom = "#%02X%02X%02X" % CLAY_BOTTOM
    bone = "#%02X%02X%02X" % BONE
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS} {CANVAS}" role="img" aria-label="Carbon">
  <title>Carbon</title>
  <defs>
    <linearGradient id="clay" x1="0" y1="{y0:g}" x2="0" y2="{y1:g}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="{top}"/>
      <stop offset="1" stop-color="{bottom}"/>
    </linearGradient>
    <mask id="knockout">
      <rect x="0" y="0" width="{CANVAS}" height="{CANVAS}" fill="black"/>
      <rect x="{bx0:g}" y="{by0:g}" width="{bx1 - bx0:g}" height="{by1 - by0:g}" rx="{BUBBLE_R:g}" fill="white"/>
      <polygon points="{tail}" fill="white"/>
      <rect x="{sx0:g}" y="{sy0:g}" width="{sx1 - sx0:g}" height="{sy1 - sy0:g}" fill="black"/>
      <polygon points="{head}" fill="black"/>
    </mask>
  </defs>
  <rect x="{x0:g}" y="{y0:g}" width="{x1 - x0:g}" height="{y1 - y0:g}" rx="{TILE_R:g}" fill="url(#clay)"/>
  <rect x="0" y="0" width="{CANVAS}" height="{CANVAS}" fill="{bone}" mask="url(#knockout)"/>
</svg>
"""


# Tauri's bundlers read these names; the .iconset members are what iconutil wants.
PNG_TARGETS = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 1024,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}
ICNS_MEMBERS = {
    "icon_16x16.png": 16,
    "icon_16x16@2x.png": 32,
    "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64,
    "icon_128x128.png": 128,
    "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256,
    "icon_256x256@2x.png": 512,
    "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}
ICO_SIZES = (16, 32, 48, 64, 128, 256)


def main() -> int:
    ICONS.mkdir(parents=True, exist_ok=True)
    SVG_OUT.parent.mkdir(parents=True, exist_ok=True)

    needed = sorted(
        set(PNG_TARGETS.values()) | set(ICNS_MEMBERS.values()) | set(ICO_SIZES)
    )
    rendered: dict[int, bytes] = {}
    for size in needed:
        print(f"  rendering {size}x{size}", flush=True)
        rendered[size] = render(size)

    for name, size in PNG_TARGETS.items():
        (ICONS / name).write_bytes(png_bytes(size, rendered[size]))

    write_ico(ICONS / "icon.ico", {s: rendered[s] for s in ICO_SIZES})

    iconset = ICONS / "icon.iconset"
    iconset.mkdir(exist_ok=True)
    for name, size in ICNS_MEMBERS.items():
        (iconset / name).write_bytes(png_bytes(size, rendered[size]))
    if sys.platform == "darwin":
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS / "icon.icns")],
            check=True,
        )
        for member in iconset.iterdir():
            member.unlink()
        iconset.rmdir()
    else:
        print("note: .icns needs macOS `iconutil`; left icon.iconset/ in place")

    SVG_OUT.write_text(svg_source())
    print(f"wrote {ICONS} and {SVG_OUT.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
