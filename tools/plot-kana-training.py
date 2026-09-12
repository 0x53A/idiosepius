#!/usr/bin/env python3
"""Plot aggregate learning curves; no restricted images or per-record data."""

import argparse
import hashlib
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", type=Path, nargs="+")
    parser.add_argument(
        "--baseline", type=Path, required=True, help="Corrected-input comparison JSON"
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    comparison = json.loads(args.baseline.read_text())
    if (
        comparison["split"] != "validation"
        or not comparison["clean_scans"]
        or len(comparison["models"]) != 1
    ):
        raise ValueError(
            "Baseline must contain one model on corrected validation inputs"
        )
    baseline = next(iter(comparison["models"].values()))
    fig, axes = plt.subplots(2, 2, figsize=(12, 8), constrained_layout=True)
    provenance = {}
    for run in args.runs:
        raw = (run / "history.json").read_bytes()
        history = json.loads(raw)
        provenance[str(run)] = hashlib.sha256(raw).hexdigest()
        config = json.loads((run / "config.json").read_text())
        label = f"{config.get('family', 'plain')}, seed {config.get('training_seed', config['seed'])}"
        epochs = [r["epoch"] for r in history]
        selected = min(history, key=lambda row: row["validation_loss"])
        for ax, dataset in zip(axes.flat, ["etl4", "etl5", "etl7"]):
            (line,) = ax.plot(
                epochs,
                [100 * r["validation_metrics"][dataset]["top1"] for r in history],
                label=label,
            )
            ax.scatter(
                selected["epoch"],
                100 * selected["validation_metrics"][dataset]["top1"],
                color=line.get_color(),
                edgecolors="black",
                zorder=3,
            )
        (line,) = axes[1, 1].plot(
            epochs, [r["validation_loss"] for r in history], label=label
        )
        axes[1, 1].scatter(
            selected["epoch"],
            selected["validation_loss"],
            color=line.get_color(),
            edgecolors="black",
            zorder=3,
        )
    for ax, dataset in zip(axes.flat, ["etl4", "etl5", "etl7"]):
        ax.axhline(
            100 * baseline["results"]["proxy_images"][dataset]["top1"],
            color="black",
            linestyle="--",
            linewidth=1,
            label="deployed checkpoint",
        )
        ax.set_title(dataset.upper() + " — known-script top-1")
        ax.set_ylabel("Validation accuracy (%)")
    axes[1, 1].set_title("Checkpoint selection: all-92 cross-entropy")
    axes[1, 1].set_ylabel("Validation loss")
    for ax in axes.flat:
        ax.set_xlabel("Epoch")
        ax.grid(alpha=0.2)
    axes[0, 0].legend(fontsize=8)
    axes[1, 1].legend(fontsize=8)
    fig.suptitle(
        "Fresh kana training on corrected validation proxies — dots mark selected checkpoints\nSame frozen cohort; these are not iPad accuracy measurements",
        fontsize=13,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output.with_suffix(".svg"))
    fig.savefig(args.output.with_suffix(".png"), dpi=150)
    plt.close(fig)
    args.output.with_suffix(".json").write_text(
        json.dumps({"history_sha256": provenance}, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
