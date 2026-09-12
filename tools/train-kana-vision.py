#!/usr/bin/env python3
"""Train the experimental kana CNN from locally obtained ETL4/ETL5 C files.

Obtain data from AIST after accepting its terms; never redistribute raw records.
Run inside nix-shell after building kana-rasterize, using the pinned CPU environment
in JAPANESE.md. Splits are frozen before training; only validation selects epochs.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import random
import struct
import subprocess
import time
from pathlib import Path

import kana_etl7
import numpy as np
import torch
from torch import nn
from torch.nn import functional as F

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("etl", ROOT / "tools/check-kana-etl.py")
etl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(etl)
LABELS = etl.HIRAGANA + etl.KATAKANA
ARCH = "conv8x3-pool-conv16x3-pool-linear92-v1"
RASTER = "kana-bitmap-32-v1"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write(path, obj):
    etl.atomic_json(path, obj)


def prepare(
    data_root, run, seed, side, ink_proxy, include_etl7=False, clean_scans=False
):
    clean = None
    if clean_scans:
        clean_spec = importlib.util.spec_from_file_location(
            "clean_scans", ROOT / "tools/check-kana-preprocessing.py"
        )
        clean = importlib.util.module_from_spec(clean_spec)
        clean_spec.loader.exec_module(clean)
    records = []
    proxy_strokes = []
    payload = bytearray()
    sources = {}
    for dataset in ["etl4", "etl5"]:
        path = data_root / dataset.upper() / (dataset.upper() + "C")
        raw = path.read_bytes()
        if len(raw) != etl.EXPECTED_RECORDS[dataset] * etl.RECORD_BYTES:
            raise ValueError("Unexpected ETL size")
        expected_hashes = {
            "etl4": "3d7178bc6b6d73b5031dd3598d6bc6a402f6a7d8e4a742ec3718765968721185",
            "etl5": "7abcac03867e8d339d5848849dc196b93463357c8f0ecbcf78e976677147f61f",
        }
        if digest(raw) != expected_hashes[dataset]:
            raise ValueError(f"Unexpected {dataset} checksum")
        sources[dataset] = {
            "sha256": digest(raw),
            "records": len(raw) // etl.RECORD_BYTES,
        }
        for index in range(len(raw) // etl.RECORD_BYTES):
            record = raw[index * etl.RECORD_BYTES : (index + 1) * etl.RECORD_BYTES]
            label = etl.decode_label(record, dataset)
            if label not in LABELS:
                continue
            bits = int.from_bytes(record[:90], "big")
            fields = [
                (bits >> (720 - 36 * (i + 1))) & ((1 << 36) - 1) for i in range(20)
            ]
            # ETL4 has one sheet per writer. ETL5's writer-to-sheet map is not
            # documented: conservatively keep all identical metadata together.
            group = (
                f"{dataset}:{fields[1]}"
                if dataset == "etl4"
                else f"{dataset}:" + ",".join(map(str, fields[9:14]))
            )
            image = np.array(etl.grayscale(record), dtype=np.uint8)
            threshold = etl.otsu_threshold(image.tolist())
            border = np.concatenate(
                [image[0], image[-1], image[1:-1, 0], image[1:-1, -1]]
            )
            if np.median(border) > threshold:
                image = 15 - image
            # Retain grayscale, suppress scan noise below the Otsu foreground.
            cutoff = 15 - threshold if np.median(border) > threshold else threshold + 1
            image = np.where(image >= cutoff, image * 17, 0).astype(np.uint8)
            if clean is not None:
                gray, ink = clean.clean_image(etl.grayscale(record))
                image = np.array(gray, dtype=np.uint8)
            payload.extend(struct.pack("<HH", 72, 76))
            payload.extend(image.tobytes())
            if ink_proxy:
                proxy_strokes.append(
                    [
                        [{"x": x, "y": y} for x, y in stroke]
                        for stroke in (
                            etl.trace_skeleton(etl.thin(ink))
                            if clean is not None
                            else etl.vectorize(record)
                        )
                    ]
                )
            records.append(
                {
                    "dataset": dataset,
                    "record": index,
                    "sheet": fields[1],
                    "group": group,
                    "label": label,
                }
            )
    if include_etl7:
        sources["etl7"] = {
            "files": {
                name: {"records": count, "sha256": sha}
                for name, (count, sha) in kana_etl7.SOURCES.items()
            },
            "records": sum(count for count, _ in kana_etl7.SOURCES.values()),
        }
        for row in kana_etl7.records(data_root):
            if row["label"] is None or row["label"] not in LABELS:
                continue
            pixels = row.pop("image")
            image = np.array(pixels, dtype=np.uint8)
            threshold = etl.otsu_threshold(pixels)
            border = np.concatenate(
                [image[0], image[-1], image[1:-1, 0], image[1:-1, -1]]
            )
            invert = np.median(border) > threshold
            if invert:
                image = 15 - image
            cutoff = 15 - threshold if invert else threshold + 1
            image = np.where(image >= cutoff, image * 17, 0).astype(np.uint8)
            if clean is not None:
                gray, ink = clean.clean_image(pixels)
                image = np.array(gray, dtype=np.uint8)
            payload.extend(struct.pack("<HH", 64, 63))
            payload.extend(image.tobytes())
            if ink_proxy:
                proxy_strokes.append(
                    [
                        [{"x": x, "y": y} for x, y in stroke]
                        for stroke in (
                            etl.trace_skeleton(etl.thin(ink))
                            if clean is not None
                            else etl.vectorize_image(pixels)
                        )
                    ]
                )
            records.append(row)
    for dataset in sorted(sources):
        groups = sorted(
            {r["group"] for r in records if r["dataset"] == dataset},
            key=lambda g: digest(f"{seed}:{g}".encode()),
        )
        # Group counts, not images, determine assignment. No image or class score
        # influences split membership; publish counts to expose imbalance.
        a = round(len(groups) * 0.70)
        b = round(len(groups) * 0.85)
        mapping = {
            g: ("train" if i < a else "validation" if i < b else "test")
            for i, g in enumerate(groups)
        }
        for r in records:
            if r["dataset"] == dataset:
                r["split"] = mapping[r["group"]]
    output = subprocess.run(
        [str(ROOT / "target/release/kana-rasterize"), "--side", str(side)],
        input=payload,
        stdout=subprocess.PIPE,
        check=True,
    ).stdout
    images = np.frombuffer(output, dtype="<f4").copy().reshape(-1, 1, side, side)
    assert len(images) == len(records)
    proxy_images = None
    if ink_proxy:
        proxy_bytes = subprocess.check_output(
            [
                str(ROOT / "target/release/kana-rasterize"),
                "--ink-batch",
                "--side",
                str(side),
            ],
            input=json.dumps(proxy_strokes).encode(),
        )
        proxy_images = (
            np.frombuffer(proxy_bytes, dtype="<f4").copy().reshape(-1, 1, side, side)
        )
        empty = proxy_images.sum(axis=(1, 2, 3)) == 0
        proxy_images[empty] = images[empty]
        write(
            run / "proxy-preparation.json",
            {
                "empty_proxy_grayscale_fallbacks": int(empty.sum()),
                "samples": len(images),
                "vectorizer_sha256": digest(
                    (ROOT / "tools/check-kana-etl.py").read_bytes()
                ),
            },
        )
    write(
        run / "split.json",
        {
            "seed": seed,
            "sources": sources,
            "method": {
                "etl4": "sheet-disjoint (one sheet per writer)",
                "etl5": "sex/age/industry/occupation/collection-date grouped; conservative, writer identity unverified",
                **({"etl7": kana_etl7.METHOD} if include_etl7 else {}),
            },
            "records": records,
        },
    )
    np.savez_compressed(
        run / "prepared.npz",
        images=images,
        labels=np.array([LABELS.index(r["label"]) for r in records]),
        **({"proxy_images": proxy_images} if ink_proxy else {}),
    )
    return images, records, sources, proxy_images


def reuse_prepared(source, run, config, side):
    if not any(
        source.resolve().is_relative_to((ROOT / area).resolve())
        for area in ("target", "kana-artifacts")
    ):
        raise ValueError(
            "Prepared ETL inputs must remain below target/ or kana-artifacts/"
        )
    previous = json.loads((source / "config.json").read_text())
    for key in ("seed", "ink_proxy", "etl7", "rasterizer", "etl7_reader_sha256"):
        if previous.get(key) != config.get(key):
            raise ValueError(f"Prepared cohort has incompatible {key}")
    if previous.get("clean_scans", False) != config.get(
        "clean_scans", False
    ) or previous.get("scan_preprocessor_sha256") != config.get(
        "scan_preprocessor_sha256"
    ):
        raise ValueError("Prepared cohort has incompatible scan preprocessing")
    split_bytes = (source / "split.json").read_bytes()
    split = json.loads(split_bytes)
    with np.load(source / "prepared.npz") as data:
        images = data["images"]
        proxy = data["proxy_images"] if config["ink_proxy"] else None
        labels = data["labels"]
    if images.shape != (len(split["records"]), 1, side, side):
        raise ValueError("Invalid prepared image shape")
    if labels.tolist() != [LABELS.index(r["label"]) for r in split["records"]]:
        raise ValueError("Prepared labels disagree with frozen split")
    for x in (images, proxy):
        if x is not None and (
            x.shape != images.shape
            or not np.isfinite(x).all()
            or x.min() < 0
            or x.max() > 1
        ):
            raise ValueError("Invalid prepared image values")
    (run / "split.json").write_bytes(split_bytes)
    os.link(source / "prepared.npz", run / "prepared.npz")
    if proxy is not None:
        (run / "proxy-preparation.json").write_bytes(
            (source / "proxy-preparation.json").read_bytes()
        )
    write(
        run / "preparation-reuse.json",
        {
            "source": str(source),
            "split_sha256": digest(split_bytes),
            "prepared_sha256": digest((source / "prepared.npz").read_bytes()),
            "source_config": previous,
            "note": "Same frozen arrays; this run does not independently reproduce preprocessing.",
        },
    )
    return images, split["records"], split["sources"], proxy


class Net(nn.Module):
    def __init__(self, side=32, depth=2, width=1):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8 * width, 3, padding=1)
        self.conv2 = nn.Conv2d(8 * width, 16 * width, 3, padding=1)
        self.conv3 = (
            nn.Conv2d(16 * width, 32 * width, 3, padding=1) if depth == 3 else None
        )
        self.fc = nn.Linear(
            32 * width * (side // 8) ** 2
            if depth == 3
            else 16 * width * (side // 4) ** 2,
            92,
        )

    def forward(self, x):
        x = F.max_pool2d(F.relu(self.conv1(x)), 2)
        x = F.max_pool2d(F.relu(self.conv2(x)), 2)
        if self.conv3 is not None:
            x = F.max_pool2d(F.relu(self.conv3(x)), 2)
        return self.fc(x.flatten(1))


class ResidualBlock(nn.Module):
    """Same-shape residual block; no training-only normalization state."""

    def __init__(self, channels):
        super().__init__()
        self.conv1 = nn.Conv2d(channels, channels, 3, padding=1)
        self.conv2 = nn.Conv2d(channels, channels, 3, padding=1)

    def forward(self, x):
        return F.relu(x + self.conv2(F.relu(self.conv1(x))))


class ResidualNet(Net):
    def __init__(self, side=64, depth=3, width=2):
        super().__init__(side, depth, width)
        self.blocks = nn.ModuleList(
            [ResidualBlock(c * width) for c in [8, 16, 32][:depth]]
        )

    def forward(self, x):
        for conv, block in zip([self.conv1, self.conv2, self.conv3], self.blocks):
            x = block(F.max_pool2d(F.relu(conv(x)), 2))
        return self.fc(x.flatten(1))


def make_net(config):
    side = int(config["rasterizer"].split("-")[2])
    cls = ResidualNet if config.get("family") == "residual" else Net
    net = cls(side, config.get("depth", 2), config.get("width", 1))
    if config.get("channels_last"):
        net.to(memory_format=torch.channels_last)
    return net


def initialize(net, source, side, depth, width):
    """Warm start, optionally widening cloned channels with 1% weight noise."""
    config = json.loads((source / "config.json").read_text())
    old_width = config.get("width", 1)
    if (
        config["depth"] != depth
        or width % old_width
        or config["rasterizer"] != (RASTER if side == 32 else "kana-bitmap-64-v1")
    ):
        raise ValueError("Incompatible warm-start architecture")
    old = Net(side, depth, old_width)
    old.load_state_dict(torch.load(source / "best.pt", weights_only=True))
    factor = width // old_width
    state = {}
    for name, value in old.state_dict().items():
        if factor == 1:
            state[name] = value
        elif name == "fc.weight":
            spatial = (side // (8 if depth == 3 else 4)) ** 2
            state[name] = (
                value.reshape(92, -1, spatial)
                .repeat_interleave(factor, 1)
                .reshape(92, -1)
                / factor
            )
        elif name == "fc.bias":
            state[name] = value
        elif name.endswith("bias"):
            state[name] = value.repeat_interleave(factor)
        else:
            state[name] = value.repeat_interleave(factor, 0)
            if name != "conv1.weight":
                state[name] = state[name].repeat_interleave(factor, 1) / factor
        if factor > 1 and name.endswith("weight"):
            state[name] += torch.randn_like(state[name]) * state[name].std() * 0.01
    net.load_state_dict(state)
    return {
        "source": str(source),
        "checkpoint_sha256": digest((source / "best.pt").read_bytes()),
        "config": config,
        "widening_noise_relative_std": 0.01 if factor > 1 else 0,
    }


def source_loss(logits, labels, datasets, known_script=True):
    """Equal source contribution, with known-script classification in each."""
    losses = []
    for dataset in sorted(set(datasets)):
        ids = [i for i, d in enumerate(datasets) if d == dataset]
        offset = 46 if known_script and dataset == "etl5" else 0
        end = offset + 46 if known_script else 92
        losses.append(F.cross_entropy(logits[ids, offset:end], labels[ids] - offset))
    return torch.stack(losses).mean()


def augment(x, *, morphology=True):
    n = len(x)
    angle = (torch.rand(n) - 0.5) * 0.30
    scale = 0.88 + torch.rand(n) * 0.24
    shear = (torch.rand(n) - 0.5) * 0.24
    theta = torch.zeros(n, 2, 3)
    theta[:, 0, 0] = angle.cos() * scale
    theta[:, 0, 1] = -angle.sin() + shear
    theta[:, 1, 0] = angle.sin()
    theta[:, 1, 1] = angle.cos() * scale
    theta[:, :, 2] = (torch.rand(n, 2) - 0.5) * 0.13
    x = F.grid_sample(
        x, F.affine_grid(theta, x.size(), align_corners=False), align_corners=False
    )
    # Scans and canvas lines differ in thickness; learn both without flipping ink.
    choice = random.random()
    # Consume the draw even when disabled, preserving paired experiment RNG state.
    if morphology and choice < 0.15:
        x = F.max_pool2d(x, 3, stride=1, padding=1)
    elif morphology and choice < 0.3:
        x = 1 - F.max_pool2d(1 - x, 3, stride=1, padding=1)
    return x


def metrics(logits, labels, records):
    result = {}
    for dataset in sorted({r["dataset"] for r in records}):
        offset = 46 if dataset == "etl5" else 0
        ids = [i for i, r in enumerate(records) if r["dataset"] == dataset]
        z = logits[ids, offset : offset + 46]
        y = labels[ids] - offset
        top = z.topk(5, dim=1).indices
        confusion = np.zeros((46, 46), dtype=int)
        for actual, pred in zip(y.tolist(), top[:, 0].tolist()):
            confusion[actual, pred] += 1
        result[dataset] = {
            "samples": len(ids),
            "macro_top1": float(np.mean(np.diag(confusion) / confusion.sum(axis=1))),
            "top1": (top[:, 0] == y).float().mean().item(),
            "top5": (top == y[:, None]).any(1).float().mean().item(),
            "labels": LABELS[offset : offset + 46],
            "confusion": confusion.tolist(),
        }
    return result


def checkpoint_key(selection, validation_loss, validation_metrics):
    """Minimize this key; strict comparison preserves the first exact tie.

    Accuracy is known-script top-1, equally weighted across present sources.
    Derive it from integer confusion counts rather than rounded tensor means.
    Only the caller's frozen validation cohort belongs here.
    """
    if not math.isfinite(validation_loss):
        raise ValueError("Non-finite validation loss")
    if selection == "loss":
        return (validation_loss,)
    if selection != "source-top1":
        raise ValueError(f"Unknown checkpoint selection: {selection}")
    if not validation_metrics:
        raise ValueError("Checkpoint selection needs validation sources")
    accuracies = []
    for source in sorted(validation_metrics):
        metric = validation_metrics[source]
        confusion = metric["confusion"]
        count = sum(sum(row) for row in confusion)
        if count <= 0 or count != metric["samples"]:
            raise ValueError("Invalid validation confusion counts")
        accuracies.append(sum(row[i] for i, row in enumerate(confusion)) / count)
    return (-math.fsum(accuracies) / len(accuracies), validation_loss)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--data", type=Path, default=ROOT / "kana-artifacts/kana-training/data"
    )
    p.add_argument("--run", type=Path, required=True)
    p.add_argument("--epochs", type=int, default=50)
    p.add_argument("--seed", type=int, default=20260908)
    p.add_argument(
        "--training-seed",
        type=int,
        help="Initialization/augmentation seed; --seed still fixes the cohort",
    )
    p.add_argument("--family", choices=["plain", "residual"], default="plain")
    p.add_argument(
        "--validation-only", action="store_true", help="Do not evaluate test records"
    )
    p.add_argument("--threads", type=int, default=6)
    p.add_argument("--side", type=int, choices=[32, 64], default=32)
    p.add_argument(
        "--ink-proxy",
        action="store_true",
        help="Mix scan and skeleton rasters in training; select on skeleton validation inputs",
    )
    p.add_argument(
        "--etl7",
        action="store_true",
        help="Add ETL7 hiragana with metadata-grouped splits across sizes",
    )
    p.add_argument("--depth", type=int, choices=[2, 3], default=2)
    p.add_argument(
        "--prepared-from",
        type=Path,
        help="Reuse a matching frozen preparation without repeating scan vectorization",
    )
    p.add_argument("--width", type=int, choices=[1, 2, 4], default=1)
    p.add_argument("--initialize-from", type=Path)
    p.add_argument("--source-balanced", action="store_true")
    p.add_argument("--learning-rate", type=float, default=0.002)
    p.add_argument("--clean-scans", action="store_true")
    p.add_argument("--prepare-only", action="store_true")
    p.add_argument(
        "--channels-last",
        action="store_true",
        help="Opt-in CPU convolution memory layout; does not change portable tensor order",
    )
    p.add_argument(
        "--keep-checkpoints",
        action="store_true",
        help="Retain every epoch for future diagnostic replay",
    )
    p.add_argument(
        "--checkpoint-selection", choices=["loss", "source-top1"], default="loss",
        help="Minimize validation loss, or maximize equal-source known-script top-1 with loss as tie-breaker",
    )
    p.add_argument("--loss-scope", choices=["all", "known-script"], default="all")
    a = p.parse_args()
    architecture = ARCH if a.side == 32 else "conv8x3-pool-conv16x3-pool-linear92-64-v1"
    if a.depth == 3:
        architecture = f"conv8x3-pool-conv16x3-pool-conv32x3-pool-linear92-{a.side}-v1"
    if a.width != 1:
        architecture = f"kana-conv-depth{a.depth}-width{a.width}-{a.side}-v1"
    if a.family == "residual":
        if a.initialize_from:
            raise SystemExit("Residual trials require fresh initialization")
        architecture = f"kana-residual-depth{a.depth}-width{a.width}-{a.side}-v1"
    rasterizer = RASTER if a.side == 32 else "kana-bitmap-64-v1"
    if a.epochs < 1 or a.threads < 1:
        raise SystemExit("epochs and threads must be positive")
    if not any(
        a.run.resolve().is_relative_to((ROOT / area).resolve())
        for area in ("target", "kana-artifacts")
    ):
        raise SystemExit(
            "Training artifacts contain restricted data; keep runs under target/ or kana-artifacts/"
        )
    if a.run.exists():
        raise SystemExit(
            "Choose a new run directory; never overwrite a holdout experiment"
        )
    a.run.mkdir(parents=True)
    torch.set_num_threads(a.threads)
    training_seed = a.seed if a.training_seed is None else a.training_seed
    torch.manual_seed(training_seed)
    np.random.seed(training_seed)
    random.seed(training_seed)
    torch.use_deterministic_algorithms(True)
    config = {
        "family": a.family,
        "training_seed": training_seed,
        "validation_only": a.validation_only,
        "keep_checkpoints": a.keep_checkpoints,
        "checkpoint_selection": a.checkpoint_selection,
        "channels_last": a.channels_last,
        "ink_proxy": a.ink_proxy,
        "depth": a.depth,
        "width": a.width,
        "source_balanced": a.source_balanced,
        "loss_scope": a.loss_scope,
        "clean_scans": a.clean_scans,
        "scan_preprocessor_sha256": digest(
            (ROOT / "tools/check-kana-preprocessing.py").read_bytes()
        )
        if a.clean_scans
        else None,
        "learning_rate": a.learning_rate,
        "etl7": a.etl7,
        "etl7_reader_sha256": digest(Path(kana_etl7.__file__).read_bytes()),
        "seed": a.seed,
        "epochs": a.epochs,
        "torch": torch.__version__,
        "numpy": np.__version__,
        "architecture": architecture,
        "rasterizer": rasterizer,
        "script_sha256": digest(Path(__file__).read_bytes()),
        "raster_source_sha256": digest(
            (ROOT / "crates/core/src/kana_vision.rs").read_bytes()
        ),
        "threads": a.threads,
    }
    (a.run / "training-script.py").write_bytes(Path(__file__).read_bytes())
    (a.run / "raster-source.rs").write_bytes(
        (ROOT / "crates/core/src/kana_vision.rs").read_bytes()
    )
    (a.run / "kana_etl7.py").write_bytes(Path(kana_etl7.__file__).read_bytes())
    (a.run / "check-kana-etl.py").write_bytes(
        (ROOT / "tools/check-kana-etl.py").read_bytes()
    )
    if a.clean_scans:
        (a.run / "check-kana-preprocessing.py").write_bytes(
            (ROOT / "tools/check-kana-preprocessing.py").read_bytes()
        )
    write(a.run / "config.json", config)
    started = time.monotonic()
    if a.prepared_from:
        images, records, sources, proxy_images = reuse_prepared(
            a.prepared_from, a.run, config, a.side
        )
    else:
        images, records, sources, proxy_images = prepare(
            a.data, a.run, a.seed, a.side, a.ink_proxy, a.etl7, a.clean_scans
        )
    if a.prepare_only:
        print("Prepared frozen cohort; no model trained", flush=True)
        return
    proxy = torch.from_numpy(proxy_images) if proxy_images is not None else None
    x = torch.from_numpy(images)
    y = torch.tensor([LABELS.index(r["label"]) for r in records])
    splits = {
        s: torch.tensor([i for i, r in enumerate(records) if r["split"] == s])
        for s in ["train", "validation", "test"]
    }
    print("Split sizes:", {k: len(v) for k, v in splits.items()}, flush=True)
    net = make_net(config)
    if a.initialize_from:
        initialization = initialize(net, a.initialize_from, a.side, a.depth, a.width)
        if (a.initialize_from / "split.json").read_bytes() != (
            a.run / "split.json"
        ).read_bytes():
            raise ValueError("Warm-start checkpoint uses a different cohort split")
        write(a.run / "initialization.json", initialization)
    optimizer = torch.optim.AdamW(
        net.parameters(), lr=a.learning_rate, weight_decay=0.01
    )
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(
        optimizer, a.epochs, eta_min=0.0001
    )
    best = None
    history = []
    for epoch in range(a.epochs):
        epoch_started = time.monotonic()
        net.train()
        total = 0
        order = splits["train"][torch.randperm(len(splits["train"]))]
        for batch in order.split(128):
            optimizer.zero_grad()
            inputs = x[batch]
            if proxy is not None:
                inputs = torch.where(
                    torch.rand(len(batch), 1, 1, 1) < 0.5, proxy[batch], inputs
                )
            logits = net(augment(inputs))
            loss = (
                source_loss(
                    logits,
                    y[batch],
                    [records[i]["dataset"] for i in batch.tolist()],
                    known_script=a.loss_scope == "known-script",
                )
                if a.source_balanced
                else F.cross_entropy(logits, y[batch])
            )
            loss.backward()
            optimizer.step()
            total += loss.item() * len(batch)
        scheduler.step()
        net.eval()
        with torch.no_grad():
            validation_input = proxy if proxy is not None else x
            val = torch.cat(
                [net(validation_input[b]) for b in splits["validation"].split(256)]
            )
            val_loss = (
                source_loss(
                    val,
                    y[splits["validation"]],
                    [records[i]["dataset"] for i in splits["validation"].tolist()],
                    known_script=a.loss_scope == "known-script",
                )
                if a.source_balanced
                else F.cross_entropy(val, y[splits["validation"]])
            ).item()
        row = {
            "epoch": epoch + 1,
            "train_loss": total / len(order),
            "validation_loss": val_loss,
            "validation_metrics": metrics(
                val,
                y[splits["validation"]],
                [records[i] for i in splits["validation"].tolist()],
            ),
        }
        key = checkpoint_key(a.checkpoint_selection, val_loss, row["validation_metrics"])
        row["checkpoint_key"] = list(key)
        if a.keep_checkpoints:
            torch.save(net.state_dict(), a.run / f"epoch-{epoch + 1:03}.pt")
        if best is None or key < best:
            best = key
            torch.save(net.state_dict(), a.run / "best.pt")
            best_epoch = epoch + 1
        history.append(row)
        print(
            json.dumps(
                {k: v for k, v in row.items() if k != "validation_metrics"}
                | {
                    "seconds": time.monotonic() - epoch_started,
                    "top1": {
                        d: m["top1"] for d, m in row["validation_metrics"].items()
                    },
                }
            ),
            flush=True,
        )
        write(a.run / "history.json", history)
    net.load_state_dict(torch.load(a.run / "best.pt", weights_only=True))
    net.eval()
    with torch.no_grad():
        results = {}
        proxy_results = {}
        for split in ["validation"] if a.validation_only else ["validation", "test"]:
            ids = splits[split]
            logits = torch.cat([net(x[b]) for b in ids.split(256)])
            results[split] = metrics(logits, y[ids], [records[i] for i in ids.tolist()])
            if proxy is not None:
                logits = torch.cat([net(proxy[b]) for b in ids.split(256)])
                proxy_results[split] = metrics(
                    logits, y[ids], [records[i] for i in ids.tolist()]
                )
        synthetic = torch.tensor(
            [((i * 37 + 11) % 101) / 100 for i in range(a.side * a.side)],
            dtype=torch.float32,
        ).reshape(1, 1, a.side, a.side)
        write(
            a.run / "synthetic-golden.json",
            {
                "fixture": "pixel[i] = ((i * 37 + 11) % 101) / 100; synthetic arithmetic fixture, no ETL pixels",
                "side": a.side,
                "logits": net(synthetic)[0].tolist(),
            },
        )
        golden_ids = splits["validation" if a.validation_only else "test"][:8]
        golden = {
            "input": x[golden_ids].flatten(1).tolist(),
            "logits": net(x[golden_ids]).tolist(),
        }
    weights = torch.cat([v.detach().flatten() for v in net.parameters()]).tolist()
    model = {
        "architecture": architecture,
        "rasterizer": rasterizer,
        "labels": LABELS,
        "weights": weights,
    }
    write(a.run / "model.json", model)
    binary = (
        (b"IDKANA03" if a.width != 1 else b"IDKANA02" if a.depth == 3 else b"IDKANA01")
        + struct.pack("<I", a.side)
        + (struct.pack("<II", a.depth, a.width) if a.width != 1 else b"")
        + struct.pack("<92I", *map(ord, LABELS))
        + np.asarray(weights, dtype="<f4").tobytes()
    )
    # Residual weights are research artifacts until a portable runtime is verified.
    if a.family == "plain":
        (a.run / "model.bin").write_bytes(binary)
    write(a.run / "golden.json", golden)
    report = {
        "config": config,
        "sources": sources,
        "split_sha256": digest((a.run / "split.json").read_bytes()),
        "model_sha256": digest((a.run / "model.json").read_bytes()),
        "binary_model_sha256": digest(binary) if a.family == "plain" else None,
        "parameter_count": len(weights),
        "float32_weight_bytes": len(weights) * 4,
        "best_epoch": best_epoch,
        "seconds": time.monotonic() - started,
        "results": results,
        "proxy_results": proxy_results,
        "checkpoint_metric": "equal-source known-script validation top-1; ties: validation loss, then earliest epoch"
        if a.checkpoint_selection == "source-top1"
        else f"equal-source {a.loss_scope} validation cross-entropy"
        if a.source_balanced
        else "skeleton validation cross-entropy"
        if a.ink_proxy
        else "grayscale validation cross-entropy",
        "limitations": [
            "Experimental identity candidates; softmax is uncalibrated and not an acceptance or quality grade.",
            "Historical scanned handwriting does not establish modern iPad pen accuracy.",
            "ETL5 and ETL7 split groups shared demographics, not verified writer identifiers.",
            "ETL4/ETL5 test metrics are a reused benchmark; only validation selects checkpoints.",
            "No invalid-ink rejection model has been trained.",
        ],
    }
    write(a.run / "report.json", report)
    print(
        json.dumps(
            {
                "best_epoch": best_epoch,
                "seconds": report["seconds"],
                "results": {
                    s: {
                        d: {
                            k: v
                            for k, v in r.items()
                            if k not in ["confusion", "labels"]
                        }
                        for d, r in ds.items()
                    }
                    for s, ds in results.items()
                },
            },
            ensure_ascii=False,
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
