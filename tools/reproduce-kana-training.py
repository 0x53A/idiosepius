#!/usr/bin/env python3
"""Repeat a prepared-data training experiment from its archived source snapshot.

Run in the trainer's pinned Python environment. A small isolated source tree
avoids swapping files in the working checkout while other development continues.
"""

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("first", type=Path)
    p.add_argument("second", type=Path)
    a = p.parse_args()
    archive = ROOT / "kana-artifacts"
    if archive.exists():
        subprocess.run(
            [sys.executable, str(ROOT / "tools/kana-artifacts.py")], check=True
        )
    for path in [a.first, a.second]:
        if not any(
            path.resolve().is_relative_to((ROOT / area).resolve())
            for area in ("target", "kana-artifacts")
        ):
            raise ValueError(
                "Keep experiment artifacts under target/ or kana-artifacts/"
            )
    if a.second.exists():
        raise ValueError("Never overwrite an experiment")
    config = json.loads((a.first / "config.json").read_text())
    preparation = json.loads((a.first / "preparation-reuse.json").read_text())
    snapshot = a.second.with_name(a.second.name + "-sources")
    snapshot.mkdir()
    (snapshot / "tools").mkdir()
    (snapshot / "crates/core/src").mkdir(parents=True)
    # Preserve archived source bytes, including their original target-only
    # path checks, by rooting durable replays in the preserved artifact tree.
    replay_root = (
        archive
        if a.second.resolve().is_relative_to(archive.resolve())
        else ROOT / "target"
    )
    (snapshot / "target").symlink_to(replay_root, target_is_directory=True)
    (snapshot / "kana-artifacts").symlink_to(archive, target_is_directory=True)
    for name, target, key in [
        ("training-script.py", "tools/train-kana-vision.py", "script_sha256"),
        ("raster-source.rs", "crates/core/src/kana_vision.rs", "raster_source_sha256"),
        ("kana_etl7.py", "tools/kana_etl7.py", "etl7_reader_sha256"),
        ("check-kana-etl.py", "tools/check-kana-etl.py", None),
    ]:
        source = a.first / name
        if key and hashlib.sha256(source.read_bytes()).hexdigest() != config[key]:
            raise ValueError(f"Archived source mismatch: {name}")
        shutil.copyfile(source, snapshot / target)
    if config.get("clean_scans"):
        source = a.first / "check-kana-preprocessing.py"
        if (
            hashlib.sha256(source.read_bytes()).hexdigest()
            != config["scan_preprocessor_sha256"]
        ):
            raise ValueError("Archived scan preprocessor changed")
        shutil.copyfile(source, snapshot / "tools/check-kana-preprocessing.py")
    args = [
        sys.executable,
        str(snapshot / "tools/train-kana-vision.py"),
        "--run",
        str(a.second),
        "--prepared-from",
        preparation["source"],
    ]
    side = int(config["rasterizer"].split("-")[2])
    for flag, value in [
        ("side", side),
        ("depth", config["depth"]),
        ("width", config.get("width", 1)),
        ("epochs", config["epochs"]),
        ("threads", config["threads"]),
        ("seed", config["seed"]),
        *[
            (key.replace("_", "-"), config[key])
            for key in ("training_seed", "family", "checkpoint_selection")
            if key in config
        ],
        ("learning-rate", config.get("learning_rate", 0.002)),
        ("loss-scope", config.get("loss_scope", "known-script")),
    ]:
        if (
            flag in ["width", "learning-rate", "loss-scope"]
            and flag.replace("-", "_") not in config
        ):
            continue
        args.extend(["--" + flag, str(value)])
    for key in [
        "ink_proxy",
        "etl7",
        "source_balanced",
        "clean_scans",
        "validation_only",
        "keep_checkpoints",
        "channels_last",
    ]:
        if config.get(key):
            args.append("--" + key.replace("_", "-"))
    if (a.first / "initialization.json").exists():
        init = json.loads((a.first / "initialization.json").read_text())
        teacher = Path(init["source"]) / "best.pt"
        if (
            hashlib.sha256(teacher.read_bytes()).hexdigest()
            != init["checkpoint_sha256"]
        ):
            raise ValueError("Warm-start checkpoint changed")
        args.extend(["--initialize-from", init["source"]])
    subprocess.run(args, cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
