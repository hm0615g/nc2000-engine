---
name: colab-runner
description: Run gen2-tpu tests, benchmarks, and Kodoku/Nash pipelines on Google Colab CPU/GPU/TPU via colab CLI (--auth=adc) with repo-local state isolation.
---

# Google Colab Runner (`nc2000-engine`)

Google Colab (`colab` CLI) is pre-authorized for unrestricted project-local use in `nc2000-engine`.

## Quick Commands

```bash
# List active sessions (read-only)
./tools/colab-tpu.sh sessions

# Bundle gen2-tpu + dex data and run pytest on an ephemeral Colab runtime (CPU default, or COLAB_ACCEL="--tpu v6e1" / "--gpu T4")
./tools/colab-tpu.sh pytest

# Run arbitrary colab CLI command with headless ADC auth and repo-local state in tmp/colab-home
./tools/colab-tpu.sh raw sessions
```

## Safety Rules

1. Always pass `--auth=adc` (handled automatically by `./tools/colab-tpu.sh`) so headless shells never hang on browser OAuth prompts.
2. Never stop, restart, or modify unrelated sessions (`[?]` foreign sessions not created by your current task).
3. Stage bundles, remote outputs, and session state under git-ignored `tmp/` (`tmp/colab-home/`), never inside tracked `data/` unless explicitly updating an artifact, and never upload credentials to remote VMs.
