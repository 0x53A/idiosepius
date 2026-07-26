#!/usr/bin/env python3
"""Format a content pack the way the packs are written by hand.

`json.dumps` would explode every option onto four lines and reorder nothing
usefully; the packs are read and edited by a person far more often than by a
program, so they keep a house style:

  * one key per line, two-space indent
  * an option or a topic is one line, because it is one thought
  * questions keep a fixed key order, so diffs are about content
  * non-ASCII stays as itself: the whole deck is full of ω and ζ

Idempotent, so it doubles as a check:

    tools/packfmt.py content/*.json          # rewrite in place
    tools/packfmt.py --check content/*.json  # non-zero if anything would change
"""

import json
import sys

QUESTION_KEYS = [
    "uid",
    "topic",
    "prompt",
    "kind",
    "answer",
    "options",
    "multi",
    "explanation",
    "explain",
    "difficulty",
    "source",
    "tags",
]

FACT_KEYS = ["uid", "kind", "label", "name", "title", "body", "source"]

LESSON_KEYS = ["uid", "topic", "ord", "title", "summary", "body", "practice", "source"]


def enc(value):
    """One value, on one line."""
    return json.dumps(value, ensure_ascii=False)


def one_line_obj(obj, keys=None):
    order = [k for k in (keys or obj.keys()) if k in obj]
    order += [k for k in obj if k not in order]
    inner = ", ".join(f"{enc(k)}: {enc(obj[k])}" for k in order)
    return "{ " + inner + " }"


def segments(items, indent):
    """A list of explanation segments: raw strings and fact references."""
    pad = " " * indent
    out = ["["]
    for i, seg in enumerate(items):
        tail = "" if i == len(items) - 1 else ","
        out.append(f"{pad}  {enc(seg)}{tail}")
    out.append(pad + "]")
    return "\n".join(out)


def explain(obj, indent):
    pad = " " * indent
    parts = []
    for key in ("short", "deep"):
        if obj.get(key):
            parts.append(f'{pad}  {enc(key)}: {segments(obj[key], indent + 2)}')
    return "{\n" + ",\n".join(parts) + f"\n{pad}}}"


def question(q, indent):
    pad = " " * indent
    order = [k for k in QUESTION_KEYS if k in q] + [
        k for k in q if k not in QUESTION_KEYS
    ]
    lines = []
    for key in order:
        value = q[key]
        if key == "options":
            opts = ",\n".join(
                f"{pad}    " + one_line_obj(o, ["text", "correct", "note"])
                for o in value
            )
            lines.append(f'{pad}  {enc(key)}: [\n{opts}\n{pad}  ]')
        elif key == "explain":
            lines.append(f'{pad}  {enc(key)}: {explain(value, indent + 2)}')
        elif key == "prompt" and isinstance(value, list):
            lines.append(f'{pad}  {enc(key)}: {segments(value, indent + 2)}')
        else:
            lines.append(f"{pad}  {enc(key)}: {enc(value)}")
    return "{\n" + ",\n".join(lines) + f"\n{pad}}}"


def fact(f, indent):
    pad = " " * indent
    order = [k for k in FACT_KEYS if k in f] + [k for k in f if k not in FACT_KEYS]
    lines = [
        f"{pad}  {enc(k)}: "
        + (segments(f[k], indent + 2) if k == "body" and isinstance(f[k], list) else enc(f[k]))
        for k in order
    ]
    return "{\n" + ",\n".join(lines) + f"\n{pad}}}"


def lesson(l, indent):
    """One lesson: a header block, then one body block per line."""
    pad = " " * indent
    order = [k for k in LESSON_KEYS if k in l] + [k for k in l if k not in LESSON_KEYS]
    lines = [
        f"{pad}  {enc(k)}: "
        + (segments(l[k], indent + 2) if k == "body" else enc(l[k]))
        for k in order
    ]
    return "{\n" + ",\n".join(lines) + f"\n{pad}}}"


def format_pack(pack):
    out = ["{"]
    out.append('  "deck": {')
    deck_keys = [k for k in ("slug", "title", "description", "exam_at") if k in pack["deck"]]
    out.append(",\n".join(f"    {enc(k)}: {enc(pack['deck'][k])}" for k in deck_keys))
    out.append("  },")

    if pack.get("topics"):
        out.append('  "topics": [')
        out.append(
            ",\n".join(
                "    " + one_line_obj(t, ["slug", "title", "ord"])
                for t in pack["topics"]
            )
        )
        out.append("  ],")

    if pack.get("facts"):
        out.append('  "facts": [')
        out.append(",\n".join("    " + fact(f, 4) for f in pack["facts"]))
        out.append("  ],")

    if pack.get("lessons"):
        out.append('  "lessons": [')
        out.append(",\n".join("    " + lesson(l, 4) for l in pack["lessons"]))
        out.append("  ],")

    out.append('  "questions": [')
    if pack.get("questions"):
        out.append(",\n".join("    " + question(q, 4) for q in pack["questions"]))
    out.append("  ]")
    out.append("}")
    return "\n".join(out) + "\n"


def main(argv):
    check = "--check" in argv
    paths = [a for a in argv if not a.startswith("--")]
    dirty = []

    for path in paths:
        with open(path, encoding="utf-8") as fh:
            original = fh.read()
        formatted = format_pack(json.loads(original))
        # The formatter must not change what the pack means.
        assert json.loads(formatted) == json.loads(original), f"{path}: not faithful"
        if formatted == original:
            continue
        dirty.append(path)
        if not check:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(formatted)

    if check and dirty:
        print("would reformat: " + ", ".join(dirty), file=sys.stderr)
        return 1
    if dirty and not check:
        print("formatted: " + ", ".join(dirty))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
