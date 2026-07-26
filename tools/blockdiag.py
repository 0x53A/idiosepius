#!/usr/bin/env python3
"""Draw block diagrams and sketches as self-contained SVG, in the app's visual
language.

A figure of kind `svg` is rasterised once by resvg (`crates/app/src/plot.rs`)
and cached by content hash, so an authored diagram is a string of strokes,
fills and text with no external reference of any kind. Hand-writing that is
miserable and inconsistent; this is the kit the packs use instead.

It is deliberately only a *kit*. The diagrams of a particular course are
particular to that course, so they live in that module's repository and import
this — see `content/control-systems/cs-diagrams.py` for a worked set.

    import sys; sys.path.insert(0, "../../tools")
    import blockdiag as bd

    bd.svg(640, 150,
        bd.arrow(20, 74, 165, 74),
        bd.block(230, 74, "H", "ct"),
        bd.arrow(295, 74, 618, 74))

Two families of element live here. The block-diagram parts — `block`, `arrow`,
`elbow`, `junction`, `dot`, `hull`, `fraction_block` — draw a loop. The sketch
parts — `curve`, `cross`, `ring`, `arc`, `guide`, `sketch_axes` — draw the
things a generated `bode` or `step` figure cannot: an annotated response, a
pole in the $s$-plane, a tangent at an operating point. They deliberately match
`crates/app/src/plot.rs`, so an authored sketch and a generated plot read as
the same family: axes in `EDGE`, the curve that carries the meaning in
`ACCENT`, guides dashed and faint, a pole marked with an `INK` cross.

Two things decide whether a diagram reads well in the app:

* **Keep it wide.** The reader gives a figure the column width and clamps its
  height to 280 px, so a viewBox taller than about half its width is scaled
  down until the labels are unreadable. Aspect ratios between 2.5 and 4 work.
* **Draw at viewBox scale, not screen scale.** These numbers are laid out for a
  640-unit viewBox rendered at roughly 560 px, so a 24-unit block label lands
  at about 21 px. Changing the viewBox width means rescaling the type.

Style: `python3 tools/blockdiag.py --demo target/blockdiag` renders a sheet of
every element, which is the quickest way to see what a change did.
"""

# The palette is `crates/app/src/theme.rs`. Kept as literals rather than parsed
# out of the Rust: an SVG is rasterised once at import and cannot follow a
# theme change anyway, so a copy that drifts is visible immediately.
LINE = "#74919A"        # signal lines            Palette::TEXT_DIM
EDGE = "#34525C"        # block borders, axes     Palette::LINE_BRIGHT
FILL = "#111B20"        # block face              Palette::CARD
INK = "#D8E6EA"         # block labels            Palette::TEXT
FAINT = "#465C64"       # captions, guides        Palette::TEXT_FAINT
ACCENT = "#2FE0C8"      # the path that makes it a loop, and the traced curve
VIOLET = "#8C70EC"      # a second curve, as in a two-panel plot  Palette::VIOLET
FONT = "Hack, monospace"

BLOCK_W, BLOCK_H = 130, 62
SUM = 32                # summing junction — a square, like everything else
HEAD = 9                # arrowhead length


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def svg(width, height, *parts):
    """Wrap drawn elements in a viewBox. Returns the string a pack stores."""
    body = "".join(p for p in parts if p)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" '
            f'width="{width}" height="{height}">{body}</svg>')


def rich(x, y, parts, size=17, fill=LINE, anchor="start"):
    """One text element. `parts` is a list of (base, subscript) pairs."""
    spans = []
    for base, sub in parts:
        if base:
            spans.append(esc(base))
        if sub:
            drop = size * 0.26
            spans.append(
                f'<tspan font-size="{size * 0.66:g}" dy="{drop:g}">{esc(sub)}</tspan>')
            # Return to the baseline, so a following span is not also lowered.
            spans.append(f'<tspan dy="{-drop:g}"></tspan>')
    return (f'<text x="{x:g}" y="{y:g}" font-family="{FONT}" font-size="{size:g}" '
            f'fill="{fill}" text-anchor="{anchor}">{"".join(spans)}</text>')


