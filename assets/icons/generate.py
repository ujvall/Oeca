"""Oeca's own icons, drawn from scratch: rounded polygons, circles and arcs
on a 24 x 24 grid, emitted as cubic-Bezier outlines.

Solid shapes run one way and holes the other, so the icons fill with the
nonzero (winding) rule: overlapping solids merge, holes cut out.

Usage: python generate.py preview.html icons.rs
  preview.html: every icon large and at 12-20 px, to check by eye
  icons.rs: the IconSeg constants, to paste over those in src/renderer.rs
"""
import math
import sys

KAPPA = 4.0 / 3.0


def area(pts):
    return sum(x0 * y1 - x1 * y0 for (x0, y0), (x1, y1) in zip(pts, pts[1:] + pts[:1])) / 2


def rounded_polygon(pts, radii, hole=False):
    """Closed outline through `pts`, each corner rounded by its radius
    (convex or concave alike). Returns a list of segments."""
    if isinstance(radii, (int, float)):
        radii = [radii] * len(pts)
    if (area(pts) < 0) != hole:  # solids positive (clockwise on screen)
        pts, radii = pts[::-1], radii[::-1]
    n = len(pts)
    corners = []
    for i in range(n):
        p0, p, p1 = pts[i - 1], pts[i], pts[(i + 1) % n]
        din = norm(sub(p, p0))
        dout = norm(sub(p1, p))
        turn = math.acos(max(-1.0, min(1.0, dot(din, dout))))
        r = radii[i]
        if r <= 0 or turn < 1e-6:
            corners.append((p, p, p, p))
            continue
        d = r * math.tan(turn / 2)
        a = sub(p, mul(din, d))
        b = add(p, mul(dout, d))
        k = KAPPA * math.tan(turn / 4) * r
        corners.append((a, add(a, mul(din, k)), sub(b, mul(dout, k)), b))
    segs = [("M", corners[0][3])]
    for i in range(1, n + 1):
        a, c1, c2, b = corners[i % n]
        segs.append(("L", a))
        if a != b:
            segs.append(("C", c1, c2, b))
    segs.append(("Z",))
    return segs


def arc(cx, cy, r, a0, a1):
    """Cubic segments along a circle from angle a0 to a1 (radians, y down)."""
    n = max(1, math.ceil(abs(a1 - a0) / (math.pi / 2)))
    out = []
    step = (a1 - a0) / n
    k = KAPPA * math.tan(step / 4) * r
    for i in range(n):
        t0, t1 = a0 + i * step, a0 + (i + 1) * step
        p0 = (cx + r * math.cos(t0), cy + r * math.sin(t0))
        p1 = (cx + r * math.cos(t1), cy + r * math.sin(t1))
        c1 = (p0[0] - k * math.sin(t0), p0[1] + k * math.cos(t0))
        c2 = (p1[0] + k * math.sin(t1), p1[1] - k * math.cos(t1))
        out.append(("C", c1, c2, p1))
    return out


def circle(cx, cy, r, hole=False):
    a0, a1 = (0, 2 * math.pi) if not hole else (2 * math.pi, 0)
    return [("M", (cx + r, cy))] + arc(cx, cy, r, a0, a1) + [("Z",)]


def rrect(x, y, w, h, r, hole=False):
    return rounded_polygon([(x, y), (x + w, y), (x + w, y + h), (x, y + h)], r, hole)


