# Agent & Contributor Guide (`nc2000-engine`)

## 1. Fast Build & Test Workflows

- **Rust workspace (fast dev/test profile in `.cargo/config.toml`)**:
  - `cargo t` (alias for `cargo test --workspace --lib --tests`, skipping 0-test doc-test discovery)
  - `cargo test -p nc2000-engine` / `cargo test -p conformance` / `cargo test -p nc2000-bot`
  - CI contract binaries preserved: `crates/engine/tests/sleep_clause.rs`, `crates/conformance/tests/kifu.rs`, `crates/conformance/tests/replay.rs`, `crates/conformance/tests/conformance_suite.rs`, and `crates/bot/tests/bot_suite.rs`.
- **WASM & Web**:
  - `crates/wasm/build.sh nodejs` + `for s in smoke determinism parity kifu fork solver destiny-bond priority-heal residual-wait own-recovery-wait preview-cache; do node "crates/wasm/tests-node/$s.js"; done`
  - `crates/wasm/build.sh && cd web && npm run build && npm run test:worker && npm run test:evaluate-worker && npm run test:policy && npm run test:e2e`
- **TPU (`gen2-tpu`)**:
  - Local 8-core TPU emulation: `uv run --project tpu pytest tpu/tests -q`

## 2. Cryptographic & Artifact Locks

- `data/compiled-preview-v1.json` (checked by `web/vite.config.ts` on `npm run build`) and `crates/bot/build.rs` SHA-256 lock `Cargo.toml`, `Cargo.lock`, `crates/{engine,conformance,bot}/Cargo.toml`, `crates/{engine,bot}/build.rs`, `crates/{engine,conformance,bot}/src/**/*.rs`, `crates/bot/examples/{meta_eco.rs,meta_eco/catalog_preview.rs,endgame_exactness_corpus.rs,anchor_gate.rs}`, and core `data/*.json` files. Do not modify locked files unless intentionally re-baking or updating the allowlist.
- When running `web/tests/compiled-preview-production.mjs` during verification, pass `NC2000_PREVIEW_CHECK_OUT=../tmp/prod-verification.json` to avoid dirtying tracked `data/preview-training-v3/production-verification.json`.

## 3. Standing Google Colab (`colab` CLI) Authorization & Workflow

Google Colab (`colab` CLI) is pre-authorized across this repository (`.claude/settings.json`, `.gemini/settings.json`, `.codex/rules/colab.rules`, `.agents/skills/colab-runner/SKILL.md`, and `tools/colab-tpu.sh`) and may be used freely at any time for TPU/GPU/CPU verification, JAX XLA benchmarks, residual training, and Kodoku / Nash baking.

### Headless Execution (`--auth=adc`)
Always invoke `colab` via `./tools/colab-tpu.sh` or pass `--auth=adc` explicitly so non-interactive shells use Application Default Credentials without browser OAuth prompts:

```bash
# List active sessions (single-line JSON summary; raw output in tmp/colab-home/colab.log)
./tools/colab-tpu.sh sessions

# Build self-contained gen2-tpu bundle (with dex + learnsets) and run pytest on an ephemeral Colab VM
./tools/colab-tpu.sh pytest

# Request a specific Colab accelerator (e.g., TPU v6e1 / v5e1 or GPU T4 / L4 / A100)
COLAB_ACCEL="--tpu v6e1" ./tools/colab-tpu.sh pytest -k "kodoku"

# Arbitrary colab CLI subcommands with repo-local state in tmp/colab-home
./tools/colab-tpu.sh raw run --tpu v6e1 --timeout 600 my_script.py
```

### Colab Safety & Hygiene Rules
1. **Isolated State**: `./tools/colab-tpu.sh` stores session state, bundles, and logs inside git-ignored `tmp/colab-home/` without writing outside the repository.
2. **Respect Foreign Sessions**: Never `stop`, `restart-kernel`, or overwrite sessions marked `[?]` or owned by other tasks; prefer `colab run` (ephemeral session with automatic teardown) or a task-specific `--session <name>`.
3. **Zero Credential Upload**: Never upload ADC/OAuth JSON files or secrets to remote Colab runtimes.
