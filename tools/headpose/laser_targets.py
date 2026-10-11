#!/usr/bin/env python3
"""Laser targets for measuring head turns: hollow rings on black, as an SVG to show full screen,
and their positions as JSON for truth.py.

    laser_targets.py OUT.svg [--json OUT.json] [--screen-mm W H] [--pivot-mm D]

A laser pointer fixed to the head (on a cap's visor, say) is aimed at each ring in turn; with the
screen's geometry and the device's eyeball centres, truth.py turns the rings' positions into the
head's true yaw and pitch at each hold (tools/headpose/README.md, "Laser truth"). The SVG's
viewBox is the screen in mm, origin at its centre, so a browser showing it full screen
(preserveAspectRatio meet) puts every ring where the JSON says, whatever the screen's resolution
or scaling; a 100 mm bar in the corner checks that with a ruler.

The rings: '0' in the centre (the beam's spot with the head straight), yaw L/R 5, 10, 20 and 25
deg (L: the head turned to its left, the spot to the screen's left), pitch U/D 5, 10 and 15 deg
(U: chin up), and the four combinations of 20 deg of yaw with 10 of pitch. A ring of nominal yaw y
and pitch p sits at x = -D tan y, y = D tan p / cos y from the centre, D the distance from the
head's pivot to the screen (--pivot-mm, 600: a face 500 mm away and ~100 mm to the neck's axis);
the angles are labels only, truth.py computes the true ones from the positions. The defaults are
the 598 x 337 mm screen the 2026-10 recordings were made on; laser_targets.json is their output.
"""

import argparse
import json
import math

RING_RADIUS_MM = 5.0
RING_STROKE_MM = 0.9
EDGE_MARGIN_MM = 2.0
TARGETS = (
    [(0, 0)]
    + [(s * a, 0) for a in (5, 10, 20, 25) for s in (1, -1)]
    + [(0, s * a) for a in (5, 10, 15) for s in (1, -1)]
    + [(sy * 20, sp * 10) for sy in (1, -1) for sp in (1, -1)]
)


def label(yaw, pitch):
    """'0', 'L20', 'D10', 'L20 U10', ..."""
    if yaw == 0 and pitch == 0:
        return "0"
    parts = []
    if yaw:
        parts.append(("L" if yaw > 0 else "R") + str(abs(yaw)))
    if pitch:
        parts.append(("U" if pitch > 0 else "D") + str(abs(pitch)))
    return " ".join(parts)


def targets(screen_mm, pivot_mm):
    """Every ring: label, nominal angles (deg) and position (mm from the screen's centre, x right,
    y up)."""
    out = []
    for yaw, pitch in TARGETS:
        x = -pivot_mm * math.tan(math.radians(yaw))
        y = pivot_mm * math.tan(math.radians(pitch)) / math.cos(math.radians(yaw))
        room_x = screen_mm[0] / 2 - abs(x) - RING_RADIUS_MM
        room_y = screen_mm[1] / 2 - abs(y) - RING_RADIUS_MM
        if min(room_x, room_y) < EDGE_MARGIN_MM:
            raise SystemExit(
                f"error: ring {label(yaw, pitch)} does not fit a {screen_mm[0]:g} x "
                f"{screen_mm[1]:g} mm screen at a pivot of {pivot_mm:g} mm"
            )
        out.append(
            dict(
                label=label(yaw, pitch),
                yaw_deg_nominal=yaw,
                pitch_deg_nominal=pitch,
                x_mm=round(x, 3),
                y_mm=round(y, 3),
            )
        )
    return out


def svg(rings, screen_mm):
    """The page: the screen's mm as the viewBox, y down as SVG has it."""
    w, h = screen_mm
    ring = (
        '<circle cx="{x:.3f}" cy="{y:.3f}" r="{r}" fill="none" stroke="#d7d7d7" '
        'stroke-width="{s}"/>'
    )
    text = (
        '<text x="{x:.3f}" y="{y:.3f}" fill="#5f5f5f" font-family="sans-serif" '
        'font-size="3.2" text-anchor="middle">{t}</text>'
    )
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%" '
        f'viewBox="{-w / 2:g} {-h / 2:g} {w:g} {h:g}" preserveAspectRatio="xMidYMid meet" '
        'style="background:#000">',
        f'<rect x="{-w / 2:g}" y="{-h / 2:g}" width="{w:g}" height="{h:g}" fill="#000"/>',
    ]
    for r in rings:
        x, y = r["x_mm"], -r["y_mm"]
        parts.append(ring.format(x=x, y=y, r=RING_RADIUS_MM, s=RING_STROKE_MM))
        if r["label"] == "0":
            parts.append(ring.format(x=x, y=y, r=1.8 * RING_RADIUS_MM, s=RING_STROKE_MM / 2))
        below = y + (2.2 if r["label"] == "0" else 1.0) * RING_RADIUS_MM + 4.5
        ty = below if below < h / 2 - 2 else y - RING_RADIUS_MM - 2.5
        parts.append(text.format(x=x, y=ty, t=r["label"]))
    x0, yb = -w / 2 + 12.0, h / 2 - 11.0
    parts.append(
        f'<path d="M{x0:g} {yb - 2:g}V{yb + 2:g}M{x0:g} {yb:g}H{x0 + 100:g}M{x0 + 100:g} '
        f'{yb - 2:g}V{yb + 2:g}" stroke="#5f5f5f" stroke-width="0.5" fill="none"/>'
    )
    parts.append(text.format(x=x0 + 8, y=yb - 3.5, t="100 mm"))
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("svg", help="the page to write")
    ap.add_argument("--json", help="also write the rings' positions (truth.py's --targets)")
    ap.add_argument("--screen-mm", type=float, nargs=2, default=(598.0, 337.0), metavar=("W", "H"))
    ap.add_argument("--pivot-mm", type=float, default=600.0)
    args = ap.parse_args()
    rings = targets(tuple(args.screen_mm), args.pivot_mm)
    with open(args.svg, "w") as f:
        f.write(svg(rings, args.screen_mm))
    if args.json:
        doc = dict(
            about="Laser targets (tools/headpose/laser_targets.py): each ring's label, nominal "
            "yaw and pitch (deg) and position on the screen (mm from its centre, x right, y up). "
            "truth.py takes the positions; the angles are labels.",
            screen_mm=list(args.screen_mm),
            pivot_mm_nominal=args.pivot_mm,
            ring_radius_mm=RING_RADIUS_MM,
            targets=rings,
        )
        with open(args.json, "w") as f:
            json.dump(doc, f, indent=1)
            f.write("\n")
    for r in rings:
        print(f"{r['label']:>8}: x {r['x_mm']:+8.1f} mm  y {r['y_mm']:+7.1f} mm")


if __name__ == "__main__":
    main()
