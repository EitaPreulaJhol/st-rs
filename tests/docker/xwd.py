#!/usr/bin/env python3
"""Compare XWD screenshots pixel by pixel.

Usage:
    xwd.py compare A.xwd B.xwd
    xwd.py different A.xwd B.xwd MIN_PIXELS
    xwd.py pixel A.xwd X Y

`compare` exits 0 when the images are pixel-identical.
`different` exits 0 when at least MIN_PIXELS differ.
`pixel` prints "r g b a" (0-255) for one pixel.
"""

import struct
import sys


def load(path):
    with open(path, "rb") as f:
        data = f.read()
    if len(data) < 100:
        raise SystemExit(f"{path}: not an XWD file")
    hdr = struct.unpack(">25I", data[:100])
    (hsize, _ver, fmt, _depth, w, h, _xoff, order, _bu, _bo, _pad,
     bpp, bpl, _vc, rmask, gmask, bmask, _brgb, _ce, ncolors,
     _ww, _wh, _wx, _wy, _bdr) = hdr
    if fmt != 2:
        raise SystemExit(f"{path}: unsupported XWD format {fmt}")
    off = hsize + ncolors * 12
    px = data[off:]
    need = bpl * h
    if len(px) < need:
        raise SystemExit(f"{path}: truncated pixel data")
    return {
        "w": w, "h": h, "bpp": bpp, "bpl": bpl, "order": order,
        "rmask": rmask, "gmask": gmask, "bmask": bmask, "px": px,
    }


def scale(mask, value):
    if mask == 0:
        return 0
    shift = (mask & -mask).bit_length() - 1
    maxv = mask >> shift
    return (((value & mask) >> shift) * 255 + maxv // 2) // maxv


def pixel(img, row, x):
    bpp = img["bpp"]
    if bpp == 32:
        return struct.unpack_from("<I" if img["order"] == 0 else ">I",
                                  row, x * 4)[0]
    if bpp == 24:
        b = row[x * 3:x * 3 + 3]
        if img["order"] == 0:
            return b[0] | b[1] << 8 | b[2] << 16
        return b[0] << 16 | b[1] << 8 | b[2]
    if bpp == 16:
        return struct.unpack_from("<H" if img["order"] == 0 else ">H",
                                  row, x * 2)[0]
    raise SystemExit(f"unsupported bits-per-pixel {bpp}")


def compare(a, b):
    if (a["w"], a["h"]) != (b["w"], b["h"]):
        return -1, None
    w, h = a["w"], a["h"]
    minx = miny = 1 << 30
    maxx = maxy = -1
    count = 0
    for y in range(h):
        ra = a["px"][y * a["bpl"]:y * a["bpl"] + w * (a["bpp"] // 8)]
        rb = b["px"][y * b["bpl"]:y * b["bpl"] + w * (b["bpp"] // 8)]
        for x in range(w):
            va = pixel(a, ra, x)
            vb = pixel(b, rb, x)
            ca = (scale(a["rmask"], va), scale(a["gmask"], va),
                  scale(a["bmask"], va))
            cb = (scale(b["rmask"], vb), scale(b["gmask"], vb),
                  scale(b["bmask"], vb))
            if ca != cb:
                count += 1
                minx = min(minx, x)
                maxx = max(maxx, x)
                miny = min(miny, y)
                maxy = max(maxy, y)
    return count, (minx, miny, maxx, maxy)


def main(argv):
    if len(argv) == 4 and argv[1] == "compare":
        a, b = load(argv[2]), load(argv[3])
        n, bbox = compare(a, b)
        if n == 0:
            print(f"pixel-identical: {a['w']}x{a['h']}")
            return 0
        if n < 0:
            print(f"FAIL: dimensions differ: {a['w']}x{a['h']} vs "
                  f"{b['w']}x{b['h']}")
        else:
            print(f"FAIL: {n} pixels differ, bbox {bbox}")
        return 1

    if len(argv) == 5 and argv[1] == "different":
        minimum = int(argv[4])
        a, b = load(argv[2]), load(argv[3])
        n, bbox = compare(a, b)
        if n < 0:
            print("different: dimensions differ")
            return 0
        print(f"{n} pixels differ (bbox {bbox})")
        return 0 if n >= minimum else 1

    if len(argv) == 5 and argv[1] == "pixel":
        img = load(argv[2])
        x, y = int(argv[3]), int(argv[4])
        if not (0 <= x < img["w"] and 0 <= y < img["h"]):
            print(f"{argv[2]}: pixel out of range", file=sys.stderr)
            return 2
        row = img["px"][y * img["bpl"]:y * img["bpl"] +
                       img["w"] * (img["bpp"] // 8)]
        p = pixel(img, row, x)
        amask = ~(img["rmask"] | img["gmask"] | img["bmask"]) & 0xffffffff
        alpha = scale(amask, p) if amask else 255
        print(scale(img["rmask"], p), scale(img["gmask"], p),
              scale(img["bmask"], p), alpha)
        return 0

    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
