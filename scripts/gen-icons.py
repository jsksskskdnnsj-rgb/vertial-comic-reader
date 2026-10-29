#!/usr/bin/env python3
"""Draw the MH Reader app icon and write every size Tauri bundles.

The mark is a soft-light glass tile (white falling into the mint accent) with a
comic page: a teal header, stacked panels, and a downward chevron for vertical
scroll. Sizes at or above 64px also carry an MH monogram. Smaller sizes (tray
16/32, taskbar) are drawn with the same silhouette and no lettering, so the
glyph stays a few solid shapes instead of a scaled-down illustration.

This script always overwrites src-tauri/icons/. It is deterministic (no photos,
no fonts) and is the source of the bytes stored in scripts/restore-binaries.json.
Run from the repo root:

    python scripts/gen-icons.py

Requires Pillow. Outputs:

    icon.png             512
    128x128.png          128
    128x128@2x.png       256
    32x32.png            32
    icon.ico             16, 24, 32, 48, 64, 128, 256
    icon.icns            16 through 1024, including the @2x PNG entries
"""
import io
import os
import struct

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src-tauri", "icons")

# Soft-light tokens (mh-ui.css): accent #80e5b6, accent-ink #25695a.
INK = (37, 105, 90, 255)
INK_DEEP = (20, 78, 66, 255)
MINT = (128, 229, 182, 255)
PAGE = (255, 252, 248, 255)
WHITE = (255, 255, 255, 255)

ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
# Apple Icon Image PNG entries. The @2x types repeat a size under a second OSType.
ICNS_ENTRIES = (
    (16, b"icp4"),
    (32, b"icp5"),
    (32, b"ic11"),
    (64, b"icp6"),
    (64, b"ic12"),
    (128, b"ic07"),
    (256, b"ic08"),
    (256, b"ic13"),
    (512, b"ic09"),
    (512, b"ic14"),
    (1024, b"ic10"),
    (1024, b"ic15"),
)


def _lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(4))


def _vgrad(w, h, top, bot):
    col = Image.new("RGBA", (1, max(1, h)))
    px = col.load()
    last = max(1, h - 1)
    for y in range(h):
        px[0, y] = _lerp(top, bot, y / last)
    return col.resize((w, h), Image.NEAREST)


def _mask(size, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return m


def _mh(d, x, y, w, h, fill):
    """Geometric MH. Coordinates are the tight bounding box."""
    gap = w * 0.10
    lw = (w - gap) / 2.0
    _m(d, x, y, lw, h, fill)
    _h(d, x + lw + gap, y, lw, h, fill)


def _m(d, x, y, w, h, fill):
    sw = max(1.0, h * 0.22)
    # Outer silhouette of a bold M, then the inner valley is the page showing through
    # because we paint the letter as a filled polygon (no counters).
    d.polygon(
        [
            (x, y + h),
            (x + sw, y + h),
            (x + sw, y + h * 0.46),
            (x + w * 0.50, y + h * 0.78),
            (x + w - sw, y + h * 0.46),
            (x + w - sw, y + h),
            (x + w, y + h),
            (x + w, y),
            (x + w - sw * 1.15, y),
            (x + w * 0.50, y + h * 0.42),
            (x + sw * 1.15, y),
            (x, y),
        ],
        fill=fill,
    )


def _h(d, x, y, w, h, fill):
    sw = max(1.0, h * 0.22)
    mid = y + h * 0.50
    d.rectangle((x, y, x + sw, y + h), fill=fill)
    d.rectangle((x + w - sw, y, x + w, y + h), fill=fill)
    d.rectangle((x, mid - sw * 0.48, x + w, mid + sw * 0.48), fill=fill)


def _chevron(d, cx, top, width, height, fill):
    arm = width * 0.34
    d.polygon(
        [
            (cx - width / 2, top),
            (cx - width / 2 + arm, top),
            (cx, top + height - arm * 0.15),
            (cx + width / 2 - arm, top),
            (cx + width / 2, top),
            (cx, top + height),
        ],
        fill=fill,
    )


def _tile(c):
    """Soft-light squircle: white falling into mint, with an ink rim and a top sheen."""
    u = c / 512.0
    radius = int(118 * u)
    plate = _vgrad(c, c, (255, 255, 255, 255), MINT)
    img = Image.new("RGBA", (c, c), (0, 0, 0, 0))
    rim = Image.new("RGBA", (c, c), (0, 0, 0, 0))
    ImageDraw.Draw(rim).rounded_rectangle((0, 0, c - 1, c - 1), radius=radius, fill=INK)
    img.alpha_composite(rim)
    inset = max(1, int(8 * u))
    inner = plate.resize((c - inset * 2, c - inset * 2), Image.LANCZOS)
    inner.putalpha(_mask(c - inset * 2, max(1, radius - inset)))
    img.paste(inner, (inset, inset), inner)
    sheen = Image.new("RGBA", (c, c), (0, 0, 0, 0))
    ImageDraw.Draw(sheen).ellipse(
        (int(-0.15 * c), int(-0.42 * c), int(1.15 * c), int(0.58 * c)),
        fill=(255, 255, 255, 110),
    )
    sheen.putalpha(Image.composite(sheen.getchannel("A"), Image.new("L", (c, c), 0), _mask(c, radius)))
    img.alpha_composite(sheen)
    return img


def _page(img, px0, py0, px1, py1, pr, u):
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (px0, py0 + int(10 * u), px1, py1 + int(12 * u)),
        radius=pr,
        fill=(20, 70, 60, 70),
    )
    img.alpha_composite(shadow)
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((px0, py0, px1, py1), radius=pr, fill=PAGE)
    return d


