#!/usr/bin/env python3
"""生成 mh-reader 的应用图标（PIL 绘制，无需外部素材）。

设计：深色圆角底 + 白色漫画页 + 顶部蓝色条 + 蓝色下箭头。
输出到 src-tauri/icons/：icon.png(512) / 128x128.png / 32x32.png / icon.ico。
克隆仓库后首次构建前运行一次：python scripts/gen-icons.py

重要：本脚本只补缺失文件，绝不覆盖已存在的文件。仓库现有的
src-tauri/icons/ 是用户自供图（2026-10-26 起），重跑本脚本也不会
把它盖掉；只有当某个图标文件缺失时（例如新克隆后 icons/ 为空），
才按上面的 PIL 设计生成那一个缺失文件。
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
    targets = ["icon.png", "128x128.png", "32x32.png", "icon.ico"]
    missing = [f for f in targets if not os.path.exists(os.path.join(OUT, f))]
    if not missing:
        print("icons already present, nothing to do (existing files are never overwritten)")
        return
    # 只为缺失文件生成内容：已有 icon.png 时直接用它派生缺失尺寸，
    # 否则按 PIL 设计重绘 512 底图。
    if "icon.png" in missing:
        base = draw_icon(512)
        base.save(os.path.join(OUT, "icon.png"))
        print("wrote icon.png")
    else:
        base = Image.open(os.path.join(OUT, "icon.png")).convert("RGBA")
        print("skip icon.png (exists)")
    if "128x128.png" in missing:
        base.resize((128, 128), Image.LANCZOS).save(os.path.join(OUT, "128x128.png"))
        print("wrote 128x128.png")
    else:
        print("skip 128x128.png (exists)")
    if "32x32.png" in missing:
        base.resize((32, 32), Image.LANCZOS).save(os.path.join(OUT, "32x32.png"))
        print("wrote 32x32.png")
    else:
        print("skip 32x32.png (exists)")
    if "icon.ico" in missing:
        base.save(
            os.path.join(OUT, "icon.ico"),
            sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
        )
        print("wrote icon.ico")
    else:
        print("skip icon.ico (exists)")


if __name__ == "__main__":
    main()
