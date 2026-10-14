#!/usr/bin/env python3
"""Draw the Nab icon (rounded gradient square, corner brackets, a bold N) at 1024 px and write
assets/icon.png (256) and assets/icon.ico (16–256). Needs Pillow: python3 -m pip install pillow"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
S = 1024                       # working size; everything below is in 256-px units × 4
k = S / 256
TOP, BOTTOM = (36, 104, 229), (21, 178, 171)   # the gradient, sampled from the original icon


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
# Gradient, masked to a rounded square inset 5 px with ~22% corner radius.
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
for y in range(S):
    gd.line([(0, y), (S, y)], fill=lerp(TOP, BOTTOM, y / (S - 1)) + (255,))
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([5 * k, 5 * k, S - 5 * k, S - 5 * k], radius=56 * k, fill=255)
im.paste(grad, (0, 0), mask)

d = ImageDraw.Draw(im)
W = (255, 255, 255, 255)
t = 17 * k                     # stroke
# Corner brackets: arms from 50 to 101 (51 px) in each corner.
for (x0, y0, sx, sy) in [(50, 50, 1, 1), (206, 50, -1, 1), (50, 206, 1, -1), (206, 206, -1, -1)]:
    x0, y0, L = x0 * k, y0 * k, 51 * k
    # horizontal arm
    d.rectangle([min(x0, x0 + sx * L), min(y0, y0 + sy * t), max(x0, x0 + sx * L), max(y0, y0 + sy * t)], fill=W)
    # vertical arm
    d.rectangle([min(x0, x0 + sx * t), min(y0, y0 + sy * L), max(x0, x0 + sx * t), max(y0, y0 + sy * L)], fill=W)

# The N: two stems and a diagonal, centred where the T used to be.
bx0, by0, bx1, by1 = 94 * k, 86 * k, 162 * k, 170 * k
t = 15 * k                     # the letter's stroke, a touch lighter than the brackets so the counters stay open
d.rectangle([bx0, by0, bx0 + t, by1], fill=W)
d.rectangle([bx1 - t, by0, bx1, by1], fill=W)
# Diagonal: a parallelogram from the top-left corner to the bottom-right corner, as thick as a stem.
import math
hw = t / math.sin(math.atan2(by1 - by0, bx1 - bx0))   # horizontal width that gives perpendicular thickness t
d.polygon([(bx0, by0), (bx0 + hw, by0), (bx1, by1), (bx1 - hw, by1)], fill=W)

icon256 = im.resize((256, 256), Image.LANCZOS)
icon256.save(ROOT / "assets" / "icon.png", optimize=True)
icon256.save(ROOT / "assets" / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("wrote assets/icon.png and assets/icon.ico")
