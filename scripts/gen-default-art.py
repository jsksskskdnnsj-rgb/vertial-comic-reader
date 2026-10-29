#!/usr/bin/env python3
"""Soft-light stand-in wallpapers for src/assets/.

The stylesheet points at bg-default.jpg and banner-default.jpg. Those files
were not in the repository, so the default background and home banner 404'd.
Run from the repo root:  python scripts/gen-default-art.py
Requires Pillow. Does not touch a user-chosen wallpaper or banner.
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src" / "assets"


def wash(w, h, blobs):
    img = Image.new("RGB", (w, h), (233, 242, 242))
    layer = Image.new("RGB", (w, h), (233, 242, 242))
    draw = ImageDraw.Draw(layer)
    for cx, cy, r, color in blobs:
        x, y = int(cx * w), int(cy * h)
        draw.ellipse((x - r, y - r, x + r, y + r), fill=color)
    layer = layer.filter(ImageFilter.GaussianBlur(radius=min(w, h) // 8))
    return Image.blend(img, layer, 0.72)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    bg = wash(1600, 1000, [
        (0.18, 0.12, 520, (255, 255, 252)),
        (0.82, 0.78, 640, (168, 214, 204)),
        (0.55, 0.42, 380, (214, 236, 230)),
        (0.08, 0.88, 300, (186, 214, 216)),
    ])
    banner = wash(1600, 520, [
        (0.22, 0.35, 420, (255, 255, 252)),
        (0.78, 0.4, 480, (150, 206, 190)),
        (0.5, 0.15, 260, (220, 242, 234)),
    ])
    bg.save(OUT / "bg-default.jpg", quality=82, optimize=True)
    banner.save(OUT / "banner-default.jpg", quality=82, optimize=True)
    for name in ("bg-default.jpg", "banner-default.jpg"):
        p = OUT / name
        print(f"{p.relative_to(ROOT)}  {p.stat().st_size} bytes")


if __name__ == "__main__":
    main()