def svg(d):
    """Segments of an SVG path (M L H V C A Z, absolute or relative). Arcs
    become cubics; the path's own directions are kept (nonzero fill)."""
    import re
    toks = re.findall(r"[MmLlHhVvCcAaZz]|-?(?:\d+\.?\d*|\.\d+)(?:e-?\d+)?", d)
    i, cmd, cur, start, segs = 0, None, (0.0, 0.0), (0.0, 0.0), []

    def num():
        nonlocal i
        i += 1
        return float(toks[i - 1])

    while i < len(toks):
        if toks[i].isalpha():
            cmd = toks[i]
            i += 1
        rel, c = cmd.islower(), cmd.upper()
        o = cur if rel else (0.0, 0.0)
        if c == "Z":
            segs.append(("Z",))
            cur = start
            continue
        if c == "M":
            cur = start = (o[0] + num(), o[1] + num())
            segs.append(("M", cur))
            cmd = "l" if rel else "L"
        elif c == "L":
            cur = (o[0] + num(), o[1] + num())
            segs.append(("L", cur))
        elif c == "H":
            cur = ((cur[0] if rel else 0.0) + num(), cur[1])
            segs.append(("L", cur))
        elif c == "V":
            cur = (cur[0], (cur[1] if rel else 0.0) + num())
            segs.append(("L", cur))
        elif c == "C":
            c1, c2 = (o[0] + num(), o[1] + num()), (o[0] + num(), o[1] + num())
            cur = (o[0] + num(), o[1] + num())
            segs.append(("C", c1, c2, cur))
        elif c == "A":
            rx, ry, rot, large, sweep = num(), num(), num(), num(), num()
            end = (o[0] + num(), o[1] + num())
            segs += svg_arc(cur, end, rx, ry, math.radians(rot), large != 0, sweep != 0)
            cur = end
    return segs


def svg_arc(p0, p1, rx, ry, phi, large, sweep):
    """An SVG endpoint arc as cubics (SVG 1.1, appendix F.6)."""
    if p0 == p1:
        return []
    if rx == 0 or ry == 0:
        return [("L", p1)]
    rx, ry = abs(rx), abs(ry)
    cs, sn = math.cos(phi), math.sin(phi)
    dx, dy = (p0[0] - p1[0]) / 2, (p0[1] - p1[1]) / 2
    x1, y1 = cs * dx + sn * dy, -sn * dx + cs * dy
    lam = (x1 / rx) ** 2 + (y1 / ry) ** 2
    if lam > 1:
        rx, ry = rx * math.sqrt(lam), ry * math.sqrt(lam)
    num_ = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
    k = math.sqrt(max(0.0, num_ / (rx * rx * y1 * y1 + ry * ry * x1 * x1)))
    if large == sweep:
        k = -k
    cxp, cyp = k * rx * y1 / ry, -k * ry * x1 / rx
    cx = cs * cxp - sn * cyp + (p0[0] + p1[0]) / 2
    cy = sn * cxp + cs * cyp + (p0[1] + p1[1]) / 2
    ang = lambda ux, uy, vx, vy: math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
    t0 = ang(1, 0, (x1 - cxp) / rx, (y1 - cyp) / ry)
    dt = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry)
    if not sweep and dt > 0:
        dt -= 2 * math.pi
    elif sweep and dt < 0:
        dt += 2 * math.pi
    n = max(1, math.ceil(abs(dt) / (math.pi / 2)))
    step = dt / n
    kk = KAPPA * math.tan(step / 4)
    pt = lambda t: (cx + rx * math.cos(t) * cs - ry * math.sin(t) * sn,
                    cy + rx * math.cos(t) * sn + ry * math.sin(t) * cs)
    dpt = lambda t: (-rx * math.sin(t) * cs - ry * math.cos(t) * sn,
                     -rx * math.sin(t) * sn + ry * math.cos(t) * cs)
    out = []
    for j in range(n):
        a, b = t0 + j * step, t0 + (j + 1) * step
        pa, pb, da, db = pt(a), pt(b), dpt(a), dpt(b)
        out.append(("C", add(pa, mul(da, kk)), sub(pb, mul(db, kk)), pb))
    out[-1] = out[-1][:3] + (p1,)
    return out


def with_room(pts, radii, room):
    """Drops unrounded points closer than `room` to a rounded corner, so a
    finely sampled curve leaves space for the corner's fillet."""
    corners = [p for p, r in zip(pts, radii) if r > 0]
    keep = [(p, r) for p, r in zip(pts, radii)
            if r > 0 or all(math.dist(p, c) > room for c in corners)]
    return [p for p, _ in keep], [r for _, r in keep]


def sub(a, b): return (a[0] - b[0], a[1] - b[1])
def add(a, b): return (a[0] + b[0], a[1] + b[1])
def mul(a, k): return (a[0] * k, a[1] * k)
def dot(a, b): return a[0] * b[0] + a[1] * b[1]
def norm(a):
    l = math.hypot(*a)
    return (a[0] / l, a[1] / l)


