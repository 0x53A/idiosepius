#!/usr/bin/env python3
"""Render a module's formula facts as a printable formula sheet.

The sheet is *generated*, never hand-written: the formulas pack is the one
source of truth, so the paper you revise from and the facts the app cites in a
derivation cannot drift apart. Adding a formula to the pack adds it to the
sheet; there is no second place to remember.

    tools/formula-sheet.py cs-00-formulas.json
    tools/formula-sheet.py ma-00-formulas.json --terse -o out.tex
    tools/formula-sheet.py cs-00-formulas.json --compact -o out.tex

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


def content_text(content):
    """Text from a shorthand string or ordered text/figure block array."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n\n".join(block for block in content if isinstance(block, str))
    return ""


PREAMBLE = r"""\documentclass[10pt,a4paper]{article}
\usepackage{amsmath,amssymb,textcomp,xcolor,geometry,multicol,titlesec}
\usepackage{adjustbox,needspace}
\geometry{margin=13mm,top=13mm,bottom=13mm}
\setlength{\columnsep}{7mm}
\setlength{\parindent}{0pt}
\definecolor{accent}{HTML}{0E6E62}
\definecolor{hair}{HTML}{9AA8AC}
%% No running head, no page numbers, no title: the sheet is carried into an
%% exam, where every line that is not a formula is a line of paper wasted.
\pagestyle{empty}
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
\newenvironment{terseentry}{\needspace{4\baselineskip}\vspace{0.7ex}\par\raggedright}{\par}
\begin{document}
__NOTE__\begin{multicols}{2}
"""

COMPACT_PREAMBLE = r"""%% Standalone snapshot: edit this file directly as needed.
%% Re-running tools/build-sheet.sh --compact regenerates and overwrites it.
\documentclass[10pt,a4paper]{article}
\usepackage{amsmath,amssymb,textcomp,geometry,multicol,titlesec}
\usepackage{adjustbox,needspace}
\geometry{left=9mm,right=9mm,top=8mm,bottom=8mm}
\setlength{\columnsep}{5mm}
\setlength{\columnseprule}{0.25pt}
\setlength{\parindent}{0pt}
\setlength{\multicolsep}{0pt}
\pagestyle{empty}
\raggedcolumns
%% Black-and-white exam edition. Section headings are only one point larger
%% than the formula text; hierarchy comes from weight and the hairline.
\titleformat{\section}
  {\normalfont\bfseries\fontsize{10.0}{10.9}\selectfont}{}{0pt}{}
\titlespacing*{\section}{0pt}{1.35ex}{0.3ex}
\newcommand{\entryname}[1]{%
  {\bfseries\fontsize{8.2}{9.0}\selectfont #1}\par}
\newcommand{\formula}[1]{%
  {\vspace{0.15ex}\centering\fontsize{9.2}{10.0}\selectfont
   \adjustbox{max width=\linewidth}{$\displaystyle #1$}\par}}
\newcommand{\gloss}[1]{%
  {\vspace{0.2ex}\fontsize{8.0}{9.1}\selectfont #1\par}}
\newenvironment{entry}
  {\needspace{3\baselineskip}\vspace{0.65ex}\par\raggedright}{\par}
\newenvironment{terseentry}
  {\needspace{3\baselineskip}\vspace{0.65ex}\par\raggedright}{\par}
\begin{document}
__NOTE__\begin{multicols}{2}
"""

# The one thing on the sheet that is not a formula, so it is set to be read
# first: the course's own conventions, which is where a remembered textbook
# result costs marks. Supplied per module — see `--note`.
NOTE = r"""\begin{center}
  \setlength{\fboxrule}{0.5pt}\setlength{\fboxsep}{1.1ex}%
  \fcolorbox{accent}{accent!7}{\parbox{\dimexpr\linewidth-2.2ex-1pt\relax}{%
    \centering\small __TEXT__}}
\end{center}
\vspace{0.6ex}
"""

COMPACT_NOTE = r"""\begin{center}
  \setlength{\fboxrule}{0.45pt}\setlength{\fboxsep}{0.65ex}%
  \fbox{\parbox{\dimexpr\linewidth-1.3ex-0.9pt\relax}{%
    \centering\fontsize{8.4}{9.2}\selectfont __TEXT__}}
\end{center}
\vspace{0.25ex}
"""

POSTAMBLE = r"""\end{multicols}
\end{document}
"""


