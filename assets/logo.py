"""Generates assets/logo.svg: an isometric rusty key ring.

The key lies flat in the z = 0 plane and has thickness. The ring stands
upright in the y-z plane and goes through the hole of the key's bow.

Run: python3 assets/logo.py > assets/logo.svg
"""

import math

COS30 = math.cos(math.radians(30))
SIN30 = 0.5
SCALE = 1.0

# Palette: light comes from the top left.
TOP_LIGHT = "#E8834A"
TOP_DARK = "#B0491A"
SIDE = "#6E2B12"
SIDE_EDGE = "#5A220E"
RIM = "#F4A774"
RUST = "#7A2E14"
RUST_LIGHT = "#9C4220"
RING = "#8C3B1E"
RING_LIGHT = "#C96A3A"
RING_SHADE = "#5E2410"

DEPTH = 7  # key thickness, in screen units


def project(x, y, z):
    return ((x - y) * COS30 * SCALE, ((x + y) * SIN30 - z) * SCALE)


def circle(cx, cy, r, n=72):
    return [(cx + r * math.cos(2 * math.pi * i / n), cy + r * math.sin(2 * math.pi * i / n)) for i in range(n)]


def rect(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


def blob(cx, cy, r, seed, n=14):
    """An irregular closed shape for a rust patch."""
    pts = []
    for i in range(n):
        a = 2 * math.pi * i / n
        wobble = 0.65 + 0.35 * math.sin(seed * 12.9898 + i * 78.233) ** 2
        pts.append((cx + r * wobble * math.cos(a), cy + r * wobble * math.sin(a)))
    return pts


def path(rings, dy=0.0):
    """SVG path data for flat polygons in z = 0, moved down by dy screen units."""
    out = []
    for poly in rings:
        pts = [project(x, y, 0) for x, y in poly]
        out.append("M " + " L ".join(f"{px:.2f} {py + dy:.2f}" for px, py in pts) + " Z")
    return " ".join(out)


# The key, in the z = 0 plane. The shaft runs along +x.
BOW = [circle(0, 0, 19), circle(0, 0, 7)]  # outer edge and hole
COLLAR = [rect(16, -7.5, 21, 7.5)]
SHAFT = [rect(17, -4.5, 70, 4.5)]
TEETH = [rect(48, 4, 55, 13), rect(60, 4, 70, 10), rect(64, 10, 70, 15)]
KEY_PARTS = [BOW, SHAFT, COLLAR, *[[t] for t in TEETH]]

RUST_SPOTS = [
    blob(-9, -10, 5.5, 1),
    blob(10, 8, 3.2, 2),
    blob(31, -1, 3.6, 3),
    blob(44, 1, 2.2, 4),
    blob(66, 6, 2.6, 5),
]
RUST_FLECKS = [blob(-12, 6, 2.0, 6), blob(4, -15, 1.6, 7), blob(25, 2, 1.4, 8)]

# The ring stands in the y-z plane. It crosses the key plane in the hole
# (y = 0) and outside the bow (y = -2R).
R = 26


def ring_points(t0, t1, n=60):
    pts = []
    for i in range(n + 1):
        t = t0 + (t1 - t0) * i / n
        pts.append(project(0, -R + R * math.cos(t), R * math.sin(t)))
    return pts


def polyline(pts, dx=0.0, dy=0.0):
    return "M " + " L ".join(f"{x + dx:.2f} {y + dy:.2f}" for x, y in pts)


def bounds():
    pts = [project(x, y, 0) for part in KEY_PARTS for poly in part for x, y in poly]
    pts += ring_points(0, 2 * math.pi)
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return min(xs), min(ys), max(xs), max(ys) + DEPTH


def main():
    x0, y0, x1, y1 = bounds()
    pad = 14
    size = max(x1 - x0, y1 - y0 + 12) + 2 * pad
    cx, cy = (x0 + x1) / 2, (y0 + y1 + 12) / 2
    vb = f"{cx - size / 2:.1f} {cy - size / 2:.1f} {size:.1f} {size:.1f}"

    shadow_c = project(30, 8, 0)
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" role="img" aria-labelledby="title">',
        '  <title id="title">rouille: a rusty key ring</title>',
        "  <defs>",
        '    <linearGradient id="top" x1="0" y1="0" x2="1" y2="1">',
        f'      <stop offset="0" stop-color="{TOP_LIGHT}"/>',
        f'      <stop offset="1" stop-color="{TOP_DARK}"/>',
        "    </linearGradient>",
        '    <filter id="blur" x="-50%" y="-50%" width="200%" height="200%">',
        '      <feGaussianBlur stdDeviation="4"/>',
        "    </filter>",
        "  </defs>",
        # Shadow on the ground, below the key.
        f'  <ellipse cx="{shadow_c[0]:.1f}" cy="{shadow_c[1] + DEPTH + 16:.1f}" rx="52" ry="11"'
        ' fill="#3A1A0C" opacity="0.22" filter="url(#blur)"/>',
    ]

    # Lower half of the ring: behind and under the key.
    back = ring_points(math.pi, 2 * math.pi)
    out.append(f'  <path d="{polyline(back)}" fill="none" stroke="{RING_SHADE}" stroke-width="7" stroke-linecap="round"/>')

    # Key thickness: stacked copies of the outline, darkest at the bottom.
    for k in range(DEPTH, 0, -1):
        color = SIDE_EDGE if k == DEPTH else SIDE
        for part in KEY_PARTS:
            # The stroke closes the anti-aliasing seams between layers.
            out.append(
                f'  <path d="{path(part, k)}" fill="{color}" fill-rule="evenodd"'
                f' stroke="{color}" stroke-width="1.2" stroke-linejoin="round"/>'
            )

    # Top faces.
    for part in KEY_PARTS:
        out.append(f'  <path d="{path(part)}" fill="url(#top)" fill-rule="evenodd"/>')
    # Lit rim on the bow's top-left edge.
    rim = [project(x, y, 0) for x, y in circle(0, 0, 19)[30:52]]
    out.append(f'  <path d="{polyline(rim)}" fill="none" stroke="{RIM}" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/>')

    # Rust.
    out.append(f'  <path d="{path(RUST_SPOTS)}" fill="{RUST}" opacity="0.9"/>')
    out.append(f'  <path d="{path(RUST_FLECKS)}" fill="{RUST_LIGHT}" opacity="0.85"/>')

    # Upper half of the ring: in front of the key.
    front = ring_points(0, math.pi)
    out.append(f'  <path d="{polyline(front)}" fill="none" stroke="{RING}" stroke-width="7" stroke-linecap="round"/>')
    out.append(
        f'  <path d="{polyline(front[18:44], -1.2, -1.6)}" fill="none" stroke="{RING_LIGHT}"'
        ' stroke-width="2" stroke-linecap="round" opacity="0.9"/>'
    )

    # Flakes falling off the tip.
    tip = project(70, 12, 0)
    for dx, dy, r, color in [(-2, 15, 2.0, TOP_DARK), (5, 22, 1.4, RUST), (-1, 28, 1.0, TOP_DARK)]:
        out.append(f'  <circle cx="{tip[0] + dx:.1f}" cy="{tip[1] + dy:.1f}" r="{r}" fill="{color}"/>')

    out.append("</svg>")
    print("\n".join(out))


if __name__ == "__main__":
    main()