def label(x, y, base, sub="", **kw):
    """A signal name: `label(20, 60, "x", "d")` is x with a subscript d."""
    return rich(x, y, [(base, sub)], **kw)


def caption(x, y, text):
    """One faint line under a diagram, saying what it is."""
    return label(x, y, text, size=15, fill=FAINT, anchor="middle")


def line(d, colour=LINE, width=2, dash=None):
    extra = f' stroke-dasharray="{dash}"' if dash else ""
    return f'<path d="{d}" fill="none" stroke="{colour}" stroke-width="{width}"{extra}/>'


def head(x, y, direction, colour=LINE, size=HEAD):
    """A filled triangle with its tip at (x, y). Markers are avoided on
    purpose: an explicit path renders identically everywhere."""
    if direction in ("right", "left"):
        s = size if direction == "right" else -size
        d = f"M{x:g},{y:g} L{x - s:g},{y - size * 0.55:g} L{x - s:g},{y + size * 0.55:g} Z"
    else:
        s = size if direction == "down" else -size
        d = f"M{x:g},{y:g} L{x - size * 0.55:g},{y - s:g} L{x + size * 0.55:g},{y - s:g} Z"
    return f'<path d="{d}" fill="{colour}"/>'


def arrow(x1, y1, x2, y2, colour=LINE):
    """A straight horizontal or vertical signal, ending in an arrowhead."""
    if y1 == y2:
        direction = "right" if x2 > x1 else "left"
        stop = x2 - HEAD if x2 > x1 else x2 + HEAD
        d = f"M{x1:g},{y1:g} L{stop:g},{y2:g}"
    else:
        direction = "down" if y2 > y1 else "up"
        stop = y2 - HEAD if y2 > y1 else y2 + HEAD
        d = f"M{x1:g},{y1:g} L{x2:g},{stop:g}"
    return line(d, colour) + head(x2, y2, direction, colour)


def elbow(points, colour=LINE, direction="left"):
    """A polyline through right angles, ending in an arrowhead. Used for the
    return path, which is the one thing in a loop diagram that is not straight."""
    d = "M" + " L".join(f"{x:g},{y:g}" for x, y in points)
    return line(d, colour) + head(points[-1][0], points[-1][1], direction, colour)


def dot(x, y, colour=LINE):
    """A take-off point: where a signal branches without being consumed."""
    return f'<circle cx="{x:g}" cy="{y:g}" r="3.5" fill="{colour}"/>'


def block(cx, cy, base, sub="", w=BLOCK_W, h=BLOCK_H, tail=None):
    """A transfer-function block, centred on (cx, cy)."""
    x, y = cx - w / 2, cy - h / 2
    out = [f'<rect x="{x:g}" y="{y:g}" width="{w:g}" height="{h:g}" '
           f'fill="{FILL}" stroke="{EDGE}" stroke-width="2"/>',
           rich(cx, cy + 8, [(base, sub)], size=24, fill=INK, anchor="middle")]
    if tail:
        out.append(rich(cx, y + h + 24, tail, size=15, fill=FAINT, anchor="middle"))
    return "".join(out)


def fraction_block(cx, cy, top, bottom, w=150, h=76):
    """A block whose label is a fraction, set over a rule rather than with a
    slash — the same reason the math renderer exists."""
    x, y = cx - w / 2, cy - h / 2
    return "".join([
        f'<rect x="{x:g}" y="{y:g}" width="{w:g}" height="{h:g}" '
        f'fill="{FILL}" stroke="{EDGE}" stroke-width="2"/>',
        rich(cx, cy - 8, top, size=19, fill=INK, anchor="middle"),
        line(f"M{cx - w / 2 + 16:g},{cy:g} L{cx + w / 2 - 16:g},{cy:g}", INK, width=1.5),
        rich(cx, cy + 24, bottom, size=19, fill=INK, anchor="middle"),
    ])