# ── The icons ─────────────────────────────────────────────────────────────

def home():
    # A soft-roofed house with an arched door
    return rounded_polygon(
        [(12, 2.6), (21.2, 10.2), (21.2, 21.4), (14.6, 21.4), (14.6, 15.6),
         (9.4, 15.6), (9.4, 21.4), (2.8, 21.4), (2.8, 10.2)],
        [2.6, 1.9, 2.6, 1.1, 2.1, 2.1, 1.1, 2.6, 1.9],
    )


def music():
    # Two beamed notes
    beam = rounded_polygon([(8.4, 6.9), (21.0, 3.6), (21.0, 7.5), (8.4, 10.8)], 1.1)
    return (circle(6.6, 17.8, 3.6) + circle(17.4, 15.2, 3.6)
            + rrect(8.3, 7.2, 1.9, 10.6, 0.9) + rrect(19.1, 4.4, 1.9, 10.8, 0.9) + beam)


def clipboard():
    # A board with a notch at the top, and its clip resting in it
    board = rounded_polygon(
        [(3.6, 4.6), (6.6, 4.6), (6.6, 8.6), (17.4, 8.6), (17.4, 4.6), (20.4, 4.6),
         (20.4, 22.2), (3.6, 22.2)],
        [3.0, 0.9, 1.6, 1.6, 0.9, 3.0, 3.0, 3.0],
    )
    return board + rrect(8.8, 1.8, 6.4, 4.8, 1.8)


def gear():
    teeth, outer, root = 8, 11.2, 8.4
    top, base = math.radians(10.5), math.radians(17.5)
    pts, radii = [], []
    for i in range(teeth):
        a = i * 2 * math.pi / teeth - math.pi / 2
        for ang, rad, rr in ((a - base, root, 0.9), (a - top, outer, 1.0),
                             (a + top, outer, 1.0), (a + base, root, 0.9)):
            pts.append((12 + rad * math.cos(ang), 12 + rad * math.sin(ang)))
            radii.append(rr)
    return rounded_polygon(pts, radii) + circle(12, 12, 3.4, hole=True)


def palette():
    # A round palette with a rounded thumb notch and four paint wells
    (cx, cy, r), (nx, ny, nr) = (12.0, 12.0, 10.2), (18.6, 18.0, 3.6)
    d = math.hypot(nx - cx, ny - cy)
    base = math.atan2(ny - cy, nx - cx)
    half = math.acos((r * r + d * d - nr * nr) / (2 * r * d))
    p_a, p_b = base - half, base + half
    rim = lambda t: (cx + r * math.cos(t), cy + r * math.sin(t))
    q_a = math.atan2(rim(p_a)[1] - ny, rim(p_a)[0] - nx)
    q_b = math.atan2(rim(p_b)[1] - ny, rim(p_b)[0] - nx)
    if q_a < q_b:
        q_a += 2 * math.pi
    # The outline as a fine polygon (the rim the long way round, then the
    # notch), so the two points where they meet can be filleted
    steps = 96
    pts = [rim(p_b + (p_a + 2 * math.pi - p_b) * i / steps) for i in range(steps)]
    cusp_a = len(pts)
    pts.append(rim(p_a))
    pts += [(nx + nr * math.cos(q_a + (q_b - q_a) * i / 24),
             ny + nr * math.sin(q_a + (q_b - q_a) * i / 24)) for i in range(1, 24)]
    # Clear room around each meeting point for its fillet
    fillet, room = 1.4, 2.0
    corners = {cusp_a: pts[cusp_a], 0: pts[0]}
    keep = [(p, fillet if i in corners else 0.0) for i, p in enumerate(pts)
            if i in corners or all(math.dist(p, c) > room for c in corners.values())]
    outline = rounded_polygon([p for p, _ in keep], [rr for _, rr in keep])
    wells = (circle(7.6, 10.4, 2.1, True) + circle(12.4, 6.9, 2.1, True)
             + circle(17.0, 9.6, 2.1, True) + circle(7.9, 15.7, 2.1, True))
    return outline + wells


