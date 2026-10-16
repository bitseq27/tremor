#!/usr/bin/env python3
"""Pack TREMOR shipping zips.

  ship/tremor-windows-x86_64.zip   self-contained tremor.exe + README.md
  ../tremor-src-<date>.zip         full source tree (no .git/target/ppm/exe)

Usage:  python3 tools/pack_ship.py [--exe PATH]
"""
import os
import sys
import time
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SHIP = os.path.join(ROOT, "ship")
DEFAULT_EXE = os.path.join(
    ROOT, "target", "x86_64-pc-windows-gnu", "release", "tremor.exe"
)


def pack_ship(exe):
    if not os.path.exists(exe):
        raise SystemExit(f"missing {exe} - cross-build first:\n"
                         "  cargo build --release --target x86_64-pc-windows-gnu")
    out = os.path.join(SHIP, "tremor-windows-x86_64.zip")
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.write(exe, "tremor.exe")
        z.write(os.path.join(SHIP, "README.md"), "README.md")
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KiB)")


def pack_src():
    out = os.path.join(
        ROOT, "..", f"tremor-src-{time.strftime('%Y-%m-%d')}.zip"
    )
    out = os.path.normpath(out)
    skip_dirs = {".git", "target", "node_modules"}
    skip_files = {"tremor.exe", "tremor-windows-x86_64.zip"}
    n = 0
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for dirpath, dirnames, filenames in os.walk(ROOT):
            dirnames[:] = [d for d in dirnames if d not in skip_dirs]
            for f in sorted(filenames):
                if f in skip_files or f.endswith(".ppm"):
                    continue
                full = os.path.join(dirpath, f)
                z.write(full, "tremor/" + os.path.relpath(full, ROOT))
                n += 1
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KiB, {n} files)")


def pack_unified():
    """One zip with the whole updated project, rebuilt exe included."""
    out = os.path.normpath(os.path.join(ROOT, "..", f"tremor-{time.strftime('%Y-%m-%d')}.zip"))
    skip_dirs = {".git", "target", "node_modules"}
    skip_files = {"tremor-windows-x86_64.zip"}  # nested ship zip stays out
    n = 0
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for dirpath, dirnames, filenames in os.walk(ROOT):
            dirnames[:] = [d for d in dirnames if d not in skip_dirs]
            for f in sorted(filenames):
                if f in skip_files or f.endswith(".ppm"):
                    continue
                full = os.path.join(dirpath, f)
                z.write(full, "tremor/" + os.path.relpath(full, ROOT))
                n += 1
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KiB, {n} files)")


if __name__ == "__main__":
    exe = DEFAULT_EXE
    if "--exe" in sys.argv:
        exe = sys.argv[sys.argv.index("--exe") + 1]
    if "--unified" in sys.argv:
        pack_unified()
    else:
        pack_ship(exe)
        pack_src()
