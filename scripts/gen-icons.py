#!/usr/bin/env python3
"""Draw the MH Reader app icon and write every size Tauri bundles.

The shipped mark is variant C: three comic pages stacked into a little tower,
a soft face on the top page, and yellow stars around it. The tile stays in the
soft-light mint theme (accent #80e5b6, ink #25695a). Small sizes keep the same
silhouette — tower, eyes, stars — and drop the page lines.

Two other drawings stay available:

    python scripts/gen-icons.py                 # variant C (default)
    python scripts/gen-icons.py --variant a     # one chubby page character
    python scripts/gen-icons.py --variant b     # a scroll with a face
    python scripts/gen-icons.py --preview-dir /tmp/icons

Requires Pillow. This script always overwrites src-tauri/icons/. It is
deterministic (no photos, no fonts) and is the source of the icon bytes in
scripts/restore-binaries.json.

    icon.png             512
    128x128.png          128
    128x128@2x.png       256
    32x32.png            32
    icon.ico             16, 24, 32, 48, 64, 128, 256
    icon.icns            16 through 1024, including the @2x PNG entries
"""
import argparse
import io
import math
import os
import struct

from PIL import Image, ImageChops, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src-tauri", "icons")

# Soft-light tokens: accent #80e5b6, accent-ink #25695a, blush #f2a0b5.
INK = (37, 105, 90, 255)
INK_DEEP = (22, 72, 62, 255)
MINT = (128, 229, 182, 255)
MINT_TOP = (214, 248, 232, 255)
PAGE = (255, 252, 246, 255)
WHITE = (255, 255, 255, 255)
STAR = (255, 204, 64, 255)
BLUSH = (242, 160, 181, 195)

ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
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
DEFAULT_VARIANT = "c"
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"


def _lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(4))


def _vgrad(w, h, top, bot):
    col = Image.new("RGBA", (1, max(1, h)))
    px = col.load()
    last = max(1, h - 1)
    for y in range(h):
        px[0, y] = _lerp(top, bot, y / last)
    return col.resize((w, h), Image.Resampling.NEAREST)


