# Answer-grader evaluation

This is an independent, disposable benchmark harness for ranking candidate
models as a Control Systems answer grader. It speaks one thing —
`POST {base_url}/chat/completions`, the OpenAI-compatible shape — and is
deliberately tied to no vendor. The same binary ranks a hosted API, a local
`llama-server`, Ollama or vLLM; only `--base-url` and `--model` change. The
latest evaluated baseline and handoff notes are in [`RESULTS.md`](RESULTS.md).

Every candidate receives the same system prompt and the same cases, and is
constrained to:

```json
{"verdict":"correct|incorrect|uncertain","reason":"..."}
```

That constraint is a JSON schema sent as `response_format`, enforced by the
server. Servers vary in what they accept, so `--response-format` picks the
mechanism: `json-schema` (default, strict — llama-server, vLLM, OpenAI,
Ollama), `json-object` (valid JSON, shape unenforced — accepted by OpenAI and
Ollama, but **LM Studio rejects it with a 400**: it allows only `json_schema`
or `text`), or `none` (prompt only, sending no constraint field at all — the
portable escape hatch when a server's structured-output path misbehaves).
Output is parsed leniently in every mode: a fenced block or surrounding prose
is peeled off rather than scored as a parse failure, so `parse_failures`
measures the model, not its formatting habits.

**LM Studio is not currently usable as a backend** in any of the three modes —
`json_schema` returns an empty `content` with the answer never written,
`json_object` 400s, and `text` returns prose instead of the object. The
measurements are in [`RESULTS.md`](RESULTS.md); the fault is on its side, and
the one-line recheck for a later build is there too.

There is no `shell.nix` here and no system toolchain to install — dropping the
bundled `llama.cpp` removed the CMake, libclang and Vulkan-header
dependencies. A stock `cargo build` is enough.

```sh
cargo run --release -- \
  --base-url http://localhost:11434/v1 \
  --model qwen2.5:7b \
  --model-id qwen2.5-7b \
  --cases cases.jsonl \
  --output ../../target/grader-eval/results/qwen2.5-7b.json
```

Authentication is off by default, which is what a local server wants. For a
hosted endpoint, put the token in an environment variable and name it:
`--api-key-env OPENAI_API_KEY` (the default). An unset or empty variable sends
no `Authorization` header rather than an empty one.

A non-2xx response aborts the run with the endpoint, status and response body,
and writes no report — an unauthorized key or an unsupported `response_format`
must not be silently scored as a grading failure.

## Keeping runs comparable

`--temperature 0` is the default, so a run is reproducible and two candidates
differ because the models differ. Two things can take that away:

- Some hosted APIs **reject any non-default sampling parameter**. Use
  `--temperature-off` to omit the field entirely.
- With sampling out of your control, a single pass is a sample rather than a
  measurement. Raise `--repetitions` and read the spread before trusting a
  ranking that turns on a few points of accuracy.

`--seed` is forwarded when the endpoint honours it. Use `--limit N` for a quick
check before committing to the full case set.

`--max-tokens` defaults to 1024 because a reasoning model spends this same
budget on its thinking phase before it writes anything: at the old ceiling of
96 such a model emits only reasoning, returns empty `content`, and scores as a
parse failure that says nothing about its grading ability. It is a ceiling
rather than a target, so non-reasoning candidates are unaffected — they stop
at their own end-of-turn. Reports carry `reasoning_tokens` per case (a subset
of `generated_tokens`) so you can see where the budget actually went, and an
empty response is reported as such instead of as malformed JSON.

## What the reports contain

Exact accuracy, false accepts, false rejects, wrong and missed `uncertain`,
parse failures, per-case latency and generated-token throughput. Token counts
come from the response's `usage` block, so they are whatever the server
reports.

Model file size and process RSS are **gone** — the model no longer runs in this
process, so there is nothing honest to measure. `compare.py` prints `n/a` in
those columns rather than a fabricated zero, and still reads the older
in-process report shape, so a stored baseline lines up against a fresh run.

Compare completed runs:

```sh
python3 compare.py ../../target/grader-eval/results/*.json
```

Full JSON reports live under `../../target/grader-eval/` and are deliberately
not committed. The durable inputs are `cases.jsonl` and the prompt and verdict
schema in `src/main.rs`. Whenever any of those changes, rerun every candidate
before comparing rankings.

`RESULTS.md` carries two baselines: the current API one, and the older
in-process llama.cpp sweep kept for its accuracy figures. **They are not one
ranking** — a grammar-constrained local decode and a server-side JSON schema
are different constraints, and only the older table's latency, throughput,
size and RSS columns ever meant anything, on hardware this harness no longer
touches.