def draw_icon(size: int) -> Image.Image:
    """Raster `size` px.

    Below 48px the page is larger and holds only a header plus one chevron —
    two shapes that still exist at 16px. From 48px up the page carries the MH
    monogram, two panel rules, and the chevron.
    """
    ss = 8 if size <= 32 else 4 if size <= 128 else 2
    c = size * ss
    u = c / 512.0
    img = _tile(c)
    compact = size < 48

    if compact:
        px0, py0, px1, py1 = int(58 * u), int(52 * u), int(454 * u), int(460 * u)
    else:
        px0, py0, px1, py1 = int(92 * u), int(74 * u), int(420 * u), int(440 * u)
    pr = int((52 if compact else 44) * u)
    d = _page(img, px0, py0, px1, py1, pr, u)

    # Spine of the page underneath, so the mark reads as a stack rather than a single card.
    if size >= 24:
        edge = max(1, int((14 if compact else 10) * u))
        d.rounded_rectangle(
            (px0 - edge, py0 + int(28 * u), px0 + int(4 * u), py1 - int(28 * u)),
            radius=max(1, edge // 2),
            fill=INK_DEEP,
        )

    page_h = py1 - py0
    header_h = int(page_h * (0.42 if compact else 0.34))
    d.rounded_rectangle((px0, py0, px1, py0 + header_h), radius=pr, fill=INK)
    d.rectangle((px0, py0 + max(0, header_h - pr), px1, py0 + header_h), fill=INK)

    if compact:
        bar_h = max(2, int(header_h * 0.28))
        bar_y = py0 + (header_h - bar_h) // 2
        d.rounded_rectangle(
            (px0 + int(56 * u), bar_y, px1 - int(56 * u), bar_y + bar_h),
            radius=bar_h // 2,
            fill=WHITE,
        )
        chev_w = int((px1 - px0) * 0.46)
        chev_h = int((py1 - (py0 + header_h)) * 0.46)
        _chevron(d, (px0 + px1) // 2, py0 + header_h + int((py1 - py0 - header_h - chev_h) * 0.42), chev_w, chev_h, INK)
    else:
        _mh(d, px0 + int(54 * u), py0 + int(header_h * 0.22), int(220 * u), int(header_h * 0.58), WHITE)
        body_top = py0 + header_h
        left = px0 + int(40 * u)
        right = px1 - int(40 * u)
        rule_h = max(2, int(20 * u))
        gap = int(26 * u)
        y = body_top + int(32 * u)
        for width_frac in (1.0, 0.70):
            d.rounded_rectangle(
                (left, y, left + int((right - left) * width_frac), y + rule_h),
                radius=rule_h // 2,
                fill=INK,
            )
            y += rule_h + gap
        chev_w = int(96 * u)
        chev_h = int(56 * u)
        _chevron(d, (px0 + px1) // 2, py1 - int(86 * u), chev_w, chev_h, INK)

    if c != size:
        img = img.resize((size, size), Image.LANCZOS)
    return img


def _png_bytes(im: Image.Image) -> bytes:
    buf = io.BytesIO()
    im.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def write_icns(path, images):
    chunks = []
    for size, ostype in ICNS_ENTRIES:
        data = _png_bytes(images[size])
        chunks.append(ostype + struct.pack(">I", 8 + len(data)) + data)
    body = b"".join(chunks)
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def write_ico(path, images):
    images[256].save(
        path,
        format="ICO",
        sizes=[(s, s) for s in ICO_SIZES],
        append_images=[images[s] for s in ICO_SIZES if s != 256],
    )


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    needed = {16, 24, 32, 48, 64, 128, 256, 512, 1024}
    images = {s: draw_icon(s) for s in sorted(needed)}
    files = {
        "icon.png": images[512],
        "128x128.png": images[128],
        "128x128@2x.png": images[256],
        "32x32.png": images[32],
    }
    for name, im in files.items():
        dest = os.path.join(OUT, name)
        im.save(dest, format="PNG", optimize=True)
        print(f"wrote {name} ({im.size[0]}x{im.size[1]})")
    write_ico(os.path.join(OUT, "icon.ico"), images)
    print("wrote icon.ico " + ",".join(str(s) for s in ICO_SIZES))
    write_icns(os.path.join(OUT, "icon.icns"), images)
    print("wrote icon.icns")


if __name__ == "__main__":
    main()