class Pen:
    """Supersampled canvas. `sw` is the comic outline width in canvas pixels."""

    def __init__(self, size):
        self.size = size
        self.ss = 8 if size <= 48 else 4 if size <= 512 else 2
        self.c = size * self.ss
        # About 4.2% of the icon, but never thinner than ~1.2px on the final image.
        self.sw = max(self.ss * 1.2, self.c * 0.042)
        self.im = Image.new("RGBA", (self.c, self.c), (0, 0, 0, 0))
        self.d = ImageDraw.Draw(self.im)

    def x(self, frac):
        return frac * self.c

    def finish(self):
        if self.c == self.size:
            return self.im
        return self.im.resize((self.size, self.size), Image.Resampling.LANCZOS)

    def ok(self, box):
        x0, y0, x1, y1 = box
        return x1 - x0 > 2 and y1 - y0 > 2

    def inset(self, box, pad):
        x0, y0, x1, y1 = box
        return (x0 + pad, y0 + pad, x1 - pad, y1 - pad)

    def rrect(self, box, radius, fill):
        if not self.ok(box):
            return
        x0, y0, x1, y1 = box
        limit = min(x1 - x0, y1 - y0) / 2
        self.d.rounded_rectangle(box, radius=max(1, min(radius, limit)), fill=fill)

    def tile(self):
        c = self.c
        sw = self.sw
        radius = c * 0.228
        self.rrect((sw * 0.35, sw * 0.7, c - sw * 0.15, c - sw * 0.05), radius, (18, 78, 64, 56))
        plate = Image.new("RGBA", (c, c), (0, 0, 0, 0))
        ImageDraw.Draw(plate).rounded_rectangle((0, 0, c - 1, c - 1), radius=radius, fill=INK)
        grad = _vgrad(c, c, MINT_TOP, MINT)
        mask = Image.new("L", (c, c), 0)
        pad = sw * 0.92
        ImageDraw.Draw(mask).rounded_rectangle(
            (pad, pad, c - 1 - pad, c - 1 - pad),
            radius=max(1, radius - pad),
            fill=255,
        )
        grad.putalpha(mask)
        plate.alpha_composite(grad)
        # Soft sticker highlight along the top. Kept light so the tile stays matte.
        shine = Image.new("RGBA", (c, c), (0, 0, 0, 0))
        ImageDraw.Draw(shine).ellipse((-0.08 * c, -0.42 * c, 1.08 * c, 0.40 * c), fill=(255, 255, 255, 78))
        shine.putalpha(ImageChops.multiply(shine.getchannel("A"), mask))
        plate.alpha_composite(shine)
        self.im.alpha_composite(plate)
        self.d = ImageDraw.Draw(self.im)

    def page(self, box, radius, fill=PAGE):
        """Chubby outlined page. A short lip underneath separates the stack."""
        x0, y0, x1, y1 = box
        lip = self.sw * 0.36
        self.rrect((x0, y0 + lip, x1, y1 + lip), radius, INK_DEEP)
        self.rrect(box, radius, INK)
        inner = self.inset(box, self.sw * 0.90)
        self.rrect(inner, max(1, radius - self.sw), fill)
        return inner

    def eye(self, cx, cy, rx, ry):
        ring = min(self.sw * 0.34, rx * 0.28, ry * 0.28)
        self.d.ellipse((cx - rx, cy - ry, cx + rx, cy + ry), fill=INK)
        if rx - ring < 1.2 or ry - ring < 1.2:
            return
        self.d.ellipse((cx - rx + ring, cy - ry + ring, cx + rx - ring, cy + ry - ring), fill=WHITE)
        prx = (rx - ring) * 0.46
        pry = (ry - ring) * 0.50
        px = cx + rx * 0.02
        py = cy + ry * 0.08
        self.d.ellipse((px - prx, py - pry, px + prx, py + pry), fill=INK_DEEP)
        hr = max(1.0, prx * 0.40)
        hx = px - prx * 0.38
        hy = py - pry * 0.42
        self.d.ellipse((hx - hr, hy - hr, hx + hr, hy + hr), fill=WHITE)

    def face(self, box, blush=True):
        if not self.ok(box):
            return
        x0, y0, x1, y1 = box
        w, h = x1 - x0, y1 - y0
        cx = (x0 + x1) / 2
        # Tray sizes: two solid eyes. Highlights and blush average into a grey blob below ~32px.
        if self.size <= 24:
            dot = min(w, h) * 0.22
            if dot < 1:
                return
            ey = y0 + h * 0.52
            spread = min(w * 0.24, (w * 0.5) - dot * 1.15)
            for side in (-1, 1):
                ex = cx + side * spread
                self.d.ellipse((ex - dot, ey - dot * 1.05, ex + dot, ey + dot * 1.05), fill=INK)
            return
        spread = w * 0.20
        rx = min(w * 0.155, h * 0.30)
        ry = min(rx * 1.20, h * 0.34)
        ey = y0 + h * 0.40
        if ry < 1.5:
            return
        for side in (-1, 1):
            self.eye(cx + side * spread, ey, rx, ry)
        if blush and rx > self.sw * 0.4:
            brx = rx * 0.50
            bry = ry * 0.30
            by = ey + ry * 0.78
            for side in (-1, 1):
                bx = cx + side * (spread + rx * 0.85)
                self.d.ellipse((bx - brx, by - bry, bx + brx, by + bry), fill=BLUSH)
        mw = spread * 0.92
        mh = ry * 0.62
        my = ey + ry * 0.95
        if my + mh < y1 - h * 0.06 and mw > 2:
            self.smile(cx, my, mw, mh, max(self.sw * 0.34, rx * 0.16))

    def smile(self, cx, cy, w, h, thick):
        pts = []
        steps = 18
        for i in range(steps + 1):
            t = i / steps
            u = 1 - t
            x = u * u * (cx - w) + 2 * u * t * cx + t * t * (cx + w)
            y = u * u * cy + 2 * u * t * (cy + h) + t * t * cy
            pts.append((x, y))
        width = max(1, int(round(thick)))
        self.d.line(pts, fill=INK, width=width, joint="curve")
        r = thick / 2
        for x, y in (pts[0], pts[-1]):
            self.d.ellipse((x - r, y - r, x + r, y + r), fill=INK)

    def star_pts(self, cx, cy, r, inner, n):
        pts = []
        for i in range(n * 2):
            ang = -math.pi / 2 + i * math.pi / n
            rad = r if i % 2 == 0 else r * inner
            pts.append((cx + rad * math.cos(ang), cy + rad * math.sin(ang)))
        return pts

    def star(self, cx, cy, r, points=5, inner=0.46):
        """Chubby star. Tiny sizes collapse to a yellow dot so the accent survives."""
        m = r + self.sw * 0.2
        cx = min(max(cx, m), self.c - m)
        cy = min(max(cy, m), self.c - m)
        if r < self.sw * 1.15:
            self.d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=STAR)
            return
        self.d.polygon(self.star_pts(cx, cy, r, inner, points), fill=INK)
        inner_r = r - self.sw * 0.40
        if inner_r > r * 0.45:
            self.d.polygon(self.star_pts(cx, cy, inner_r, inner, points), fill=STAR)
            hr = inner_r * 0.16
            self.d.ellipse((cx - hr * 0.2, cy - inner_r * 0.34, cx + hr * 1.3, cy - inner_r * 0.34 + hr * 1.6), fill=WHITE)

    def windows(self, box, columns):
        """Inset comic panels. Omitted below 64px, where the page silhouette is enough."""
        if self.size < 64 or not self.ok(box):
            return
        x0, y0, x1, y1 = box
        w, h = x1 - x0, y1 - y0
        pad = max(self.sw * 0.48, min(w, h) * 0.12)
        area = (x0 + pad, y0 + pad, x1 - pad, y1 - pad)
        if not self.ok(area):
            return
        thick = max(self.ss * 0.9, self.sw * 0.30)
        gap = thick * 1.5
        aw = area[2] - area[0]
        col_w = (aw - gap * (columns - 1)) / columns
        if col_w < thick * 2.2:
            columns = 1
            col_w = aw
            gap = 0
        slots = []
        x = area[0]
        for _ in range(columns):
            slots.append((x, area[1], x + col_w, area[3]))
            x += col_w + gap
        rad = min(self.sw * 0.85, h * 0.22)
        for slot in slots:
            self.rrect(slot, rad, INK)
            hole = self.inset(slot, thick)
            self.rrect(hole, max(1, rad - thick), PAGE)


