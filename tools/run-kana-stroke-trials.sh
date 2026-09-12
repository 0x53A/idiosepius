#!/usr/bin/env bash
# Run the two frozen research candidates sequentially; never installs weights.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "${1:-}" == confirmation ]]; then
  for stage in reproduction confirmation; do
    if [[ "$stage" == reproduction ]]; then
      initialization=fresh-clean-residual-v1
      protocol=trial-v1.json
    else
      initialization=fresh-clean-residual-seed2-v1
      protocol=confirmation-v1.json
    fi
    uv run --python 3.12 --with torch==2.14.0+cpu --with numpy==2.5.2 \
      --index https://download.pytorch.org/whl/cpu \
      python tools/train-kana-stroke-trial.py \
      --prepared kana-artifacts/kana-training/stroke-trial-prepared-v1 \
      --initialize-from "kana-artifacts/kana-training/$initialization" \
      --protocol "tools/fixtures/kana-beginner/$protocol" \
      --candidate residual --threads 6 \
      --run "kana-artifacts/kana-training/stroke-finetune-residual-v1-$stage" \
      > "$stage.log" 2>&1
  done
  exit 0
fi
for candidate in plain residual; do
  if [[ "$candidate" == plain ]]; then
    initialization=cnn-64-etl7-deep-v2
  else
    initialization=fresh-clean-residual-v1
  fi
  uv run --python 3.12 --with torch==2.14.0+cpu --with numpy==2.5.2 \
    --index https://download.pytorch.org/whl/cpu \
    python tools/train-kana-stroke-trial.py \
    --prepared kana-artifacts/kana-training/stroke-trial-prepared-v1 \
    --initialize-from "kana-artifacts/kana-training/$initialization" \
    --protocol tools/fixtures/kana-beginner/trial-v1.json \
    --candidate "$candidate" --threads 6 \
    --run "kana-artifacts/kana-training/stroke-finetune-$candidate-v1" \
    > "$candidate.log" 2>&1
done
