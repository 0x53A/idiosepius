# Grader benchmark — 2026-07-26

Machine: Intel Core i5-1345U, integrated Iris Xe graphics. Models are Q4_K_M
GGUFs. Decoding is greedy and grammar-constrained. The suite has 38 cases built
from eight Control Systems questions, including paraphrases, omissions, false
extra claims, calculations, ambiguity and prompt injection.

| model | backend | accuracy | false accepts | p50 latency | output tok/s | peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Qwen 3.5 4B | Vulkan | 36/38 (94.7%) | 0 | 9.2 s | 3.56 | 3045 MiB |
| Phi-4 Mini | Vulkan | 35/38 (92.1%) | 0 | 5.3 s | 5.27 | 2528 MiB |
| Ministral 3 3B | Vulkan | 34/38 (89.5%) | 0 | 5.2 s | 7.09 | 2193 MiB |
| Qwen 3.5 2B | Vulkan | 33/38 (86.8%) | 0 | 5.2 s | 6.75 | 1503 MiB |
| Qwen 3.5 0.8B | Vulkan | 28/38 (73.7%) | 6 | 3.2 s | 8.82 | 724 MiB |
| Llama 3.2 1B | Vulkan | 20/38 (52.6%) | 4 | 1.4 s | 10.96 | 918 MiB |

Qwen 3.5 4B on four CPU threads produced the same 36/38 verdicts, but median
latency rose from 9.2 s to 19.7 s and peak RSS from 3045 MiB to 3317 MiB.
Vulkan is therefore the useful backend on this machine.

## Current decision

Phi-4 Mini is the best interactive default in this sweep: it gives up one
correct verdict to Qwen 4B while returning in roughly half the time. Qwen 4B is
the maximum-accuracy choice when a nine-second median response is acceptable.
The 0.8B and 1B models are not safe graders because they accept several
substantively wrong answers.

Qwen 4B's two misses were the deliberately adversarial answer containing a
correct statement plus an instruction to output `incorrect`, and a genuinely
ambiguous Bode answer which it rejected instead of returning `uncertain`.
Phi-4 Mini missed those same situations and was additionally over-strict on a
correct Routh paraphrase.

This is a screening result, not a final quality estimate. The one-case gap
between Qwen 4B and Phi-4 Mini is not meaningful with only 38 synthetic cases
and eight underlying rubrics. Before integrating a grader, the suite needs an
independent holdout of real student wording, more ambiguous answers, and cases
from the German and automotive decks.

## Resume here

No grader has been integrated into the application. The work is isolated in
`tools/grader-eval/`. The version-controlled source of truth is:

- `cases.jsonl` — the 38 labelled inputs;
- `src/main.rs` — system prompt, JSON grammar, inference and measurements;
- `compare.py` — report comparison;
- this file — the baseline decision that should survive ignored build output.

The downloaded GGUFs and detailed reports are local, ignored artifacts:

```text
target/grader-eval/models/
target/grader-eval/results/
```

To inspect the existing reports:

```sh
cd tools/grader-eval
nix-shell
python3 compare.py ../../target/grader-eval/results/*.json
```

To rerun the current interactive choice:

```sh
cargo run --release -- \
  --model ../../target/grader-eval/models/phi4-mini-q4_k_m.gguf \
  --model-id phi-4-mini-instruct-q4_k_m \
  --cases cases.jsonl \
  --output ../../target/grader-eval/results/phi4-mini-vulkan.json \
  --backend vulkan
```

When continuing:

1. Build an independent holdout, prioritising genuine student answers rather
   than more paraphrases written alongside the rubric.
2. Add ambiguous boundary cases and treat false accepts as the primary safety
   gate.
3. Add German Maths 2 and automotive-mechatronics cases before generalising
   the Control Systems result.
4. Revisit the prompt-injection case where a valid answer is accompanied by
   hostile instructions; both leading models currently reject it.
5. Rerun all candidates after changing the cases, prompt, grammar or token
   budget. Do not compare reports produced by different protocols.
