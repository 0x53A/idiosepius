"""Experimental stroke-level dakuten/handakuten candidates, independent of labels.

This is a research adapter, not deployed recognition or calibrated acceptance.
Pen order/direction do not matter. Candidate marks must be small and upper-right;
the caller must recognize the remaining body and validate the composition.
"""
import math
from itertools import combinations

VERSION = "kana-marks-v2"


def candidates(strokes):
    ink = [[(float(p["x"]), float(p["y"])) if isinstance(p, dict) else tuple(p) for p in s] for s in strokes]
    if not ink or any(not s for s in ink):
        return []
    points = [p for s in ink for p in s]
    if not all(math.isfinite(v) for p in points for v in p):
        return []
    x0, y0 = (min(p[k] for p in points) for k in (0, 1))
    glyph_width = max(p[0] for p in points) - x0
    glyph_height = max(p[1] for p in points) - y0
    side = max(glyph_width, glyph_height)
    if side <= 0:
        return []
    lines, circles = [], []
    for i, stroke in enumerate(ink):
        s = [((x-x0)/side, (y-y0)/side) for x,y in stroke]
        width = max(x for x,y in s)-min(x for x,y in s)
        height = max(y for x,y in s)-min(y for x,y in s)
        cx = (max(x for x,y in s)+min(x for x,y in s))/2
        cy = (max(y for x,y in s)+min(y for x,y in s))/2
        # Position belongs to each axis's actual extent. Dividing x by the
        # longest side incorrectly excluded upper-right marks on narrow glyphs.
        if cx * side < .60 * glyph_width or cy * side > .35 * glyph_height:
            continue
        length = sum(math.dist(a,b) for a,b in zip(s,s[1:]))
        chord = math.dist(s[0],s[-1])
        if max(width,height) <= .30 and min(width,height) > .025 and .5 < width/height < 2 and chord < .35*max(width,height) and length > 2*max(width,height):
            circles.append(dict(mark="handakuten", stroke_indices=[i]))
        dx,dy = s[-1][0]-s[0][0],s[-1][1]-s[0][1]
        if max(width,height) <= .22 and .025 <= length <= .30 and chord >= .75*length and dx*dy > 0 and abs(dx) >= .15*chord and abs(dy) >= .25*chord:
            lines.append(dict(index=i,cx=cx,cy=cy,dx=dx/chord,dy=dy/chord,length=length))
    pairs = []
    for a,b in combinations(lines,2):
        if (.025 <= abs(a["cx"]-b["cx"]) <= .20 and abs(a["cy"]-b["cy"]) <= .14
                and abs(a["dx"]*b["dx"]+a["dy"]*b["dy"]) >= .7
                and max(a["length"],b["length"])/min(a["length"],b["length"]) <= 2.5):
            pairs.append(dict(mark="dakuten",stroke_indices=sorted([a["index"],b["index"]])))
    return [c for c in circles+pairs if len(c["stroke_indices"]) < len(ink)]


def self_test():
    body = [[dict(x=10,y=10),dict(x=20,y=90),dict(x=80,y=60)]]
    marks = [[dict(x=82,y=2),dict(x=87,y=8)],[dict(x=91,y=0),dict(x=96,y=6)]]
    assert not candidates(body)
    assert candidates(body+marks) == [dict(mark="dakuten",stroke_indices=[1,2])]
    reversed_ink = [list(reversed(s)) for s in reversed(body+marks)]
    assert candidates(reversed_ink) == [dict(mark="dakuten",stroke_indices=[0,1])]
    transformed = [[dict(x=p["x"]*3+17,y=p["y"]*3-9) for p in s] for s in body+marks]
    assert candidates(transformed) == candidates(body+marks)
    narrow = [[dict(x=10,y=0),dict(x=10,y=120)],
              [dict(x=40,y=5),dict(x=47,y=12)],
              [dict(x=52,y=2),dict(x=59,y=9)]]
    assert candidates(narrow) == [dict(mark="dakuten",stroke_indices=[1,2])]
    ring = [[dict(x=91+4*math.cos(i*math.tau/24),y=5+4*math.sin(i*math.tau/24)) for i in range(25)]]
    assert candidates(body+ring) == [dict(mark="handakuten",stroke_indices=[1])]
    assert not candidates([]) and not candidates([[dict(x=float("nan"),y=0)]])
    print("Mark candidate tests passed: region, scale/translation, order/direction, circle, invalid coordinates")


if __name__ == "__main__":
    self_test()
