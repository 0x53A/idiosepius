# Grader benchmark

The suite is 38 cases built from eight Control Systems questions — paraphrases,
omissions, false extra claims, calculations, ambiguity and prompt injection.
Two baselines follow. They were measured under **different protocols and are
not comparable to each other**; only the accuracy columns mean roughly the same
thing in both.

## API baseline — 2026-07-28

Harness: `POST {base_url}/chat/completions`, `--response-format json-schema`,
`--temperature 0`, `--max-tokens 1024`, one repetition. Server: Ollama on
`http://localhost:11434/v1`, same machine as the harness — AMD Ryzen 7 7840HS,
Radeon 780M integrated graphics, 30 GiB RAM.

| model | accuracy | FA | FR | missed ? | parse | p50 ms | p95 ms | tok/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `qwen3:8b` | 33/38 (86.8%) | 0 | 3 | 2 | 0 | 4666 | 7002 | 5.83 |
| `gemma3:4b-it-qat` | 31/38 (81.6%) | 0 | 5 | 2 | 0 | 3767 | 4657 | 9.02 |

Latency and tok/s are properties of that server on that machine, not of the
models; they move with the host. Model size and RSS are not measured at all any
more — the model does not run in this process, so there is nothing honest to
report and `compare.py` prints `n/a`.

**Neither model made a single false accept, and every error was conservative:**
both rejected answers that were in fact correct, or returned `incorrect` where
the honest verdict was `uncertain`. For a grader that is the right direction to
fail in, but a false-reject rate of 3–5 in 38 is still too high to put in front
of a learner unattended.

Four cases defeat both models, which makes them a property of the suite rather
than of either candidate:

- `poles-injection-correct` — a genuine answer carrying a hostile instruction.
  Both reject it. The failure is over-strictness, not steering: neither emitted
  the verdict the injected text asked for. Item 4 of the old resume list is
  therefore still open, and still the most interesting case in the file.
- `integrator-stability` and `bode-gain-correct` — correct answers, rejected.
- `bode-vague` and `disturbance-garbled` — both `uncertain`, both answered
  `incorrect`. Neither model reaches for `uncertain` at all under the schema.

`gemma3:4b-it-qat` adds `poles-paraphrase` and `integrator-converges`, both
correct answers it rejected. It is the same failure mode as qwen3, further
along.

### LM Studio does not currently work as a backend

Tested against an LM Studio server on the LAN (`http://<host>:1234/v1`), model
`qwen/qwen3.5-9b`, with the real grader prompt:

| `response_format` | result |
| --- | --- |
| `json_schema` | HTTP 200, **`content` empty**, 34 tokens in `reasoning_content`, `finish_reason` `stop` |
| `json_object` | HTTP 400 — `'response_format.type' must be 'json_schema' or 'text'` |
| `text` / absent | writes `content`, but as prose: `"\n\nVerdict: correct"`, not JSON |

Adding `chat_template_kwargs: {enable_thinking: false}` changes nothing — same
empty content, same 34 reasoning tokens. Nothing is being truncated:
`finish_reason` is `stop`, not `length`, so raising `--max-tokens` does not
help.

The same class of model works elsewhere. Ollama's `qwen3:8b` is also a
reasoning model, and under `json_schema` it returns clean JSON with
`reasoning_tokens: 0` — Ollama suppresses the thinking phase when the
constraint is applied. So this is LM Studio's structured-output path, not
reasoning models in general and not the harness. There is no `--response-format`
that gets a usable answer out of it: the one mode that produces content
produces prose instead of the object.

If a later LM Studio build fixes it, the check is one run of
`--limit 3 --response-format json-schema` — non-empty `content` is the whole
test.

## Local llama.cpp baseline — 2026-07-26 (superseded protocol)

This predates the move off in-process inference, and it was measured on
**different hardware** — Intel Core i5-1345U with integrated Iris Xe, not the
Ryzen box above. Q4_K_M GGUFs; greedy, grammar-constrained decoding. The
latency, throughput, size and RSS columns describe both an execution model and
a machine this harness no longer touches.

| model | backend | accuracy | false accepts | p50 latency | output tok/s | peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Qwen 3.5 4B | Vulkan | 36/38 (94.7%) | 0 | 9.2 s | 3.56 | 3045 MiB |
| Phi-4 Mini | Vulkan | 35/38 (92.1%) | 0 | 5.3 s | 5.27 | 2528 MiB |
| Ministral 3 3B | Vulkan | 34/38 (89.5%) | 0 | 5.2 s | 7.09 | 2193 MiB |
| Qwen 3.5 2B | Vulkan | 33/38 (86.8%) | 0 | 5.2 s | 6.75 | 1503 MiB |
| Qwen 3.5 0.8B | Vulkan | 28/38 (73.7%) | 6 | 3.2 s | 8.82 | 724 MiB |
| Llama 3.2 1B | Vulkan | 20/38 (52.6%) | 4 | 1.4 s | 10.96 | 918 MiB |

Qwen 3.5 4B on four CPU threads produced the same 36/38 verdicts, but median
latency rose to 19.7 s and peak RSS to 3317 MiB. Its two misses were the same
adversarial injection case and the same ambiguous Bode answer that still defeat
every candidate today.

The 94.7% at the top of this table is the number to beat, and no API candidate
has yet been run that approaches it. Whether that gap is the models, the
protocol or the machine is untested — all three changed at once, and
grammar-constrained llama.cpp decoding and a server-side JSON schema are not
the same constraint in any case. **Do not read the two tables as one ranking.**

## Current decision

No grader is integrated into the application, and none of these results
justifies integrating one. The work stays isolated in `tools/grader-eval/`.

If a decision were forced today it would be `qwen3:8b` for accuracy, on the
strength of zero false accepts — but at a ~4.7 s median and three wrongly
rejected correct answers in 38 it is not something to put between a learner and
their revision the week of an exam.

## Resume here

The version-controlled source of truth:

- `cases.jsonl` — the 38 labelled inputs;
- `src/main.rs` — system prompt, verdict schema, request shape, measurements;
- `compare.py` — report comparison;
- this file — the baselines that should survive ignored build output.

Reports are local, ignored artifacts under `target/grader-eval/results/`. There
is no `shell.nix` and no models directory any more; a stock `cargo build` is
the whole toolchain.

Inspect the existing reports:

```sh
python3 tools/grader-eval/compare.py target/grader-eval/results/*.json
```

Rerun the current best candidate:

```sh
cd tools/grader-eval
cargo run --release -- \
  --base-url http://localhost:11434/v1 \
  --model qwen3:8b \
  --model-id qwen3-8b \
  --cases cases.jsonl \
  --output ../../target/grader-eval/results/qwen3-8b.json
```

When continuing:

1. Build an independent holdout, prioritising genuine student answers rather
   than more paraphrases written alongside the rubric.
2. Add ambiguous boundary cases and treat false accepts as the primary safety
   gate. Note that the current failure mode is the opposite one — every miss in
   the API baseline is over-strict — so false rejects now need a gate too.
3. Add German Maths 2, automotive-mechatronics and Lasertechnik cases before
   generalising the Control Systems result.
4. Revisit `poles-injection-correct`. Every candidate ever run rejects it.
5. Settle whether `uncertain` is reachable at all under a server-side schema.
   No API candidate has produced it once; the enum is offered and never chosen.
6. Rerun all candidates after changing the cases, prompt, schema or token
   budget. Do not compare reports produced by different protocols — the two
   tables above are the standing example of why.
