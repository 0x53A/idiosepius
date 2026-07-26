#!/usr/bin/env python3
"""Validate the content packs against what crates/app/src/math.rs can render.

    tools/check-packs.py                       # every module under content/
    tools/check-packs.py content/maths-2       # one module
    tools/check-packs.py content/maths-2/*.json

`SUPPORTED` below is the renderer's command set. **A command added to
`crates/app/src/math.rs` has to be added here too**, or this keeps rejecting
content that would in fact display correctly. That coupling is why the script
lives in the application repository rather than beside the packs.
"""
import json, re, sys, glob, os, math
import xml.etree.ElementTree as ET

SUPPORTED = set("""frac dfrac tfrac sqrt underbrace left right begin end
text mathrm mathbf mathit mathsf mathcal mathbb operatorname
dot ddot hat widehat bar overline vec tilde widetilde
sum prod int iint iiint oint quad qquad hline
alpha beta gamma delta epsilon varepsilon zeta eta theta vartheta iota kappa
lambda mu nu xi pi rho sigma tau upsilon phi varphi chi psi omega
Gamma Delta Theta Lambda Xi Pi Sigma Upsilon Phi Psi Omega
le leq ge geq ne neq approx equiv sim propto ll gg in notin subset
to rightarrow longrightarrow leftarrow Rightarrow Leftrightarrow mapsto
cdot times div pm mp ast circ cup cap
infty partial nabla angle degree deg ldots dots cdots vdots prime
forall exists emptyset Re Im jmath imath
sin cos tan cot sec csc arctan arcsin arccos sinh cosh tanh log ln lg exp
lim max min arg det dim gcd sup inf""".split())

CMD = re.compile(r'\\([a-zA-Z]+)')
# `\\` is a row break, not the start of a command: without dropping it first,
# `\begin{pmatrix}x-1\\y-1\end{pmatrix}` is reported as an unknown `\y`.
ROW_BREAK = re.compile(r'\\\\')

def walk(node, path, out):
    if isinstance(node, str):
        out.append((path, node))
    elif isinstance(node, dict):
        for k, v in node.items():
            # SVG text is markup, not authored prose: dollar signs and
            # backslashes inside it do not belong to the math renderer.
            if node.get('kind') == 'svg' and k == 'src':
                continue
            walk(v, f"{path}.{k}", out)
    elif isinstance(node, list):
        for i, v in enumerate(node):
            walk(v, f"{path}[{i}]", out)


def walk_figures(node, path):
    """Yield every figure block, wherever it occurs in authored content."""
    if isinstance(node, dict):
        for key, value in node.items():
            here = f"{path}.{key}"
            if key == 'figure':
                yield here, value
            else:
                yield from walk_figures(value, here)
    elif isinstance(node, list):
        for i, value in enumerate(node):
            yield from walk_figures(value, f"{path}[{i}]")

def packs(args):
    """Every pack named, expanding directories. No arguments: all modules."""
    if not args:
        repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        return sorted(glob.glob(os.path.join(repo, 'content', '*', '*.json')))
    out = []
    for arg in args:
        out.extend(sorted(glob.glob(os.path.join(arg, '*.json')))
                   if os.path.isdir(arg) else [arg])
    return out


def validate_figure(figure, where):
    """Return authoring problems for one inline figure specification."""
    out = []
    if not isinstance(figure, dict):
        return [(where, 'FIGURE', 'figure must be an object')]
    kind = figure.get('kind')
    if kind not in {'bode', 'nyquist', 'step', 'svg'}:
        return [(where, 'FIGURE', f'unknown figure kind {kind!r}')]

    if kind == 'svg':
        src = figure.get('src')
        if not isinstance(src, str) or not src.strip():
            return [(where, 'FIGURE', 'SVG src must be a non-empty string')]
        try:
            root = ET.fromstring(src)
            if root.tag.rsplit('}', 1)[-1] != 'svg':
                out.append((where, 'FIGURE', 'SVG src root element is not <svg>'))
            for element in root.iter():
                for attribute, value in element.attrib.items():
                    if (attribute.rsplit('}', 1)[-1] == 'href'
                            and not value.startswith(('data:', '#'))):
                        out.append((where, 'FIGURE',
                                    f'SVG external reference is not self-contained: {value!r}'))
        except ET.ParseError as error:
            out.append((where, 'FIGURE', f'invalid SVG XML: {error}'))
        return out

    arrays = {}
    for name in ('num', 'den'):
        values = figure.get(name)
        if not isinstance(values, list) or not values:
            out.append((where, 'FIGURE', f'{name} must be a non-empty array'))
            continue
        if any(isinstance(v, bool) or not isinstance(v, (int, float))
               or not math.isfinite(v) for v in values):
            out.append((where, 'FIGURE', f'{name} coefficients must be finite numbers'))
            continue
        arrays[name] = values

    if 'den' in arrays and not any(abs(v) > 1e-12 for v in arrays['den']):
        out.append((where, 'FIGURE', 'denominator must not be identically zero'))

    if kind == 'bode' and 'phase' in figure and not isinstance(figure['phase'], bool):
        out.append((where, 'FIGURE', 'phase must be true or false'))

    if kind == 'step':
        times = figure.get('t')
        if (not isinstance(times, list) or len(times) != 2
                or any(isinstance(v, bool) or not isinstance(v, (int, float))
                       or not math.isfinite(v) for v in times)
                or times[0] < 0 or times[1] <= times[0]):
            out.append((where, 'FIGURE',
                        't must be [start, end] with 0 <= start < end'))
        if 'num' in arrays and 'den' in arrays:
            degree = lambda a: len(a) - next(
                (i for i, value in enumerate(a) if abs(value) > 1e-12),
                len(a) - 1) - 1
            if degree(arrays['num']) > degree(arrays['den']):
                out.append((where, 'FIGURE',
                            'step response requires a proper transfer function'))
    return out


files = packs(sys.argv[1:])
if not files:
    print('no packs found', file=sys.stderr)
    sys.exit(1)

problems = []
for f in files:
    doc = json.load(open(f))
    base = os.path.basename(f)
    for where, figure in walk_figures(doc, ''):
        for path, code, message in validate_figure(figure, where):
            problems.append((base, path, code, message))
    strings = []
    walk(doc, '', strings)
    # A formula fact's label is maths without the fences, so check it as such.
    for fa in doc.get('facts', []):
        if fa.get('kind') == 'formula':
            strings.append((f".formula[{fa['uid']}]", '$' + fa['label'] + '$'))
    # A lesson's display equation is maths without the fences, for the same
    # reason: it is always set as maths.
    for le in doc.get('lessons', []):
        for i, block in enumerate(le.get('body', [])):
            if isinstance(block, dict) and 'math' in block:
                strings.append((f".lesson[{le['uid']}].body[{i}]",
                                '$' + block['math'] + '$'))
    for path, s in strings:
        if s.count('$') % 2:
            problems.append((base, path, 'ODD-DOLLAR', s[:110]))
        for span in re.findall(r'\$([^$]*)\$', s):
            for c in CMD.findall(ROW_BREAK.sub(' ', span)):
                if c not in SUPPORTED:
                    problems.append((base, path, f'UNKNOWN \\{c}', span[:110]))

for p in problems:
    print(' | '.join(p))
print(f"\n{len(problems)} problem(s) in {len(files)} pack(s)")
# Exit non-zero when something is wrong, so this can gate a commit or a script
# rather than only being read by eye.
sys.exit(1 if problems else 0)
