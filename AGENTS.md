# Agent & Contributor Guide (`nc2000-engine`)

## 1. Fast Build & Test Workflows

- **Rust workspace (fast dev/test profile in `.cargo/config.toml`)**:
  - `cargo t` (alias for `cargo test --workspace --lib --tests`, skipping 0-test doc-test discovery)
  - `cargo test -p nc2000-engine` / `cargo test -p conformance` / `cargo test -p nc2000-bot`
  - CI contract binaries preserved: `crates/engine/tests/sleep_clause.rs`, `crates/conformance/tests/kifu.rs`, `crates/conformance/tests/replay.rs`, `crates/conformance/tests/conformance_suite.rs`, and `crates/bot/tests/bot_suite.rs`.
- **WASM & Web**:
  - `crates/wasm/build.sh nodejs` + `for s in smoke determinism parity kifu fork solver endgame destiny-bond priority-heal residual-wait own-recovery-wait preview-cache; do node "crates/wasm/tests-node/$s.js"; done`
  - `crates/wasm/build.sh && cd web && npm run build && npm run test:worker && npm run test:evaluate-worker && npm run test:policy && npm run test:e2e`

## 2. Epoch & Cache Versioning

- **2-axis epoch tracking**:
  - `ENGINE_EPOCH = 1` in `crates/engine/src/replay.rs` governs compact replay (`NC2-…`) and kifu structural compatibility; bump only on breaking mechanics or replay format changes.
  - `POLICY_EPOCH = "policy-v1"` in `crates/bot/src/compiled_preview.rs` governs compiled preview table compatibility (`data/compiled-preview-v1.json`). `web/vite.config.ts` and `web/src/compiled-preview.ts` validate `POLICY_EPOCH`, `dexSha256`, `catalogSha256`, and `priorSha256` / `priorDependencies`, treating mismatches as a soft-cache miss that falls back to live search rather than failing builds.
- `tools/bundle-core-sdk.js` writes `dist/nc2000-core-sdk/engine-manifest.json` (untracked); `web/tests/compiled-preview-coverage.mjs` and `web/tests/compiled-preview-production.mjs` write verification outputs under `../target/` by default.

## 3. Research & Strength Roadmap

- Playing-strength architecture, measured-null dead ends, and open `81k` frontiers: see [`docs/BOT-STRENGTH-HANDOFF.md`](docs/BOT-STRENGTH-HANDOFF.md).
- Do NOT consult gitignored local scratch/archive directories (`findings/`, `tmp/`, `.git/sota/`) for current architecture or open bottlenecks; verify claims directly against [`docs/BOT-STRENGTH-HANDOFF.md`](docs/BOT-STRENGTH-HANDOFF.md) and live code under `crates/`.

## 4. Compute Policy

- **Local-only compute**: Do NOT offload `nc2000-engine` builds, self-play, or search sweeps to Google Colab (nor retired `cx` Spot-VM / TPU paths). Colab CPU offloading is not cost-effective relative to local native execution (~16% of local Apple M6 CS-PGO throughput and prone to CLI/websocket hangs).
- Run heavy sweeps locally with native CS-PGO binaries (`tools/build-native-pgo.sh`, `<= 10–11` threads) and resumable chunk checkpoints.
