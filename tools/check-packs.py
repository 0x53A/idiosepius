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
import json, re, sys, glob, os

SUPPORTED = set("""frac dfrac tfrac sqrt left right begin end
text mathrm mathbf mathit mathsf mathcal mathbb operatorname
dot ddot hat widehat bar overline vec tilde widetilde
sum prod int iint iiint oint quad qquad
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
            walk(v, f"{path}.{k}", out)
    elif isinstance(node, list):
        for i, v in enumerate(node):
            walk(v, f"{path}[{i}]", out)

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


files = packs(sys.argv[1:])
if not files:
    print('no packs found', file=sys.stderr)
    sys.exit(1)

problems = []
for f in files:
    doc = json.load(open(f))
    base = os.path.basename(f)
    strings = []
    walk(doc, '', strings)
    # A formula fact's label is maths without the fences, so check it as such.
    for fa in doc.get('facts', []):
        if fa.get('kind') == 'formula':
            strings.append((f".formula[{fa['uid']}]", '$' + fa['label'] + '$'))
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
