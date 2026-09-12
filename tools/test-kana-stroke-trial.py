#!/usr/bin/env python3
"""Synthetic end-to-end checks of the remote trial, including replay and Rust export."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
import torch

spec = importlib.util.spec_from_file_location("trainer", Path(__file__).with_name("train-kana-vision.py"))
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)
ROOT = trainer.ROOT


def main():
    torch.set_num_threads(1)
    torch.manual_seed(20260912)
    with tempfile.TemporaryDirectory(prefix="stroke-trial-smoke-", dir=ROOT / "target") as temp:
        root = Path(temp)
        prepared = root / "prepared"
        prepared.mkdir()
        labels = np.array(list(range(46)) + list(range(46, 92)) + list(range(46)))
        records = {split: [dict(label=trainer.LABELS[int(label)], dataset="etl4" if i < 46 else "etl5" if i < 92 else "etl7",
                        split=split, group=split + str(i)) for i, label in enumerate(labels)] for split in ["train", "validation"]}
        trainer.write(prepared / "records.json", records)
        rng = np.random.default_rng(42)
        arrays = {f"{split}_{view}": rng.random((138, 1, 64, 64), dtype=np.float32)
                  for split in ["train", "validation"] for view in ["original", "clean"]}
        arrays.update(train_labels=labels, validation_labels=labels,
            synthetic_images=rng.random((92, 1, 64, 64), dtype=np.float32), synthetic_labels=np.arange(92))
        np.savez_compressed(prepared / "trial.npz", **arrays)
        trainer.write(prepared / "preparation.json", dict(trial_sha256=trainer.digest((prepared / "trial.npz").read_bytes()),
            records_sha256=trainer.digest((prepared / "records.json").read_bytes()), synthetic_ids=["embedded:synthetic-fixture"]))
        protocol = json.loads((ROOT / "tools/fixtures/kana-beginner/trial-v1.json").read_text())
        protocol.update(version="synthetic-smoke-only", epochs=2)
        trainer.write(root / "protocol.json", protocol)
        for family in ["plain", "residual"]:
            source = root / protocol["candidates"][family]["initialization"]
            source.mkdir()
            config = dict(family=family, depth=3, width=1 if family == "plain" else 2, rasterizer="kana-bitmap-64-v1")
            trainer.write(source / "config.json", config)
            torch.save(trainer.make_net(config).state_dict(), source / "best.pt")
            for repeat in ["first", "repeat"]:
                run = root / f"{family}-{repeat}"
                subprocess.run([sys.executable, str(ROOT / "tools/train-kana-stroke-trial.py"), "--prepared", str(prepared),
                    "--initialize-from", str(source), "--protocol", str(root / "protocol.json"), "--candidate", family,
                    "--threads", "1", "--run", str(run)], check=True, stdout=subprocess.DEVNULL)
            first, repeat = root / f"{family}-first", root / f"{family}-repeat"
            for name in ["model.bin", "best.pt", "epoch-001.pt", "epoch-002.pt", "history.json", "config.json", "report.json"]:
                assert (first / name).read_bytes() == (repeat / name).read_bytes(), (family, name)
            report = json.loads((first / "report.json").read_text())
            history = json.loads((first / "history.json").read_text())
            assert report["best_epoch"] == min(history, key=lambda r: tuple(r["checkpoint_key"]))["epoch"]
            assert set(report["validation"]) == {f"{v}:{s}" for v in ["original", "clean"] for s in ["etl4", "etl5", "etl7"]}
            golden = json.loads((first / "synthetic-golden.json").read_text())
            actual = json.loads(subprocess.check_output([str(ROOT / "target/release/kana-rasterize"), "--logits", "--model", str(first / "model.bin")], input=json.dumps(golden["input"]).encode()))
            delta = float(np.max(np.abs(np.array(actual) - np.array(golden["logits"]))))
            assert delta < .0002, delta
            print(f"{family}: exact two-epoch replay, selection, and Rust export passed; logit delta {delta:.3g}", flush=True)


if __name__ == "__main__":
    main()
