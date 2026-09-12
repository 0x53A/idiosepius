#!/usr/bin/env python3
"""Benchmark the real kana models on synthetic ink; no course or ETL data needed."""
import argparse
import hashlib
import importlib.util
import json
import platform
import statistics
import time
from pathlib import Path

import torch

spec = importlib.util.spec_from_file_location("trainer", Path(__file__).with_name("train-kana-vision.py"))
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--steps", type=int, default=8)
    a = p.parse_args()
    if a.steps < 3 or a.output.exists():
        p.error("Use at least three steps and a new output path")
    results = []
    for family in ["plain", "residual"]:
        for threads in [1, 3, 6]:
            for channels_last in [False, True]:
                torch.set_num_threads(threads)
                torch.manual_seed(20260912)
                torch.use_deterministic_algorithms(True)
                config = dict(family=family, depth=3, width=2, rasterizer="kana-bitmap-64-v1", channels_last=channels_last)
                net = trainer.make_net(config)
                optimizer = torch.optim.AdamW(net.parameters(), lr=0.002)
                x = torch.rand(128, 1, 64, 64)
                y = torch.arange(128) % 92
                timings = []
                for step in range(a.steps + 3):
                    start = time.perf_counter()
                    optimizer.zero_grad()
                    logits = net(trainer.augment(x))
                    torch.nn.functional.cross_entropy(logits, y).backward()
                    optimizer.step()
                    elapsed = time.perf_counter() - start
                    if step >= 3:
                        timings.append(elapsed)
                row = dict(family=family, threads=threads, channels_last=channels_last,
                           median_seconds=statistics.median(timings), seconds=timings,
                           samples_per_second=128 / statistics.median(timings))
                results.append(row)
                print(json.dumps(row), flush=True)
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(dict(host=platform.node(), platform=platform.platform(),
        torch=torch.__version__, trainer_sha256=hashlib.sha256(Path(trainer.__file__).read_bytes()).hexdigest(),
        benchmark_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        batch_size=128, side=64, warmup_steps=3, results=results,
        note="Synthetic CPU training-step timing including augmentation and AdamW; excludes data preparation, validation, checkpoint IO and transfer. No GPU claim."), indent=2) + "\n")


if __name__ == "__main__":
    main()