def _stack_boxes(p):
    """Three pages, narrower toward the top, centered. Outer boxes include the outline."""
    c = p.c
    sw = p.sw
    gap = max(sw * 0.55, c * 0.016)
    specs = (
        (0.50, 0.360),  # top, the face
        (0.64, 0.198),  # middle page
        (0.78, 0.186),  # bottom page
    )
    total = sum(h for _, h in specs) * c + gap * 2
    y = (c - total) / 2 + c * 0.012
    boxes = []
    for wfrac, hfrac in specs:
        w, h = wfrac * c, hfrac * c
        x0 = (c - w) / 2
        boxes.append((x0, y, x0 + w, y + h))
        y += h + gap
    return boxes


def draw_c(p):
    """Variant C — stacked comic pages, face on the top page, stars around."""
    p.tile()
    top, mid, bot = _stack_boxes(p)
    for box, columns in ((bot, 1), (mid, 2)):
        radius = p.sw * 1.25
        inner = p.page(box, radius)
        p.windows(inner, columns)
    top_r = min(top[2] - top[0], top[3] - top[1]) * 0.30
    inner = p.page(top, top_r)
    p.face(inner, blush=p.size >= 32)
    # Four stars, each large enough that the outline does not eat the points.
    c = p.c
    p.star(c * 0.155, top[1] + c * 0.012, c * 0.070)
    p.star(c * 0.855, top[1] + c * 0.028, c * 0.058)
    p.star(c * 0.108, (mid[1] + mid[3]) / 2, c * 0.060)
    p.star(c * 0.888, bot[1] + (bot[3] - bot[1]) * 0.22, c * 0.056)


