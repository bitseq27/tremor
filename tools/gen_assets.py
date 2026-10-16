#!/usr/bin/env python3
"""Generate embedded binary assets for the TREMOR wgpu app.

Outputs (into assets/):
  font_<tag>_<W>x<H>.bin   - 1bpp bitmap font, 96 glyphs (ASCII 32..127), row-major,
                             each glyph H rows of ceil(W/8) bytes.
  landmask_<W>x<H>.bin     - 1bpp equirectangular land mask, row-major packed bits.
  preview_*.png            - human-checkable previews.

Sources:
  TTF fonts from the google/fonts repository (OFL licensed).
  Land polygons from Natural Earth 110m public-domain geojson.
"""
import json
import os
import sys
import urllib.request

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
os.makedirs(ASSETS, exist_ok=True)

FONTS = {
    "vt323": "https://raw.githubusercontent.com/google/fonts/main/ofl/vt323/VT323-Regular.ttf",
    "plexmono": "https://raw.githubusercontent.com/google/fonts/main/ofl/ibmplexmono/IBMPlexMono-Regular.ttf",
    "spacemono": "https://raw.githubusercontent.com/google/fonts/main/ofl/spacemono/SpaceMono-Regular.ttf",
}
LAND_URL = ("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/"
            "master/geojson/ne_110m_land.geojson")
LAND50_URL = ("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/"
              "master/geojson/ne_50m_land.geojson")


def fetch(url, dest):
    if not os.path.exists(dest):
        print("fetch", url)
        urllib.request.urlretrieve(url, dest)
    return dest


def make_font(tag, url, cw, ch, px_size):
    ttf = fetch(url, f"/tmp/{tag}.ttf")
    font = ImageFont.truetype(ttf, px_size)
    asc, desc = font.getmetrics()
    glyphs = []
    preview = Image.new("RGB", (cw * 16, ch * 6), (0, 0, 0))
    pd = ImageDraw.Draw(preview)
    for code in range(32, 128):
        ch_ = chr(code)
        img = Image.new("L", (cw * 4, ch * 4), 0)
        d = ImageDraw.Draw(img)
        d.text((cw, ch), ch_, fill=255, font=font)
        # downsample to cw x ch by centering on the ink bbox of a reference render
        ref = Image.new("L", (64, 64), 0)
        rd = ImageDraw.Draw(ref)
        rd.text((4, 4), ch_, fill=255, font=font)
        bbox = ref.getbbox()
        small = Image.new("L", (cw, ch), 0)
        if bbox:
            # render glyph alone tightly, then paste centered horizontally, baseline aligned
            tight = Image.new("L", (64, 64), 0)
            td = ImageDraw.Draw(tight)
            td.text((4 - bbox[0], 4 - bbox[1]), ch_, fill=255, font=font)
            tb = tight.getbbox()
            crop = tight.crop(tb)
            gw = min(cw, crop.width)
            gh = min(ch - 2, crop.height)
            crop = crop.resize((gw, gh), Image.LANCZOS) if (gw, gh) != crop.size else crop
            ox = (cw - gw) // 2
            oy = ch - 3 - gh  # 2px descender room, 1px top room
            oy = max(1, oy)
            small.paste(crop, (ox, oy))
        bits = bytearray()
        for y in range(ch):
            row = 0
            for x in range(cw):
                if small.getpixel((x, y)) >= 110:
                    row |= 1 << (7 - x)
            bits.append(row)
        glyphs.append(bytes(bits))
        # preview
        px, py = (code - 32) % 16, (code - 32) // 16
        for y in range(ch):
            for x in range(cw):
                if small.getpixel((x, y)) >= 110:
                    preview.putpixel((px * cw + x, py * ch + y), (64, 224, 208))
    data = b"".join(glyphs)
    out = os.path.join(ASSETS, f"font_{tag}_{cw}x{ch}.bin")
    with open(out, "wb") as f:
        f.write(data)
    preview.save(os.path.join(ASSETS, f"preview_font_{tag}.png"))
    print("wrote", out, len(data), "bytes")