def junction(cx, cy, signs=(("left", "+"), ("bottom", "−"))):
    """A summing junction. Square, not the traditional circle: DESIGN.md has no
    rounded anything, and the signs carry the meaning in any case."""
    x, y = cx - SUM / 2, cy - SUM / 2
    out = [f'<rect x="{x:g}" y="{y:g}" width="{SUM}" height="{SUM}" '
           f'fill="{FILL}" stroke="{EDGE}" stroke-width="2"/>']
    where = {"left": (cx - SUM / 2 - 10, cy - 10),
             "right": (cx + SUM / 2 + 10, cy - 10),
             "top": (cx + 14, cy - SUM / 2 - 6),
             "top-left": (cx - 14, cy - SUM / 2 - 6),
             "bottom": (cx + 14, cy + SUM / 2 + 20)}
    for side, sign in signs:
        px, py = where[side]
        out.append(label(px, py, sign, size=19, fill=INK, anchor="middle"))
    return "".join(out)


def hull(x1, y1, x2, y2):
    """A dashed box around a sub-diagram: 'this part collapses to one block'."""
    return line(f"M{x1:g},{y1:g} L{x2:g},{y1:g} L{x2:g},{y2:g} L{x1:g},{y2:g} Z",
                EDGE, width=1.5, dash="7 6")


# ------------------------------------------------------------- sketches --
#
# A `step` or `bode` figure is generated from coefficients and cannot be
# annotated; these draw the cases where the annotation *is* the lesson — where
# t_p sits on the curve, what the angle to a pole means. Styled after
# `crates/app/src/plot.rs` so the two do not look like different courses.


def curve(points, colour=ACCENT, width=1.7):
    """A traced response: a polyline through sampled points, no arrowhead."""
    d = "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in points)
    return line(d, colour, width)


def guide(x1, y1, x2, y2, colour=FAINT, width=1):
    """A dashed construction line — a final value, a band edge, a projection.
    Dashed because it is not a signal and must not read as one."""
    return line(f"M{x1:g},{y1:g} L{x2:g},{y2:g}", colour, width, dash="4 5")


def band(x1, y1, x2, y2, colour=ACCENT, opacity=0.16):
    """A translucent region: a tolerance band, a half-plane. Washed rather than
    outlined, so it sits behind the curve without competing with it."""
    return (f'<rect x="{min(x1, x2):g}" y="{min(y1, y2):g}" '
            f'width="{abs(x2 - x1):g}" height="{abs(y2 - y1):g}" '
            f'fill="{colour}" fill-opacity="{opacity:g}"/>')


def cross(x, y, r=6, colour=INK, width=1.7):
    """A pole. The same × the plot renderer draws for a marked point."""
    return (line(f"M{x - r:g},{y - r:g} L{x + r:g},{y + r:g}", colour, width)
            + line(f"M{x - r:g},{y + r:g} L{x + r:g},{y - r:g}", colour, width))


def ring(cx, cy, r, colour=EDGE, width=1.5, dash=None):
    """An outlined circle: the unit circle, or a constant-ω₀ locus."""
    extra = f' stroke-dasharray="{dash}"' if dash else ""
    return (f'<circle cx="{cx:g}" cy="{cy:g}" r="{r:g}" fill="none" '
            f'stroke="{colour}" stroke-width="{width}"{extra}/>')


def arc(cx, cy, r, a0, a1, colour=FAINT, width=1.2, dash=None):
    """An arc from angle `a0` to `a1`, both in degrees measured the way a
    reader does — anticlockwise from the positive x axis. Used for the angle
    that *is* the damping ratio, so it is drawn, not merely labelled."""
    import math
    p0 = (cx + r * math.cos(math.radians(a0)), cy - r * math.sin(math.radians(a0)))
    p1 = (cx + r * math.cos(math.radians(a1)), cy - r * math.sin(math.radians(a1)))
    large = 1 if abs(a1 - a0) > 180 else 0
    sweep = 0 if a1 > a0 else 1          # SVG y grows downward
    d = (f"M{p0[0]:.1f},{p0[1]:.1f} A{r:g},{r:g} 0 {large} {sweep} "
         f"{p1[0]:.1f},{p1[1]:.1f}")
    extra = f' stroke-dasharray="{dash}"' if dash else ""
    return f'<path d="{d}" fill="none" stroke="{colour}" stroke-width="{width}"{extra}/>'