def _ribbon(p, box):
    """Bookmark on the top-right of the page. A folded corner disappears on this radius."""
    x0, y0, x1, y1 = box
    w, h = x1 - x0, y1 - y0
    rw = max(p.sw * 1.15, w * 0.11)
    rh = h * 0.34
    rx0 = x1 - rw * 0.72
    ry0 = y0 + h * 0.06
    outer = (rx0, ry0, rx0 + rw, ry0 + rh)
    p.rrect(outer, rw * 0.28, INK)
    inner = p.inset(outer, p.sw * 0.32)
    p.rrect(inner, rw * 0.18, STAR)
    # Notch, so the bookmark reads as a ribbon rather than a sticker.
    tip_y = ry0 + rh
    mid = rx0 + rw / 2
    notch = rh * 0.22
    p.d.polygon(
        [(rx0 + p.sw * 0.2, tip_y - p.sw * 0.2), (mid, tip_y - notch), (rx0 + rw - p.sw * 0.2, tip_y - p.sw * 0.2)],
        fill=PAGE,
    )


def draw_a(p):
    """Variant A — one chubby page with ears, a soft face, and a folded corner."""
    p.tile()
    c = p.c
    sw = p.sw
    box = (c * 0.17, c * 0.22, c * 0.83, c * 0.84)
    x0, y0, x1, y1 = box
    w = x1 - x0
    ew, eh = w * 0.20, w * 0.24
    for side in (0.24, 0.76):
        ecx = x0 + w * side
        ear = (ecx - ew / 2, y0 - eh * 0.62, ecx + ew / 2, y0 + eh * 0.28)
        p.d.ellipse(ear, fill=INK)
        pad = sw * 0.55
        p.d.ellipse((ear[0] + pad, ear[1] + pad, ear[2] - pad, ear[3] - pad), fill=PAGE)
        inner_pad = pad + sw * 0.35
        p.d.ellipse(
            (ear[0] + inner_pad, ear[1] + eh * 0.28, ear[2] - inner_pad, ear[3] - pad * 0.4),
            fill=BLUSH,
        )
    radius = min(x1 - x0, y1 - y0) * 0.28
    inner = p.page(box, radius)
    if p.size >= 48 and p.ok(inner):
        _ribbon(p, inner)
    p.face(inner, blush=True)


def draw_b(p):
    """Variant B — a vertical scroll. Rolls at the ends, face on the paper."""
    p.tile()
    c = p.c
    sw = p.sw
    body = (c * 0.30, c * 0.24, c * 0.70, c * 0.78)
    roll_w = c * 0.50
    roll_h = c * 0.145
    cx = c / 2

    def roll(cy):
        box = (cx - roll_w / 2, cy - roll_h / 2, cx + roll_w / 2, cy + roll_h / 2)
        r = roll_h / 2
        p.rrect((box[0], box[1] + sw * 0.28, box[2], box[3] + sw * 0.28), r, INK_DEEP)
        p.rrect(box, r, INK)
        inner = p.inset(box, sw * 0.52)
        p.rrect(inner, r, (232, 247, 238, 255))
        # Cylinder highlight and a spiral cap on the left.
        if p.ok(inner):
            p.rrect(
                (inner[0] + sw * 0.3, inner[1] + sw * 0.15, inner[2] - sw * 0.3, inner[1] + (inner[3] - inner[1]) * 0.42),
                r,
                (255, 255, 255, 170),
            )
            cap = roll_h * 0.22
            cap_box = (inner[0] + sw * 0.25, cy - cap, inner[0] + sw * 0.25 + cap * 2, cy + cap)
            p.d.ellipse(cap_box, fill=INK)
            p.d.ellipse(p.inset(cap_box, sw * 0.28), fill=PAGE)

    roll(body[3] - roll_h * 0.08)
    inner = p.page(body, sw * 1.3 + (body[2] - body[0]) * 0.08)
    roll(body[1] + roll_h * 0.08)
    if p.ok(inner):
        face_box = (inner[0], inner[1] + roll_h * 0.42, inner[2], inner[3] - roll_h * 0.42)
        p.face(face_box, blush=True)


DRAW = {"a": draw_a, "b": draw_b, "c": draw_c}


