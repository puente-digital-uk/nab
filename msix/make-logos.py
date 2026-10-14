#!/usr/bin/env python3
"""Regenerate msix/Assets/*.png from assets/icon.png. Needs Pillow: python3 -m pip install pillow"""
from pathlib import Path
from PIL import Image

HERE = Path(__file__).resolve().parent
icon = Image.open(HERE.parent / "assets" / "icon.png").convert("RGBA")
out = HERE / "Assets"
out.mkdir(exist_ok=True)

def tile(name, w, h, icon_px):
    """A w×h transparent tile with the icon icon_px wide, centred."""
    im = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ic = icon.resize((icon_px, icon_px), Image.LANCZOS)
    im.paste(ic, ((w - icon_px) // 2, (h - icon_px) // 2), ic)
    im.save(out / name, optimize=True)

for scale in (100, 200):
    s = scale / 100
    tile(f"Square44x44Logo.scale-{scale}.png", int(44 * s), int(44 * s), int(44 * s))
    tile(f"Square150x150Logo.scale-{scale}.png", int(150 * s), int(150 * s), int(100 * s))
    tile(f"Wide310x150Logo.scale-{scale}.png", int(310 * s), int(150 * s), int(100 * s))
    tile(f"StoreLogo.scale-{scale}.png", int(50 * s), int(50 * s), int(50 * s))
# Unqualified copies (= scale-100), so the manifest's plain names resolve even without a resource index.
for base in ("Square44x44Logo", "Square150x150Logo", "Wide310x150Logo", "StoreLogo"):
    Image.open(out / f"{base}.scale-100.png").save(out / f"{base}.png", optimize=True)
for px in (16, 24, 32, 48, 256):  # taskbar / Start / file-type sizes, drawn without a plate behind them
    tile(f"Square44x44Logo.targetsize-{px}_altform-unplated.png", px, px, px)
print("wrote", len(list(out.glob("*.png"))), "logos to", out)