def sketch_axes(x0, y0, x1, y1, x_label=None, y_label=None, sub=""):
    """A pair of axes crossing at the origin (x0, y0), arrowed at (x1, y1).

    `x_label`/`y_label` are placed at the tips. Pass an origin that is not a
    corner and the result is an s-plane; pass one that is, and it is the usual
    time/amplitude frame."""
    out = [arrow(x0, y0, x1, y0, EDGE), arrow(x0, y0, x0, y1, EDGE)]
    if x_label:
        out.append(label(x1 - 4, y0 + 22, x_label, sub, size=16,
                         fill=LINE, anchor="end"))
    if y_label:
        out.append(label(x0 + 10, y1 + 14, y_label, size=16, fill=LINE))
    return "".join(out)


def demo():
    """Every element once, so a style change can be looked at."""
    y = 96
    return svg(640, 250,
               label(20, y - 14, "x", "d"),
               arrow(20, y, 100, y),
               junction(130, y),
               arrow(146, y, 202, y),
               block(266, y, "H", "ct"),
               line(f"M331,{y} L470,{y}"),
               head(470, y, "right"),
               dot(404, y),
               elbow([(404, y), (404, 186), (331, 186)], ACCENT, "left"),
               block(266, 186, "H", "F", h=50),
               elbow([(207, 186), (130, 186), (130, y + SUM / 2)], ACCENT, "up"),
               hull(104, 40, 480, 214),
               label(496, y + 8, "=", size=22, fill=INK),
               fraction_block(570, y, [("H", "ct")], [("1 + H", "ct"), ("H", "F")], w=130),
               caption(320, 240, "blockdiag demo — every element once"))


def sketch_demo():
    """The sketch half of the kit, on one sheet."""
    import math

    x0, y0, x1 = 40, 190, 340
    pts = [(x0 + t * 3, y0 - 110 * (1 - math.exp(-0.03 * t) * math.cos(0.06 * t)))
           for t in range(0, 101)]
    return svg(640, 250,
               sketch_axes(x0, y0, x1, 34, "t", y_label="x"),
               guide(x0, y0 - 110, x1 - 20, y0 - 110),
               curve(pts),
               cross(pts[40][0], pts[40][1], r=5),
               label(x0 + 6, y0 - 124, "final value", size=14, fill=FAINT),
               # s-plane half: axes crossing away from the corner.
               sketch_axes(500, 150, 630, 40, "σ", y_label="jω"),
               arrow(500, 150, 400, 150, EDGE),
               ring(500, 150, 70, dash="4 5"),
               arc(500, 150, 40, 180, 143),
               line("M500,150 L444,108", EDGE),
               cross(444, 108),
               cross(444, 192),
               label(462, 138, "φ", size=15, fill=INK),
               caption(320, 240, "blockdiag sketch demo — axes, curve, guide, "
                                 "cross, ring, arc"))


if __name__ == "__main__":
    import sys, os, xml.etree.ElementTree as ET

    if "--demo" not in sys.argv:
        print(__doc__.strip().splitlines()[0])
        print("usage: tools/blockdiag.py --demo [outdir]")
        raise SystemExit(0)
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = args[0] if args else "target/blockdiag"
    os.makedirs(out, exist_ok=True)
    for name, draw in (("demo", demo), ("sketch-demo", sketch_demo)):
        src = draw()
        ET.fromstring(src)
        path = os.path.join(out, f"{name}.svg")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(src)
        print(path)
