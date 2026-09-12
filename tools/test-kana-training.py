#!/usr/bin/env python3
"""Numerical checks for warm-start widening and source-balanced objectives.

Run with the trainer's pinned PyTorch/NumPy environment. No ETL data needed.
"""

import importlib.util
import json
import tempfile
from pathlib import Path
from unittest.mock import patch

import torch

spec = importlib.util.spec_from_file_location(
    "trainer", Path(__file__).with_name("train-kana-vision.py")
)
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def main():
    def cohort(correct, total):
        return {"samples": total, "confusion": [[correct, total - correct], [0, 0]]}

    key = trainer.checkpoint_key
    # A large easy source must not drown out a smaller difficult source.
    first = {"etl4": cohort(6, 10), "etl5": cohort(990, 1000)}
    improved = {"etl4": cohort(8, 10), "etl5": cohort(980, 1000)}
    assert key("source-top1", 2.0, improved) < key("source-top1", 1.0, first)
    assert key("loss", 2.0, improved) > key("loss", 1.0, first)
    replicated = {"etl4": cohort(600, 1000), "etl5": first["etl5"]}
    assert key("source-top1", 1.0, first) == key("source-top1", 1.0, replicated)
    assert key("source-top1", 0.9, first) < key("source-top1", 1.0, first)
    candidates = [(1.0, first), (2.0, improved), (1.5, improved), (1.5, improved)]
    assert min(range(4), key=lambda i: key("source-top1", *candidates[i])) == 2
    for selection, loss, values in [
        ("loss", float("nan"), first),
        ("source-top1", float("inf"), first),
        ("source-top1", 1.0, {}),
        ("source-top1", 1.0, {"etl4": cohort(0, 0)}),
        ("unknown", 1.0, first),
    ]:
        try:
            key(selection, loss, values)
        except ValueError:
            pass
        else:
            raise AssertionError("Invalid checkpoint inputs were accepted")
    torch.set_num_threads(3)
    torch.manual_seed(42)
    with tempfile.TemporaryDirectory() as temp:
        source = Path(temp)
        old = trainer.Net(32, 3)
        torch.save(old.state_dict(), source / "best.pt")
        (source / "config.json").write_text(
            json.dumps({"depth": 3, "rasterizer": trainer.RASTER})
        )
        inputs = torch.rand(3, 1, 32, 32)
        for width in [1, 2, 4]:
            wider = trainer.Net(32, 3, width)
            # Test the underlying channel replication independently of deliberate
            # symmetry-breaking noise used during real training.
            with patch.object(torch, "randn_like", side_effect=torch.zeros_like):
                trainer.initialize(wider, source, 32, 3, width)
            torch.testing.assert_close(wider(inputs), old(inputs), atol=1e-6, rtol=1e-5)
        wider = trainer.Net(32, 3, 2)
        trainer.initialize(wider, source, 32, 3, 2)
        assert not torch.equal(wider.conv1.weight[0], wider.conv1.weight[1])
    residual = trainer.ResidualBlock(8)
    for parameter in residual.parameters():
        parameter.data.zero_()
    positive = torch.rand(2, 8, 8, 8, requires_grad=True)
    torch.testing.assert_close(residual(positive), positive)
    residual(positive).sum().backward()
    torch.testing.assert_close(positive.grad, torch.ones_like(positive))
    model = trainer.ResidualNet(64, 3, 2)
    assert sum(p.numel() for p in model.parameters()) * 4 < 10_000_000
    assert model(torch.rand(2, 1, 64, 64)).shape == (2, 92)
    logits = torch.randn(4, 92, requires_grad=True)
    labels = torch.tensor([0, 1, 46, 2])
    sources = ["etl4", "etl4", "etl5", "etl7"]
    loss = trainer.source_loss(logits, labels, sources)
    # Replicating one complete source must not increase its objective weight.
    duplicated = torch.tensor([0, 1, 0, 1, 2, 3])
    torch.testing.assert_close(
        loss,
        trainer.source_loss(
            logits[duplicated], labels[duplicated], [sources[i] for i in duplicated]
        ),
    )
    all_logits = logits.detach().clone().requires_grad_()
    trainer.source_loss(all_logits, labels, sources, known_script=False).backward()
    assert (all_logits.grad[2, :46] != 0).all()
    loss.backward()
    assert torch.equal(logits.grad[2, :46], torch.zeros(46))
    assert torch.equal(logits.grad[[0, 1, 3], 46:], torch.zeros(3, 46))
    print(
        "Training numerical checks passed: checkpoint selection, widening, symmetry breaking, source balance, script masking"
    )


if __name__ == "__main__":
    main()
