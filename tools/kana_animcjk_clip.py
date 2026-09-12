"""Derive visible centerline fragments from AnimCJK's clipped animation medians.

Some loop strokes share an animation trajectory extending far outside their own
filled outline. Those invisible extensions are not user pen strokes. We flatten
the original outlines, sample the median at <=2 font units, retain samples within
the nonzero fill, and keep separated runs separate. This is a geometry proxy,
not reconstructed human stroke order, pressure, or timing.
"""
import importlib.util
import math
from pathlib import Path
import re

spec = importlib.util.spec_from_file_location("cross_source", Path(__file__).with_name("check-kana-cross-source.py"))
cross_source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cross_source)
VERSION = "animcjk-outline-clipped-median-v1"


def contains(point, contours):
    x,y = point
    winding = 0
    for contour in contours:
        for a,b in zip(contour,contour[1:]+contour[:1]):
            side = (b[0]-a[0])*(y-a[1])-(x-a[0])*(b[1]-a[1])
            if abs(side)<1e-8 and min(a[0],b[0])-1e-8<=x<=max(a[0],b[0])+1e-8 and min(a[1],b[1])-1e-8<=y<=max(a[1],b[1])+1e-8:
                return True
            if a[1]<=y<b[1] and side>0:
                winding += 1
            elif b[1]<=y<a[1] and side<0:
                winding -= 1
    return winding != 0


def clipped_runs(median, contours, step=2.):
    points = []
    for a,b in zip(median,median[1:]):
        count = max(1,math.ceil(math.dist(a,b)/step))
        points.extend([(a[0]+(b[0]-a[0])*i/count,a[1]+(b[1]-a[1])*i/count) for i in range(count)])
    points.append(tuple(median[-1]))
    runs, current = [], []
    for point in points:
        if contains(point,contours):
            current.append(point)
        else:
            if len(current)>=2:
                runs.append(current)
            current=[]
    if len(current)>=2:
        runs.append(current)
    return runs


def strokes(record):
    result=[]
    for outline,median in zip(record["strokes"],record["medians"],strict=True):
        if "m" in outline or not outline.startswith("M"):
            raise ValueError("Expected pinned uppercase absolute subpaths")
        contours = [cross_source.svg_path_points(part) for part in re.findall(r"M[^M]*",outline)]
        for run in clipped_runs(median,contours):
            result.append([dict(x=x,y=900-y) for x,y in run])
    if not result:
        raise ValueError(f"Empty clipped character: {record['character']}")
    return result


def trajectory_groups(record):
    """Map identical animation trajectories to all their visible fragments.

    Several filled outlines may share a median. Removing just one such outline
    is not removal of the complete animation trajectory. Group identity is
    source evidence, not a claim about a human writer's pen lifts.
    """
    groups = {}
    fragment_offset = 0
    for index, (outline, median) in enumerate(zip(record["strokes"], record["medians"], strict=True)):
        key = tuple(tuple(point) for point in median)
        group = groups.setdefault(key, dict(outline_indices=[], fragment_indices=[]))
        group["outline_indices"].append(index)
        # Reuse the exact extraction path so flattened indexing cannot drift.
        try:
            fragments = strokes(dict(character=record["character"], strokes=[outline], medians=[median]))
        except ValueError as error:
            if not str(error).startswith("Empty clipped character:"):
                raise
            fragments = []
        group["fragment_indices"].extend(range(fragment_offset, fragment_offset + len(fragments)))
        fragment_offset += len(fragments)
    return list(groups.values())


def self_test():
    outer=[(0,0),(10,0),(10,10),(0,10)]
    hole=[(3,3),(3,7),(7,7),(7,3)]
    assert contains((1,1),[outer,hole]) and not contains((5,5),[outer,hole])
    assert not contains((-1,5),[outer]) and contains((0,5),[outer])
    runs=clipped_runs([(-10,5),(20,5)],[outer])
    assert len(runs)==1 and runs[0][0]==(0.,5.) and runs[0][-1]==(10.,5.)
    second=[(20,0),(30,0),(30,10),(20,10)]
    runs=clipped_runs([(-10,5),(40,5)],[outer,second])
    assert len(runs)==2 and max(x for x,y in runs[0])<=10 and min(x for x,y in runs[1])>=20
    record=dict(character="fixture",strokes=["M0,0L10,0L10,10L0,10Z"],medians=[[[-10,5],[20,5]]])
    assert strokes(record)[0][0]==dict(x=0.,y=895.)
    record=dict(character="split",strokes=["M0,0L10,0L10,10L0,10Z", "M20,0L30,0L30,10L20,10Z"],
                medians=[[[-10,5],[40,5]],[[-10,5],[40,5]]])
    assert trajectory_groups(record)==[dict(outline_indices=[0,1],fragment_indices=[0,1])]
    assert len(strokes(record))==2
    print("Clipped median tests passed: boundaries, nonzero holes, disjoint runs, screen orientation")


if __name__ == "__main__":
    self_test()
