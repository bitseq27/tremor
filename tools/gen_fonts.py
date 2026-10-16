#!/usr/bin/env python3
"""Generate the TREMOR multi-size bitmap font faces.

The old VT323 pixel face fell apart at the orrery UI sizes (broken strokes at
10/13 px, ragged at 16 px), so the HUD now rasterises **Source Code Pro**
(SIL Open Font License, bundled in `assets/fonts/`) - a hinted monospace that
stays fully legible at small sizes - with **8-bit anti-aliased coverage**
cells instead of 1 bpp rows. The GPU font atlas is R8, so the coverage bytes
render as smooth glyph edges with no shader change.

  assets/font_scp_xs_WxH.bin   - 10 px face (plot ticks / chips)
  assets/font_scp_sm_WxH.bin   - 13 px face (labels / readouts / keys)
  assets/font_scp_md_WxH.bin   - 16 px face (banners / body)

Cell pitch = ceil(font advance) + 1 px tracking; cell height and baseline row
are measured from the ink extents of the whole charset (so descenders of
`µ` and the ring of `°` are never clipped). Glyphs share a COMMON baseline
(anchor 'ls') and are centred in their cell.

Each file holds 103 glyphs, row-major, one byte per pixel (coverage 0..255):
ASCII 32..127 first (96 glyphs), then the seven extra symbols the HUD needs,
in this order:
  U+2013 –   U+00B7 ·   U+00D7 ×   U+00B0 °   U+00B1 ±   U+00B5 µ   U+0394 Δ

Usage:  python3 tools/gen_fonts.py
"""
import math
import os

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
TTF = os.path.join(ASSETS, "fonts", "SourceCodePro-Regular.ttf")

# faces: (tag, px_size)
FACES = [("xs", 10), ("sm", 13), ("md", 16)]

EXTRA_GLYPHS = ["–", "·", "×", "°", "±", "µ", "Δ"]


def charset():
    return [chr(c) for c in range(32, 128)] + EXTRA_GLYPHS


def load_font(px):
    f = ImageFont.truetype(TTF, px)
    try:
        names = [n.decode() if isinstance(n, bytes) else n for n in f.get_variation_names()]
        if "Regular" in names:
            f.set_variation_by_name("Regular")
    except Exception:
        pass
    return f


def measure(font, px):
    """(cell_w, cell_h, baseline_row) from advance + charset ink extents."""
    adv = font.getlength("M")
    cw = int(math.ceil(adv)) + 1
    probe = Image.new("L", (96, 96), 0)
    d = ImageDraw.Draw(probe)
    top, bot = 96, 0
    for g in charset():
        d.rectangle((0, 0, 96, 96), fill=0)
        d.text((48, 64), g, fill=255, font=font, anchor="ls")
        bb = probe.getbbox()
        if bb:
            top = min(top, bb[1])
            bot = max(bot, bb[3])
    ch = bot - top
    baseline = 64 - top
    return cw, ch, baseline, adv


def make_face(tag, px):
    font = load_font(px)
    cw, ch, baseline, adv = measure(font, px)
    pen_x = int((cw - adv) / 2.0 + 0.5)
    glyphs = []
    for g in charset():
        img = Image.new("L", (cw, ch), 0)
        d = ImageDraw.Draw(img)
        d.text((pen_x, baseline), g, fill=255, font=font, anchor="ls")
        # boost thin stroke coverage so 1px arms stay crisp at 10/13 px
        img = img.point(lambda v: min(255, int(v * 1.5)))
        glyphs.append(img.tobytes())
    data = b"".join(glyphs)
    out = os.path.join(ASSETS, f"font_scp_{tag}_{cw}x{ch}.bin")
    with open(out, "wb") as f:
        f.write(data)

    # human-checkable preview (4x, phosphor cyan on black)
    cols = 16
    rows = (len(glyphs) + cols - 1) // cols
    prev = Image.new("RGB", (cols * cw * 4, rows * ch * 4), (0, 0, 0))
    for gi, gb in enumerate(glyphs):
        gx = (gi % cols) * cw
        gy = (gi // cols) * ch
        for y in range(ch):
            for x in range(cw):
                v = gb[y * cw + x]
                if v:
                    c = (v * 33 // 255, v * 208 // 255, v * 197 // 255)
                    for dy in range(4):
                        for dx in range(4):
                            prev.putpixel(((gx + x) * 4 + dx, (gy + y) * 4 + dy), c)
    prev.save(os.path.join(ASSETS, f"preview_font_scp_{tag}.png"))
    print(
        f"wrote {out} ({len(data)} bytes, {len(glyphs)} glyphs {cw}x{ch} "
        f"baseline {baseline} advance {adv:.2f})"
    )
    return tag, cw, ch


def main():
    if not os.path.exists(TTF):
        raise SystemExit(f"missing {TTF} (copy SourceCodePro-Regular.ttf into assets/fonts/)")
    metrics = [make_face(tag, px) for tag, px in FACES]
    print("\n// paste into src/assets.rs:")
    yoff = 0
    for tag, cw, ch in metrics:
        print(f"// FACE_{tag.upper()}: cw {cw}, ch {ch}, yoff {yoff}  (7 glyph rows/face)")
        yoff += 7 * ch
    print(f"// ATLAS_W = 16 * {max(m[1] for m in metrics)}, ATLAS_H = {yoff}")


if __name__ == "__main__":
    main()