def scanline_fill(polys, w, h):
    """polys: list of lists of rings; ring: list of (lon,lat). Returns bytearray of bits."""
    mask = bytearray((w * h + 7) // 8)
    # work in pixel space: x = (lon+180)/360*w, y = (90-lat)/180*h
    for rings in polys:
        for ring in rings:
            pts = [(((lon + 180.0) / 360.0) * w, ((90.0 - lat) / 180.0) * h)
                   for (lon, lat) in ring]
            n = len(pts)
            if n < 3:
                continue
            # gather x-intersections per scanline y+0.5
            edges = []
            ymin = max(0, int(min(p[1] for p in pts)))
            ymax = min(h - 1, int(max(p[1] for p in pts)))
            for i in range(n):
                x0, y0 = pts[i]
                x1, y1 = pts[(i + 1) % n]
                if y0 == y1:
                    continue
                edges.append((x0, y0, x1, y1))
            for y in range(ymin, ymax + 1):
                yc = y + 0.5
                xs = []
                for x0, y0, x1, y1 in edges:
                    if (y0 <= yc < y1) or (y1 <= yc < y0):
                        t = (yc - y0) / (y1 - y0)
                        xs.append(x0 + t * (x1 - x0))
                xs.sort()
                for k in range(0, len(xs) - 1, 2):
                    xa = max(0, int(round(xs[k])))
                    xb = min(w - 1, int(round(xs[k + 1])))
                    for x in range(xa, xb + 1):
                        mask[(y * w + x) // 8] |= 1 << (7 - ((y * w + x) % 8))
    return mask


def make_landmask(w, h):
    src = fetch(LAND_URL, "/tmp/ne_110m_land.geojson")
    with open(src) as f:
        gj = json.load(f)
    polys = []
    geom_list = gj["features"]
    for feat in geom_list:
        g = feat["geometry"]
        if g["type"] == "Polygon":
            polys.append(g["coordinates"])
        elif g["type"] == "MultiPolygon":
            polys.extend(g["coordinates"])
    mask = scanline_fill(polys, w, h)
    out = os.path.join(ASSETS, f"landmask_{w}x{h}.bin")
    with open(out, "wb") as f:
        f.write(bytes(mask))
    img = Image.new("RGB", (w, h), (4, 10, 18))
    for y in range(h):
        for x in range(w):
            if mask[(y * w + x) // 8] & (1 << (7 - ((y * w + x) % 8))):
                img.putpixel((x, y), (46, 201, 186))
    img.save(os.path.join(ASSETS, "preview_landmask.png"))
    print("wrote", out, len(mask), "bytes")


def make_landmask_url(w, h, url, tag):
    src = fetch(url, f"/tmp/land_{tag}.geojson")
    with open(src) as f:
        gj = json.load(f)
    polys = []
    for feat in gj["features"]:
        g = feat["geometry"]
        if g["type"] == "Polygon":
            polys.append(g["coordinates"])
        elif g["type"] == "MultiPolygon":
            polys.extend(g["coordinates"])
    mask = scanline_fill(polys, w, h)
    out = os.path.join(ASSETS, f"landmask_{w}x{h}.bin")
    with open(out, "wb") as f:
        f.write(bytes(mask))
    print("wrote", out, len(mask), "bytes")



# ---- city lights + capital cities (GeoNames, CC-BY 4.0) -------------------
CITIES_URL = "https://download.geonames.org/export/dump/cities15000.zip"

def make_citylights(w=2048, h=1024):
    """Bake an 8-bit equirectangular night-lights map from GeoNames cities
    (population >= 100k -> light dot, intensity ~ log10 pop), and write a
    companion capital-cities asset (PPLC feature code: i16 lat/lon*100 +
    length-prefixed uppercase name)."""
    import io, zipfile
    zdest = fetch(CITIES_URL, "/tmp/cities15000.zip")
    with zipfile.ZipFile(zdest) as z:
        txt = z.read("cities15000.txt").decode("utf-8", "replace")
    lights = bytearray(w * h)
    caps = []
    nlights = 0
    for line in txt.splitlines():
        f = line.split("\t")
        if len(f) < 15:
            continue
        try:
            lat, lon, pop = float(f[4]), float(f[5]), int(f[14])
        except ValueError:
            continue
        name, code = f[1], f[7]
        if code == "PPLC":
            caps.append((lat, lon, name.upper()[:24]))
        if pop < 100000:
            continue
        nlights += 1
        # stamp a small gaussian-ish dot; metro areas get a wider blob
        import math
        inten = min(1.0, 0.35 + 0.22 * math.log10(pop / 100000.0))
        px = (lon + 180.0) / 360.0 * w
        py = (90.0 - lat) / 180.0 * h
        rad = 1.0 + min(2.0, pop / 8_000_000.0 * 2.0)
        r = int(rad) + 1
        for dy in range(-r, r + 1):
            for dx in range(-r, r + 1):
                x = int(px + dx) % w
                y = int(py + dy)
                if y < 0 or y >= h:
                    continue
                d = math.hypot(dx, dy)
                if d > rad + 0.5:
                    continue
                v = inten * max(0.0, 1.0 - d / (rad + 0.5))
                lights[y * w + x] = max(lights[y * w + x], int(v * 255))
    out = os.path.join(ASSETS, f"citylights_{w}x{h}.bin")
    with open(out, "wb") as f:
        f.write(bytes(lights))
    img = Image.new("RGB", (w // 2, h // 2), (4, 10, 18))
    for y in range(h // 2):
        for x in range(w // 2):
            v = lights[(y * 2) * w + (x * 2)]
            if v:
                img.putpixel((x, y), (255, 190 + v // 6, 120 + v // 4))
    img.save(os.path.join(ASSETS, "preview_citylights.png"))
    print("wrote", out, len(lights), "bytes,", nlights, "cities,", len(caps), "capitals")
    # capitals asset: [count u32][ (lat i16*100, lon i16*100, namelen u8, name) ... ]
    import struct
    buf = io.BytesIO()
    buf.write(struct.pack("<I", len(caps)))
    for lat, lon, name in caps:
        nb = name.encode("ascii", "replace")
        buf.write(struct.pack("<hhB", int(lat * 100), int(lon * 100), len(nb)))
        buf.write(nb)
    cout = os.path.join(ASSETS, "capitals.bin")
    with open(cout, "wb") as f:
        f.write(buf.getvalue())
    print("wrote", cout, len(buf.getvalue()), "bytes")


if __name__ == "__main__":
    import sys
    # NOTE: the bitmap font faces moved to tools/gen_fonts.py (multi-size
    # VT323: 10/13/16 px, matching the orrery HMI UI sizes).
    if "--cities" in sys.argv:
        make_citylights()
    elif "--hires" in sys.argv:
        make_landmask_url(3600, 1800, LAND50_URL, "50m")
    else:
        make_landmask(720, 360)
