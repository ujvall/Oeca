"""Traces a black-on-light icon image into the smooth closed outlines that
`traced.py` holds (one point list per outline, y down, in image pixels).

Usage: python trace.py image.png NAME [solid] >> traced.py   (needs numpy + Pillow)
  solid: keep only the solid outlines (no holes: a tick or slot is filled in)

Marching squares at the half-way level (sub-pixel), the outlines smoothed a
little and resampled evenly; `generate.py` turns them into cubic Beziers.
"""
import sys

import numpy as np
from PIL import Image


def field(path):
    im = np.asarray(Image.open(path).convert("RGBA"), dtype=float) / 255
    lum = im[..., :3].mean(axis=2)
    return im[..., 3] * (1 - lum)


def contours(v, level=0.5):
    """Closed contours of `v` at `level` (marching squares, chained)."""
    h, w = v.shape
    v = np.pad(v, 1)  # so every shape closes inside the grid
    segs = {}

    def cross(x0, y0, x1, y1):
        a, b = v[y0, x0], v[y1, x1]
        t = (level - a) / (b - a)
        return (x0 + (x1 - x0) * t - 1, y0 + (y1 - y0) * t - 1)

    for y in range(v.shape[0] - 1):
        for x in range(v.shape[1] - 1):
            c = [v[y, x], v[y, x + 1], v[y + 1, x + 1], v[y + 1, x]]
            idx = sum((c[i] >= level) << i for i in range(4))
            if idx in (0, 15):
                continue
            e = {
                "t": lambda: cross(x, y, x + 1, y),
                "r": lambda: cross(x + 1, y, x + 1, y + 1),
                "b": lambda: cross(x, y + 1, x + 1, y + 1),
                "l": lambda: cross(x, y, x, y + 1),
            }
            table = {
                1: [("l", "t")], 2: [("t", "r")], 3: [("l", "r")], 4: [("r", "b")],
                5: [("l", "t"), ("r", "b")], 6: [("t", "b")], 7: [("l", "b")],
                8: [("b", "l")], 9: [("b", "t")], 10: [("t", "r"), ("b", "l")],
                11: [("b", "r")], 12: [("r", "l")], 13: [("r", "t")], 14: [("t", "l")],
            }
            for a, b in table[idx]:
                segs[(x, y, a)] = (e[a](), (x, y, b))
    # Chain by shared cell edges: an edge "r" of a cell is the "l" of its right neighbour
    nxt = {"t": lambda x, y: (x, y - 1, "b"), "b": lambda x, y: (x, y + 1, "t"),
           "l": lambda x, y: (x - 1, y, "r"), "r": lambda x, y: (x + 1, y, "l")}
    seen, out = set(), []
    for key in list(segs):
        if key in seen:
            continue
        pts, k = [], key
        while k in segs and k not in seen:
            seen.add(k)
            p, (x, y, b) = segs[k]
            pts.append(p)
            k = nxt[b](x, y)
        if len(pts) > 8:
            out.append(pts)
    return out


def smooth(pts, window):
    p = np.array(pts)
    n = len(p)
    out = np.zeros_like(p)
    for i in range(n):
        idx = [(i + j) % n for j in range(-window, window + 1)]
        out[i] = p[idx].mean(axis=0)
    return out


def resample(p, step):
    d = np.hypot(*(np.roll(p, -1, axis=0) - p).T)
    total = d.sum()
    n = max(8, int(round(total / step)))
    cum = np.concatenate([[0], np.cumsum(d)])
    q = np.linspace(0, total, n, endpoint=False)
    closed = np.vstack([p, p[:1]])
    return np.stack([np.interp(q, cum, closed[:, 0]), np.interp(q, cum, closed[:, 1])], axis=1)


def signed_area(p):
    return sum(x0 * y1 - x1 * y0 for (x0, y0), (x1, y1) in zip(p, p[1:] + p[:1])) / 2


def main(path, name, solid=False):
    v = field(path)
    ys, xs = np.nonzero(v >= 0.5)
    size = max(xs.max() - xs.min(), ys.max() - ys.min())
    result = []
    for c in contours(v):
        p = resample(smooth(c, 2), size / 45)
        result.append([(round(float(x), 1), round(float(y), 1)) for x, y in p])
    if solid:  # holes run the other way from the largest outline
        big = max(result, key=lambda o: abs(signed_area(o)))
        result = [o for o in result if signed_area(o) * signed_area(big) > 0]
    print("%s = %r" % (name, result))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], "solid" in sys.argv[3:])
