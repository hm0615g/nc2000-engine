# Agent & Contributor Guide (`nc2000-engine`)

## 1. Fast Build & Test Workflows

- **Rust workspace (fast dev/test profile in `.cargo/config.toml`)**:
  - `cargo t` (alias for `cargo test --workspace --lib --tests`, skipping 0-test doc-test discovery)
  - `cargo test -p nc2000-engine` / `cargo test -p conformance` / `cargo test -p nc2000-bot`
  - CI contract binaries preserved: `crates/engine/tests/sleep_clause.rs`, `crates/conformance/tests/kifu.rs`, `crates/conformance/tests/replay.rs`, `crates/conformance/tests/conformance_suite.rs`, and `crates/bot/tests/bot_suite.rs`.
- **WASM & Web**:
  - `crates/wasm/build.sh nodejs` + `for s in smoke determinism parity kifu fork solver endgame destiny-bond priority-heal residual-wait own-recovery-wait preview-cache; do node "crates/wasm/tests-node/$s.js"; done`
  - `crates/wasm/build.sh && cd web && npm run build && npm run test:worker && npm run test:evaluate-worker && npm run test:policy && npm run test:e2e`

## 2. Cryptographic & Artifact Locks

- `data/compiled-preview-v1.json` (verified by `web/vite.config.ts` on `npm run build`) embeds SHA-256 locks over `Cargo.toml`, `Cargo.lock`, `crates/{engine,conformance,bot}/Cargo.toml`, `crates/{engine,bot}/build.rs`, `crates/{engine,conformance,bot}/src/**/*.rs`, `crates/bot/examples/{meta_eco.rs,meta_eco/catalog_preview.rs}`, and core `data/*.json` files (plus semantic prior checks on `data/belief-pool-v3/belief-pool.json`). `crates/bot/build.rs` computes `M17E_SOLVER_BUILD_FINGERPRINT` over `Cargo.toml`, `Cargo.lock`, `crates/{bot,engine,conformance}/Cargo.toml`, `crates/bot/build.rs`, `crates/{bot,engine,conformance}/src/**/*.rs`, and `crates/bot/examples/{endgame_exactness_corpus,anchor_gate}.rs`, and `crates/engine/build.rs` hashes `crates/engine/src/**/*.rs` into `ENGINE_FINGERPRINT`. Do not modify locked files unless intentionally re-baking or updating the allowlist.
- When running `web/tests/compiled-preview-production.mjs` during verification, pass `NC2000_PREVIEW_CHECK_OUT=../tmp/prod-verification.json` to avoid dirtying tracked `data/preview-training-v3/production-verification.json`.

## 3. Research & Strength Roadmap

- Off-pool belief inference & zero-drift search diagnostics: see [`docs/SEARCH-AND-BELIEF-HANDOFF.md`](docs/SEARCH-AND-BELIEF-HANDOFF.md).