def sidecar(pack):
    """Optional `<pack>.sheet.json`: settings that are the sheet's, not the app's.

    A course's conventions and the headings it does not want printed belong to
    that course, but the app has no use for either — so they sit beside the
    pack rather than inside it, and nothing module-specific has to be known
    here or passed in by the caller.

        {"note": "Conventions: ...", "drop_headings": ["..."]}
    """
    path = re.sub(r"\.json$", "", pack) + ".sheet.json"
    if not os.path.exists(path):
        return {}
    with open(path) as fh:
        return json.load(fh)


def leading_sentences(src, count):
    """Keep compact prose useful without printing whole fact bodies."""
    parts = re.split(r"(?<=[.!?])\s+(?=[A-Z$])", src)
    return " ".join(parts[:count])


def render(facts, terse, drop_headings=(), section_order=(),
           heading_aliases=None, compact=False, gloss_sections=(),
           gloss_sentences=1):
    # Sections in first-appearance order. A dict preserves insertion order, so
    # the pack's own layout is the sheet's layout and there is no list of
    # section names to keep in step with the content.
    groups = {}
    for f in facts:
        groups.setdefault(f.get("source") or "Other", []).append(f)

    ordered_names = [name for name in section_order if name in groups]
    ordered_names.extend(name for name in groups if name not in ordered_names)
    heading_aliases = heading_aliases or {}

    body = []
    for name in ordered_names:
        group = groups[name]
        # A heading may be suppressed without disturbing the `source` it comes
        # from: the citation is content, the heading is only how this sheet
        # sets it. The formulas stay where the pack put them.
        if name not in drop_headings:
            heading = heading_aliases.get(name, name)
            body.append(f"\\section*{{{text(heading)}}}")
            if compact:
                body.append(r"\vspace{-0.55ex}\hrule height 0.35pt")
            else:
                body.append(
                    r"\vspace{-0.7ex}\textcolor{hair}{\hrule height 0.4pt}")
        for f in group:
            environment = "terseentry" if terse else "entry"
            body.append(f"\\begin{{{environment}}}")
            if f.get("title"):
                body.append(f"\\entryname{{{text(f['title'])}}}")
            body.append(f"\\formula{{{math(f['label'])}}}")
            gloss = content_text(f.get("body"))
            if compact and name in gloss_sections:
                gloss = leading_sentences(gloss, gloss_sentences)
            if (not terse or name in gloss_sections) and gloss:
                body.append(f"\\gloss{{{prose(gloss)}}}")
            body.append(f"\\end{{{environment}}}")
    return "\n".join(body)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("pack", help="a pack holding the module's formula facts")
    ap.add_argument("--terse", action="store_true",
                    help="formulas only, without the explanatory glosses")
    ap.add_argument("--compact", action="store_true",
                    help="compact black-and-white formulas-only exam edition")
    ap.add_argument("--note", default=None,
                    help="a line to set in a box at the top of the first page, "
                         "e.g. the conventions this course marks to; prose "
                         "with $...$ spans. Overrides the sidecar's `note`.")
    ap.add_argument("-o", "--out", default="-")
    args = ap.parse_args()

    doc = json.load(open(args.pack))
    facts = [f for f in doc.get("facts", []) if f.get("kind") == "formula"]
    if not facts:
        sys.exit(f"{args.pack}: no formula facts")
    missing = [f["uid"] for f in facts if not f.get("label")]
    if missing:
        sys.exit(f"{args.pack}: formula facts without a label: {missing}")

    settings = sidecar(args.pack)
    note = (args.note if args.note is not None else settings.get("note", "")).strip()
    preamble = COMPACT_PREAMBLE if args.compact else PREAMBLE
    note_template = COMPACT_NOTE if args.compact else NOTE
    head = preamble.replace(
        "__NOTE__", note_template.replace("__TEXT__", prose(note)) if note else "")
    tex = (head
           + render(
               facts,
               args.terse or args.compact,
               set(settings.get("drop_headings", [])),
               settings.get("section_order", []),
               settings.get("heading_aliases", {}),
               args.compact,
               set(settings.get("compact_gloss_sections", [])),
               settings.get("compact_gloss_sentences", 1))
           + POSTAMBLE)

    if args.out == "-":
        sys.stdout.write(tex)
    else:
        with open(args.out, "w") as fh:
            fh.write(tex)
        print(f"{args.out}: {len(facts)} formulas", file=sys.stderr)


if __name__ == "__main__":
    main()
