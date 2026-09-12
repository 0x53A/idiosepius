#!/usr/bin/env python3
"""Check exact training reproducibility, including arrays inside timestamped NPZs.

The two run directories remain local. Only hashes and aggregate evidence are
written to the first run's reproduction.json; no ETL image data is exported.
"""

import argparse
import hashlib
import json
import zipfile
from pathlib import Path


def checksum(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def array_hashes(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(set(names)) != len(names) or not all(n.endswith(".npy") for n in names):
            raise ValueError("ambiguous or unexpected prepared array members")
        result = {}
        for name in sorted(names):
            with archive.open(name) as stream:
                result[name] = hashlib.file_digest(stream, "sha256").hexdigest()
        return result


def verify(first, second):
    reports = [json.loads((run / "report.json").read_text()) for run in (first, second)]
    residual = reports[0]["config"].get("family") == "residual"
    required = (
        *([] if residual else ["model.bin"]),
        "model.json",
        "split.json",
        "history.json",
        "synthetic-golden.json",
        "config.json",
        "training-script.py",
        "raster-source.rs",
    )
    if reports[0]["config"].get("keep_checkpoints"):
        required += tuple(
            f"epoch-{epoch:03}.pt"
            for epoch in range(1, reports[0]["config"]["epochs"] + 1)
        )
    optional = (
        "proxy-preparation.json",
        "kana_etl7.py",
        "check-kana-etl.py",
        "initialization.json",
        "check-kana-preprocessing.py",
    )
    hashes = {}
    for name in (*required, *optional):
        a, b = first / name, second / name
        if name in optional and not a.exists() and not b.exists():
            continue
        hashes[name] = checksum(a)
        if hashes[name] != checksum(b):
            raise ValueError(f"reproduction differs: {name}")
    arrays = array_hashes(first / "prepared.npz")
    if arrays != array_hashes(second / "prepared.npz"):
        raise ValueError("prepared arrays differ")
    for report in reports:
        for field, filename in (
            ("binary_model_sha256", "model.bin"),
            ("model_sha256", "model.json"),
            ("split_sha256", "split.json"),
        ):
            if residual and filename == "model.bin":
                if report[field] is not None:
                    raise ValueError("Residual research model claims a portable binary")
                continue
            if report[field] != hashes[filename]:
                raise ValueError(f"report does not describe {filename}")
        for field, filename in (
            ("script_sha256", "training-script.py"),
            ("raster_source_sha256", "raster-source.rs"),
        ):
            if report["config"][field] != hashes[filename]:
                raise ValueError(f"config does not describe {filename}")
    for field in (
        "best_epoch",
        "results",
        "proxy_results",
        "sources",
        "checkpoint_metric",
        "config",
    ):
        if reports[0].get(field) != reports[1].get(field):
            raise ValueError(f"reported {field} differs")
    reused = any((run / "preparation-reuse.json").exists() for run in (first, second))
    return {
        "passed": True,
        "first_run": str(first),
        "second_run": str(second),
        "file_sha256": hashes,
        "prepared_array_sha256": arrays,
        "independent_preprocessing": not reused,
        "note": "Byte-identical weights, history, split, configuration, fixtures, and array contents on this workstation. NPZ ZIP timestamps are ignored. "
        + (
            "Prepared arrays were reused; preprocessing was not independently repeated here."
            if reused
            else "Both runs repeated raw ETL preprocessing."
        ),
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("first", type=Path)
    p.add_argument("second", type=Path)
    a = p.parse_args()
    result = verify(a.first, a.second)
    encoded = json.dumps(result, indent=2) + "\n"
    (a.first / "reproduction.json").write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
