#!/usr/bin/env python3
"""生成 mh-reader 的应用图标（PIL 绘制，无需外部素材）。

设计：深色圆角底 + 白色漫画页 + 顶部蓝色条 + 蓝色下箭头。
输出到 src-tauri/icons/：icon.png(512) / 128x128.png / 32x32.png / icon.ico。
克隆仓库后首次构建前运行一次：python scripts/gen-icons.py
"""
from PIL import Image, ImageDraw
import os

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src-tauri", "icons")

BG = (28, 28, 30, 255)      # 深色底
PAGE = (255, 255, 255, 255)  # 漫画页
BLUE = (30, 136, 235, 255)   # 蓝条 / 箭头


def draw_icon(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = size / 512.0  # 以 512 为基准的缩放系数

    # 深色圆角底
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=int(115 * u), fill=BG)

    # 白色漫画页
    px0, py0 = int(155 * u), int(115 * u)
    px1, py1 = int(357 * u), int(400 * u)
    pr = int(28 * u)
    d.rounded_rectangle([px0, py0, px1, py1], radius=pr, fill=PAGE)

    # 顶部蓝色条（上圆角与页对齐，下方直角）
    band_h = int(95 * u)
    d.rounded_rectangle([px0, py0, px1, py0 + band_h], radius=pr, fill=BLUE)
    d.rectangle([px0, py0 + band_h - pr, px1, py0 + band_h], fill=BLUE)

    # 蓝色下箭头：竖条 + 三角头
    cx = size // 2
    bar_w, bar_top, bar_bot = int(48 * u), int(238 * u), int(335 * u)
    d.rectangle([cx - bar_w // 2, bar_top, cx + bar_w // 2, bar_bot], fill=BLUE)
    hw, hy0, hy1 = int(58 * u), int(318 * u), int(392 * u)
    d.polygon([(cx - hw, hy0), (cx + hw, hy0), (cx, hy1)], fill=BLUE)

    return img


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    base = draw_icon(512)
    base.save(os.path.join(OUT, "icon.png"))
    base.resize((128, 128), Image.LANCZOS).save(os.path.join(OUT, "128x128.png"))
    base.resize((32, 32), Image.LANCZOS).save(os.path.join(OUT, "32x32.png"))
    # ico：多尺寸打包
    base.save(
        os.path.join(OUT, "icon.ico"),
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("icons written to", OUT)


if __name__ == "__main__":
    main()