def draw_icon(size, variant=DEFAULT_VARIANT):
    p = Pen(size)
    DRAW[variant](p)
    return p.finish()


def _png_bytes(im):
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


def write_icons(variant):
    os.makedirs(OUT, exist_ok=True)
    needed = {16, 24, 32, 48, 64, 128, 256, 512, 1024}
    images = {s: draw_icon(s, variant) for s in sorted(needed)}
    files = {
        "icon.png": images[512],
        "128x128.png": images[128],
        "128x128@2x.png": images[256],
        "32x32.png": images[32],
    }
    for name, im in files.items():
        dest = os.path.join(OUT, name)
        im.save(dest, format="PNG", optimize=True)
        print(f"wrote {name} ({im.size[0]}x{im.size[1]}) variant {variant}")
    write_ico(os.path.join(OUT, "icon.ico"), images)
    print("wrote icon.ico " + ",".join(str(s) for s in ICO_SIZES))
    write_icns(os.path.join(OUT, "icon.icns"), images)
    print("wrote icon.icns")


def _font(size):
    try:
        return ImageFont.truetype(FONT, size)
    except OSError:
        return ImageFont.load_default()


def write_previews(directory, variant):
    os.makedirs(directory, exist_ok=True)
    for key in ("a", "b", "c"):
        im = draw_icon(512, key)
        path = os.path.join(directory, f"icon-cartoon-{key}.png")
        im.save(path, format="PNG", optimize=True)
        print(f"wrote {path}")
    _write_ladder(directory, variant)


def _write_ladder(directory, variant):
    sizes = [16, 24, 32, 48, 64, 128, 256, 512]
    icons = [draw_icon(s, variant) for s in sizes]
    pad = 28
    label_h = 32
    zoom_sizes = [16, 24, 32, 48]
    zoom_target = 128
    zoomed = []
    for s in zoom_sizes:
        src = icons[sizes.index(s)]
        scale = max(1, round(zoom_target / s))
        zoomed.append(src.resize((s * scale, s * scale), Image.Resampling.NEAREST))

    row1_w = sum(im.width for im in icons) + pad * (len(icons) + 1)
    row2_w = sum(im.width for im in zoomed) + pad * (len(zoomed) + 1)
    width = max(row1_w, row2_w)
    height = pad + 512 + label_h + pad + zoom_target + label_h + pad
    sheet = Image.new("RGBA", (width, height), (243, 248, 245, 255))
    draw = ImageDraw.Draw(sheet)
    font = _font(18)
    ink = INK[:3]

    x = pad
    y = pad
    for s, im in zip(sizes, icons):
        sheet.alpha_composite(im, (x, y + (512 - im.height)))
        label = str(s)
        bbox = draw.textbbox((0, 0), label, font=font)
        tw = bbox[2] - bbox[0]
        draw.text((x + (im.width - tw) / 2, y + 512 + 6), label, fill=ink, font=font)
        x += im.width + pad

    x = pad
    y = pad + 512 + label_h + pad
    caption = _font(16)
    for s, im in zip(zoom_sizes, zoomed):
        sheet.alpha_composite(im, (x, y))
        label = f"{s}px ×{im.width // s}"
        bbox = draw.textbbox((0, 0), label, font=caption)
        tw = bbox[2] - bbox[0]
        draw.text((x + max(0, (im.width - tw) / 2), y + im.height + 6), label, fill=ink, font=caption)
        x += im.width + pad

    path = os.path.join(directory, "icon-size-ladder.png")
    sheet.convert("RGB").save(path, format="PNG", optimize=True)
    print(f"wrote {path}")


def main():
    parser = argparse.ArgumentParser(description="Draw MH Reader app icons. Default variant is C.")
    parser.add_argument("--variant", choices=tuple(DRAW), default=DEFAULT_VARIANT, help="a chubby page, b scroll, c stacked pages (default)")
    parser.add_argument("--preview-dir", default="", help="also write 512px A/B/C previews and a size ladder of the chosen variant")
    args = parser.parse_args()
    write_icons(args.variant)
    if args.preview_dir:
        write_previews(args.preview_dir, args.variant)


if __name__ == "__main__":
    main()