def info():
    return (circle(12, 12, 10.2) + circle(12, 7.4, 1.6, True)
            + rrect(10.6, 10.4, 2.8, 8.0, 1.4, True))


def copy():
    # Ujval's copy icon (20 x 20 SVG): a sheet in front of an outlined one
    return svg(
        "M14.256 3.513A2.5 2.5 0 0 1 16.5 6v4l-.013.256a2.5 2.5 0 0 1-2.231 2.231L14 12.5h-1.5V14"
        "a2.5 2.5 0 0 1-2.5 2.5H6A2.5 2.5 0 0 1 3.5 14v-4A2.5 2.5 0 0 1 6 7.5h1.5V6A2.5 2.5 0 0 1 10 3.5h4z"
        "M10 5a1 1 0 0 0-1 1v1.5h1a2.5 2.5 0 0 1 2.5 2.5v1H14a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1z"
    )


def trash():
    # A lid with its handle, just wider than the straight-sided, solid bin
    lid = rrect(4.0, 4.4, 16.0, 2.8, 1.4) + rrect(8.8, 1.8, 6.4, 3.6, 1.4)
    body = rounded_polygon([(4.8, 9.2), (19.2, 9.2), (19.2, 22.2), (4.8, 22.2)],
                           [1.0, 1.0, 2.4, 2.4])
    return lid + body


def tray():
    # An open tray: slanted walls, the opening cut out above a solid front
    # panel whose top dips in a round notch
    outer = rounded_polygon(
        [(5.0, 4.6), (19.0, 4.6), (22.5, 10.6), (22.5, 19.2), (1.5, 19.2), (1.5, 10.6)],
        [2.0, 2.0, 1.0, 2.4, 2.4, 1.0],
    )
    dip = [(12 + 3.0 * math.cos(t), 10.0 + 3.0 * math.sin(t))
           for t in (math.pi * k / 32 for k in range(33))]
    pts = [(6.6, 6.6), (17.4, 6.6), (20.1, 10.0)] + dip + [(3.9, 10.0)]
    radii = [0.7, 0.7, 0.5] + [0.9] + [0.0] * 31 + [0.9] + [0.5]
    pts, radii = with_room(pts, radii, 1.4)
    return outer + rounded_polygon(pts, radii, hole=True)


def stroke(pts, w):
    """The outline of a polyline `w` wide, with fully round ends and a
    rounded joint: one closed shape (so it can be cut out whole)."""
    h = w / 2
    left, right = [], []
    for i, p in enumerate(pts):
        dirs = []
        if i > 0:
            dirs.append(norm(sub(p, pts[i - 1])))
        if i < len(pts) - 1:
            dirs.append(norm(sub(pts[i + 1], p)))
        d = norm(add(dirs[0], dirs[-1]))
        n = (-d[1], d[0])
        # Miter: the offset grows as the polyline turns
        cos_half = dot(n, (-dirs[0][1], dirs[0][0]))
        k = h / max(cos_half, 0.2)
        left.append(add(p, mul(n, k)))
        right.append(sub(p, mul(n, k)))
    outline = left + right[::-1]
    radii = [h * 0.98] + [h * 0.5] * (len(pts) - 2) + [h * 0.98] * 2 + [h * 0.5] * (len(pts) - 2) + [h * 0.98]
    return outline, radii


def done():
    # A check cut out of a disc
    pts, radii = stroke([(7.2, 12.6), (10.6, 16.0), (17.0, 9.0)], 2.6)
    return circle(12, 12, 10.2) + rounded_polygon(pts, radii, hole=True)


def alert():
    # An exclamation mark cut out of a disc
    return (circle(12, 12, 10.2) + rrect(10.6, 5.6, 2.8, 8.0, 1.4, True)
            + circle(12, 17.0, 1.6, True))


