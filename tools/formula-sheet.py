#!/usr/bin/env python3
"""Render a module's formula facts as a printable formula sheet.

The sheet is *generated*, never hand-written: the formulas pack is the one
source of truth, so the paper you revise from and the facts the app cites in a
derivation cannot drift apart. Adding a formula to the pack adds it to the
sheet; there is no second place to remember.

    tools/formula-sheet.py cs-00-formulas.json
    tools/formula-sheet.py ma-00-formulas.json --terse -o out.tex

Nothing here knows about any particular module. Sections are the distinct
`source` values of the formula facts, in the order they first appear in the
pack — so section order is the authoring order, with no list to keep in step.
"""

import argparse
import json
import os
import re
import sys

# The pack targets the app's own renderer, which is not LaTeX. This is the
# only place the two disagree: the app draws a degree sign, LaTeX wants a
# superscript circle.
MATH_FIXUPS = [("°", r"^\circ")]

# Order matters — the backslash has to go first or it would escape the
# escapes.
TEXT_ESCAPES = [
    ("\\", r"\textbackslash{}"),
    ("&", r"\&"),
    ("%", r"\%"),
    ("#", r"\#"),
    ("_", r"\_"),
    ("{", r"\{"),
    ("}", r"\}"),
    ("~", r"\textasciitilde{}"),
    ("^", r"\textasciicircum{}"),
    ("°", r"\textdegree{}"),
    ("—", "---"),
    ("–", "--"),
    ("’", "'"),
    ("“", "``"),
    ("”", "''"),
    ("…", r"\ldots{}"),
]


def math(src):
    for a, b in MATH_FIXUPS:
        src = src.replace(a, b)
    return src


def text(src):
    for a, b in TEXT_ESCAPES:
        src = src.replace(a, b)
    return src


def prose(src):
    """Prose with `$...$` spans: escape the words, pass the maths through."""
    out = []
    for i, part in enumerate(re.split(r"\$", src)):
        out.append(f"${math(part)}$" if i % 2 else text(part))
    return "".join(out)


PREAMBLE = r"""\documentclass[10pt,a4paper]{article}
\usepackage{amsmath,amssymb,textcomp,xcolor,geometry,multicol,titlesec}
\usepackage{fancyhdr,adjustbox,needspace}
\geometry{margin=13mm,top=16mm,bottom=14mm}
\setlength{\columnsep}{7mm}
\setlength{\parindent}{0pt}
\definecolor{accent}{HTML}{0E6E62}
\definecolor{hair}{HTML}{9AA8AC}
\pagestyle{fancy}\fancyhf{}
\renewcommand{\headrulewidth}{0.4pt}
\fancyhead[L]{\footnotesize\textsc{__DECK__ \ ---\ formula sheet}}
\fancyhead[R]{\footnotesize\thepage}
\titleformat{\section}{\normalfont\bfseries\color{accent}\scshape}{}{0pt}{}
\titlespacing*{\section}{0pt}{2.4ex}{0.4ex}
%% One entry: its name, the formula on a line of its own, then the gloss.
%% The box shrinks a formula that would otherwise run out of the column,
%% which is the whole reason a two-column sheet is workable at all.
\newcommand{\entryname}[1]{\textbf{\footnotesize #1}\par}
\newcommand{\formula}[1]{%
  {\vspace{0.4ex}\centering\adjustbox{max width=\linewidth}{$\displaystyle #1$}\par}}
\newcommand{\gloss}[1]{{\footnotesize\color{black!70}#1\par}}
%% A name stranded at the foot of a column with its formula in the next one
%% is worse than a short column, so claim the room for both up front.
\newenvironment{entry}{\needspace{4\baselineskip}\vspace{1.1ex}\par\raggedright}{\par}
\begin{document}
\begin{center}
  {\large\bfseries __DECK__}\quad\textcolor{hair}{\rule[0.4ex]{7mm}{0.5pt}}\quad
  {\normalsize\textsc{formula sheet}}\\[0.5ex]
  {\footnotesize\color{black!60}__COUNT__ formulas, generated from
   \texttt{cs-00-formulas.json}. Conventions are this course's:
   5\,\% settling band, $\zeta \approx 0.01\varphi_m$.}
\end{center}
\vspace{0.5ex}
\begin{multicols}{2}
"""

POSTAMBLE = r"""\end{multicols}
\end{document}
"""


def render(facts, terse):
    # Sections in first-appearance order. A dict preserves insertion order, so
    # the pack's own layout is the sheet's layout and there is no list of
    # section names to keep in step with the content.
    groups = {}
    for f in facts:
        groups.setdefault(f.get("source") or "Other", []).append(f)

    body = []
    for name, group in groups.items():
        body.append(f"\\section*{{{text(name)}}}")
        body.append(r"\vspace{-0.7ex}\textcolor{hair}{\hrule height 0.4pt}")
        for f in group:
            body.append(r"\begin{entry}")
            if f.get("title"):
                body.append(f"\\entryname{{{text(f['title'])}}}")
            body.append(f"\\formula{{{math(f['label'])}}}")
            if not terse and f.get("body"):
                body.append(f"\\gloss{{{prose(f['body'])}}}")
            body.append(r"\end{entry}")
    return "\n".join(body)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("pack", help="a pack holding the module's formula facts")
    ap.add_argument("--terse", action="store_true",
                    help="formulas only, without the explanatory glosses")
    ap.add_argument("-o", "--out", default="-")
    args = ap.parse_args()

    doc = json.load(open(args.pack))
    facts = [f for f in doc.get("facts", []) if f.get("kind") == "formula"]
    if not facts:
        sys.exit(f"{args.pack}: no formula facts")
    missing = [f["uid"] for f in facts if not f.get("label")]
    if missing:
        sys.exit(f"{args.pack}: formula facts without a label: {missing}")

    head = (PREAMBLE
            .replace("__DECK__", text(doc["deck"]["title"]))
            .replace("__COUNT__", str(len(facts))))
    tex = head + render(facts, args.terse) + POSTAMBLE

    if args.out == "-":
        sys.stdout.write(tex)
    else:
        with open(args.out, "w") as fh:
            fh.write(tex)
        print(f"{args.out}: {len(facts)} formulas", file=sys.stderr)


if __name__ == "__main__":
    main()
