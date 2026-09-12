#!/usr/bin/env python3
"""Regenerate all portable arithmetic fixtures with pinned CPU PyTorch; no ETL data.

Run from the repository root in the trainer's pinned environment.
"""

import importlib.util
import json
import sys
from pathlib import Path

import torch

sys.path.insert(0, str(Path("tools").resolve()))
spec = importlib.util.spec_from_file_location(
    "trainer", Path("tools/train-kana-vision.py")
)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
torch.set_num_threads(1)
torch.use_deterministic_algorithms(True)
cases = []
for side in [32, 64]:
    for depth, width, residual in [
        (d, w, r) for d in [2, 3] for w in [1, 2, 4] for r in [False, True]
    ]:
        net = (m.ResidualNet if residual else m.Net)(side, depth, width)
        offset = 0
        with torch.no_grad():
            for p in net.parameters():
                w = torch.tensor(
                    [
                        (((i * 17 + 13) % 37) - 18) / 80
                        for i in range(offset, offset + p.numel())
                    ],
                    dtype=torch.float32,
                ).reshape(p.shape)
                p.copy_(w)
                offset += p.numel()
            x = torch.tensor(
                [((i * 37 + 11) % 101) / 100 for i in range(side * side)],
                dtype=torch.float32,
            ).reshape(1, 1, side, side)
            logits = net(x)[0].tolist()
        cases.append(
            {
                "residual": residual,
                "side": side,
                "depth": depth,
                "width": width,
                "parameters": offset,
                "logits": logits,
            }
        )
Path("crates/core/assets/kana_vision_arithmetic.json").write_text(
    json.dumps(
        {
            "source": "Synthetic arithmetic only; no ETL images or learned weights. PyTorch "
            + torch.__version__,
            "input": "float32(((i*37+11)%101)/100)",
            "weights": "float32((((i*17+13)%37)-18)/80), concatenated conv weight, conv bias per layer, then linear weight/bias, then residual block conv weight/bias",
            "cases": cases,
        },
        indent=2,
    )
    + "\n"
)
print(
    [
        (c["side"], c["depth"], c["parameters"], max(map(abs, c["logits"])))
        for c in cases
    ]
)