ICONS = [
    ("HOUSE_BLANK", "Home: a soft-roofed house with an arched door; the Home space.", home, False),
    ("MUSIC_ALT", "Music: two beamed notes; the Music space.", music, True),
    ("CLIPBOARD", "Clipboard: a board and its clip; the Clipboard space.", clipboard, True),
    ("PALETTE", "Palette: three paint wells and a thumb notch; Settings window, Appearance.", palette, True),
    ("INFO", "Info: an \"i\" cut from a disc; Settings window, About.", info, True),
    ("GEAR", "Settings: an eight-tooth gear; the header's Settings icon.", gear, True),
    ("COPY", "Copy: a sheet in front of an outlined one; clipboard row copy button.", copy, False),
    ("TRASH", "Trash: a lid over a solid bin; clipboard row delete button.", trash, False),
    ("INBOX", "Tray: an open tray with a dipped front; the drop page glyph.", tray, False),
    ("DONE", "Done: a check cut from a disc; a drop's success.", done, False),
    ("ALERT", "Alert: an exclamation mark cut from a disc; a drop that failed.", alert, False),
]


def points(segs):
    for s in segs:
        for p in s[1:]:
            yield p


def svg_path(segs, fit=None):
    f = fit or (lambda p: p)
    out = []
    for s in segs:
        if s[0] == "Z":
            out.append("Z")
        else:
            out.append(s[0] + " ".join("%.3f %.3f" % f(p) for p in s[1:]))
    return " ".join(out)


# Constants Clippy's `approx_constant` lint recognises within the unit box
# (1/pi, log10(e), pi/8, log10(2)), as 4-digit literals rounded or truncated
LINT_CONSTANTS = {f"{c:.4f}" for c in (1 / math.pi, math.log10(math.e), math.pi / 8, math.log10(2))} | {
    f"{math.floor(c * 1e4) / 1e4:.4f}"
    for c in (1 / math.pi, math.log10(math.e), math.pi / 8, math.log10(2))
}


def rust_num(v):
    """A coordinate as a 4-digit Rust literal, nudged by 0.0001-0.0002 (far
    below a pixel) if Clippy would take it for a mathematical constant."""
    text = f"{v:.4f}"
    step = 0.0001 if v >= 0 else -0.0001
    while text.lstrip("-") in LINT_CONSTANTS:
        v += step
        text = f"{v:.4f}"
    return "0.0000" if text == "-0.0000" else text


def unit_fit(segs):
    xs, ys = zip(*points(segs))
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    s = 1 / max(x1 - x0, y1 - y0)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    return lambda p: ((p[0] - cx) * s, (p[1] - cy) * s)


def main(html_path, rs_path):
    tiles = []
    rs = []
    for name, doc, make, public in ICONS:
        segs = make()
        fit = unit_fit(segs)
        d = svg_path(segs, lambda p: tuple(v + 0.5 for v in fit(p)))
        big = '<svg width="96" height="96" viewBox="0 0 1 1"><path fill="#fff" fill-rule="nonzero" d="%s"/></svg>' % d
        small = "".join('<svg width="%d" height="%d" viewBox="0 0 1 1"><path fill="#fff" d="%s"/></svg>' % (n, n, d) for n in (12, 16, 20))
        tiles.append('<figure><div class="big">%s</div><div class="small">%s</div><figcaption>%s</figcaption></figure>' % (big, small, name))
        rs.append("/// %s" % doc)
        rs.append("%sconst %s: &[IconSeg] = &[" % ("pub(crate) " if public else "", name))
        for s in segs:
            if s[0] == "Z":
                rs.append("    IconSeg::Z,")
            else:
                vals = ", ".join(rust_num(v) for p in s[1:] for v in fit(p))
                rs.append("    IconSeg::%s(%s)," % (s[0], vals))
        rs.append("];")
    html = """<!doctype html><meta charset="utf-8"><title>Oeca icons</title>
<style>body{background:#000;color:#999;font:12px system-ui;margin:24px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,140px);gap:20px}
figure{margin:0;padding:14px;background:#111;border-radius:14px;text-align:center}
.big svg{display:block;margin:auto}.small{margin-top:12px;display:flex;gap:10px;justify-content:center;align-items:end}
figcaption{margin-top:10px}</style><div class="grid">%s</div>""" % "".join(tiles)
    open(html_path, "w", encoding="utf-8").write(html)
    open(rs_path, "w", encoding="utf-8").write("\n".join(rs) + "\n")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
