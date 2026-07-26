# Local answer-grader evaluation

This is an independent, disposable benchmark harness. It borrows the useful
implementation choices from Benevolent Shoggoth—`llama.cpp` through
`llama-cpp-2` and Vulkan layer offload—but neither imports nor links Shoggoth.
The latest evaluated baseline and handoff notes are in
[`RESULTS.md`](RESULTS.md).

Every candidate receives the same system prompt and the same cases. The GGUF's
embedded chat template is used. Decoding is deterministic and constrained by a
JSON grammar to:

```json
{"verdict":"correct|incorrect|uncertain","reason":"..."}
```

This harness has its own shell because `llama-cpp-sys-2` needs CMake, libclang,
the Vulkan headers and `glslc`; the application workspace itself does not.
Enter it and run from this directory:

```sh
nix-shell
cargo run --release -- \
  --model ../../target/grader-eval/models/model.gguf \
  --model-id example \
  --cases cases.jsonl \
  --output ../../target/grader-eval/results/example-vulkan.json \
  --backend vulkan
```

Use `--backend cpu` for a CPU comparison. Reports include exact accuracy,
false accepts, false rejects, parse failures, latency, throughput, file size,
and process peak RSS. On an integrated Intel GPU, model buffers use shared
system memory, so the GGUF size and process RSS are more meaningful than a
nominal dedicated-VRAM figure.

CPU runs default to four threads. Set `--threads` explicitly when tuning, and
use `--limit N` for a quick performance sample before running the complete case
set.

Compare completed runs:

```sh
python3 compare.py ../../target/grader-eval/results/*.json
```

GGUFs and full JSON reports live under `../../target/grader-eval/` and are
deliberately not committed. The durable inputs are `cases.jsonl`, the prompt
and grammar in `src/main.rs`, and the summarized baseline in `RESULTS.md`.
Whenever any of those inputs changes, rerun every candidate before comparing
rankings.
