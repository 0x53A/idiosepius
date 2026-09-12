#!/usr/bin/env python3
"""Run a predeclared, private stroke-domain fine-tuning experiment on CPU."""
import argparse
import hashlib
import importlib.util
import json
import math
import platform
import random
import struct
import time
from pathlib import Path

import numpy as np
import torch
from torch.nn import functional as F

spec = importlib.util.spec_from_file_location("trainer", Path(__file__).with_name("train-kana-vision.py"))
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def evaluate(net, arrays, records):
    result = {}
    net.eval()
    with torch.no_grad():
        for view in ["original", "clean"]:
            logits = torch.cat([net(batch) for batch in arrays[f"validation_{view}"].split(256)])
            labels = arrays["validation_labels"]
            metrics = trainer.metrics(logits, labels, records)
            for source, row in metrics.items():
                ids = [i for i, r in enumerate(records) if r["dataset"] == source]
                row["loss"] = F.cross_entropy(logits[ids], labels[ids]).item()
                result[f"{view}:{source}"] = row
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--prepared", type=Path, required=True)
    p.add_argument("--initialize-from", type=Path, required=True)
    p.add_argument("--protocol", type=Path, required=True)
    p.add_argument("--candidate", choices=["plain", "residual"], required=True)
    p.add_argument("--run", type=Path, required=True)
    p.add_argument("--threads", type=int, default=6)
    a = p.parse_args()
    if a.run.exists() or a.threads < 1:
        p.error("Choose a new run and positive thread count")
    protocol = json.loads(a.protocol.read_text())
    recipe = protocol["candidates"][a.candidate]
    if a.initialize_from.name != recipe["initialization"]:
        p.error("Initialization does not match the frozen protocol")
    prep = json.loads((a.prepared / "preparation.json").read_text())
    assert sha(a.prepared / "trial.npz") == prep["trial_sha256"]
    assert sha(a.prepared / "records.json") == prep["records_sha256"]
    records = json.loads((a.prepared / "records.json").read_text())
    assert set(records) == {"train", "validation"}
    assert all(r["split"] == k for k, rows in records.items() for r in rows)
    assert not {r["group"] for r in records["train"]} & {r["group"] for r in records["validation"]}
    allowed_sources=protocol.get("synthetic_sources",["embedded"])
    assert allowed_sources and set(allowed_sources)<= {"embedded","animcjk_clipped_hiragana"}
    assert all(i.split(":",1)[0] in allowed_sources for i in prep["synthetic_ids"])
    torch.set_num_threads(a.threads)
    torch.use_deterministic_algorithms(True)
    torch.manual_seed(protocol["seed"])
    np.random.seed(protocol["seed"])
    random.seed(protocol["seed"])
    config = json.loads((a.initialize_from / "config.json").read_text())
    assert config.get("family", "plain") == a.candidate
    config.update(channels_last=True, threads=a.threads, trial=protocol["version"], training_seed=protocol["seed"],
                  initialization_sha256=sha(a.initialize_from / "best.pt"), protocol_sha256=sha(a.protocol),
                  trial_data_sha256=prep["trial_sha256"], trial_script_sha256=sha(Path(__file__)),
                  trainer_source_sha256=sha(Path(trainer.__file__)), host=platform.node(),
                  torch=torch.__version__, numpy=np.__version__, epochs=protocol["epochs"])
    net, teacher = trainer.make_net(config), trainer.make_net(config)
    state = torch.load(a.initialize_from / "best.pt", weights_only=True)
    net.load_state_dict(state)
    teacher.load_state_dict(state)
    teacher.eval()
    teacher.requires_grad_(False)
    with np.load(a.prepared / "trial.npz") as source:
        arrays = {k: torch.from_numpy(source[k]) for k in source.files}
    assert len(arrays["train_labels"]) == len(records["train"])
    assert len(arrays["validation_labels"]) == len(records["validation"])
    for partition in ["train", "validation"]:
        assert arrays[f"{partition}_labels"].tolist() == [trainer.LABELS.index(r["label"]) for r in records[partition]]
    for key, x in arrays.items():
        if "labels" not in key:
            assert x.ndim == 4 and tuple(x.shape[1:]) == (1, 64, 64)
            assert torch.isfinite(x).all() and x.min() >= 0 and x.max() <= 1
    a.run.mkdir(parents=True)
    trainer.write(a.run / "config.json", config)
    for path, name in [(a.protocol, "protocol.json"), (Path(__file__), "trial-script.py"), (Path(trainer.__file__), "training-script.py")]:
        (a.run / name).write_bytes(path.read_bytes())
    initial = evaluate(net, arrays, records["validation"])
    trainer.write(a.run / "initial-validation.json", initial)
    optimizer = torch.optim.AdamW(net.parameters(), lr=recipe["learning_rate"])
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, protocol["epochs"], eta_min=recipe["learning_rate"] * .1)
    history, best = [], None
    for epoch in range(1, protocol["epochs"] + 1):
        start = time.monotonic()
        net.train()
        losses = []
        order = torch.randperm(len(arrays["train_labels"]))
        for batch in order.split(protocol["etl_batch_size"]):
            x = torch.where(torch.rand(len(batch), 1, 1, 1) < .5,
                            arrays["train_original"][batch], arrays["train_clean"][batch])
            x = trainer.augment(x)
            ids = torch.randint(len(arrays["synthetic_images"]), (protocol["synthetic_batch_size"],))
            synthetic = trainer.augment(arrays["synthetic_images"][ids],
                                        morphology=protocol.get("synthetic_morphology", True))
            optimizer.zero_grad()
            logits = net(x)
            with torch.no_grad():
                teacher_logits = teacher(x)
            temperature = protocol["teacher_temperature"]
            weight = protocol["synthetic_loss_weight"]
            loss = (1 - weight) * F.cross_entropy(logits, arrays["train_labels"][batch])
            loss += weight * F.cross_entropy(net(synthetic), arrays["synthetic_labels"][ids])
            loss += protocol["teacher_kl_weight"] * temperature ** 2 * F.kl_div(
                F.log_softmax(logits / temperature, dim=1),
                F.softmax(teacher_logits / temperature, dim=1), reduction="batchmean")
            if not torch.isfinite(loss):
                raise ValueError("Non-finite training loss")
            loss.backward()
            optimizer.step()
            losses.append(loss.item())
        scheduler.step()
        metrics = evaluate(net, arrays, records["validation"])
        mean_loss = math.fsum(m["loss"] for m in metrics.values()) / len(metrics)
        key = trainer.checkpoint_key("source-top1", mean_loss, metrics)
        torch.save(net.state_dict(), a.run / f"epoch-{epoch:03}.pt")
        if best is None or key < best:
            best, best_epoch = key, epoch
            torch.save(net.state_dict(), a.run / "best.pt")
        row = dict(epoch=epoch, train_loss=sum(losses) / len(losses), checkpoint_key=list(key), validation_metrics=metrics)
        history.append(row)
        trainer.write(a.run / "history.json", history)
        print(json.dumps(dict(epoch=epoch, best_epoch=best_epoch, seconds=time.monotonic()-start,
            top1={k: v["top1"] for k, v in metrics.items()})), flush=True)
    net.load_state_dict(torch.load(a.run / "best.pt", weights_only=True))
    net.eval()
    selected = evaluate(net, arrays, records["validation"])
    weights = np.concatenate([p.detach().numpy().flatten() for p in net.parameters()]).astype("<f4")
    family, depth, width = config.get("family", "plain"), config["depth"], config.get("width", 1)
    if family == "residual":
        header = b"IDKANA04" + struct.pack("<III", 64, depth, width)
    elif width != 1:
        header = b"IDKANA03" + struct.pack("<III", 64, depth, width)
    else:
        assert depth == 3
        header = b"IDKANA02" + struct.pack("<I", 64)
    binary = header + struct.pack("<92I", *map(ord, trainer.LABELS)) + weights.tobytes()
    assert len(binary) < 10_000_000 and np.isfinite(weights).all()
    (a.run / "model.bin").write_bytes(binary)
    with torch.no_grad():
        x = torch.tensor([((i * 37 + 11) % 101) / 100 for i in range(64 * 64)]).reshape(1, 1, 64, 64)
        trainer.write(a.run / "synthetic-golden.json", dict(input=x.flatten(1).tolist(), logits=net(x).tolist()))
    trainer.write(a.run / "report.json", dict(config=config, best_epoch=best_epoch, validation=selected,
        checkpoint_sha256=sha(a.run / "best.pt"), model_sha256=sha(a.run / "model.bin"),
        parameter_bytes=len(weights) * 4, note="Private stroke fine-tuning candidate; no test evaluated, no model deployed. See protocol for separate promotion gates."))


if __name__ == "__main__":
    main()
