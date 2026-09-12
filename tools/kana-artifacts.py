#!/usr/bin/env python3
"""Restore historical research paths after clearing target (no data is copied)."""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ARCHIVE = ROOT / "kana-artifacts"
NAMES = ("kana-training", "kana-diagnostics", "kana-review")


def restore_links():
    for name in NAMES:
        if not (ARCHIVE / name).is_dir():
            raise SystemExit(f"Missing preserved artifacts: {ARCHIVE / name}")
        link = ROOT / "target" / name
        if (link.exists() or link.is_symlink()) and link.resolve() != ARCHIVE / name:
            raise SystemExit(f"Refusing to replace existing path: {link}")
    ROOT.joinpath("target").mkdir(exist_ok=True)
    for name in NAMES:
        link = ROOT / "target" / name
        if not link.is_symlink():
            link.symlink_to(
                Path("..") / "kana-artifacts" / name, target_is_directory=True
            )
    # Archived trainers enforce a target-only research root. Their isolated
    # replay trees use ARCHIVE as that root, with a link to rebuilt executables.
    release = ARCHIVE / "release"
    expected = ROOT / "target/release"
    if release.exists() or release.is_symlink():
        if release.resolve() != expected.resolve():
            raise SystemExit(f"Refusing to replace existing path: {release}")
    else:
        release.symlink_to(Path("..") / "target/release", target_is_directory=True)
    print(f"Research artifacts: {ARCHIVE}")
    print("Historical target/kana-* links restored; build outputs remain disposable.")


if __name__ == "__main__":
    restore_links()
