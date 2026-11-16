# Search Speed & Belief Inference Handoff

> **Status**:
> - **Track 3** zero-drift speedups (`WordHasher` batching, `CompactBattle` rollouts, `PolyStep` incremental hashing, jump-table callbacks, `RosterCompactCache`) are landed.
> - **Track 2** Imperfect-Information Architecture Upgrades are landed: symmetric hidden runtime timers, live Hidden Power effectiveness narrowing & passive item exclusions, near-pool soft shell repair & joint fallback synthesis, low-discrepancy quantile-coupled world resampling, `LocalPlayer` / `ObservedPosition` boundary unification, zero-prior Gen 2 species supplementation, and off-pool low-Speed / special low-Attack EV/IV spread synthesis.
> - **Retired path**: The earlier Residual NN / branchless JAX-TPU pipeline (`tpu/`, `bot::value`, `fvi_*`) was removed from active code to avoid dual-engine drift and per-iteration latency overhead while preserving historical datasets under [`data/`](../data/).

---

## 1. Track 2 — Off-Pool Belief Inference & Subsampled World Scaling

### 1.1 Architectural Design & Invariants

Imperfect-information error on held-out / off-pool rosters (documented in [`findings/headroom-probe-v1/README.md`](../findings/headroom-probe-v1/README.md)) is the primary ceiling bottleneck compared with open-sheet leaf evaluation.

- **Choice A (Strict pool trust until contradiction & Turn 0 invariance)**: While `Belief::alive` is non-empty, pool filtering and Turn 0 Team Preview sampling stay unchanged. Near-pool soft shell repair, quantile-coupled fallback synthesis, and unappeared-bench loadout diversification activate only after the known pool is contradicted in battle.
- **No world-dependent root exclusions**: Guessed hidden opponent attributes (Speed, Quick Claw, bench sets, HP rolls) cannot prove action dominance; `BlindSearch::use_public_exclusions` restricts root exclusions strictly to turn-order-independent public/own-certain facts via [`crates/bot/src/mask.rs`](../crates/bot/src/mask.rs) (`public_noop_reason`).
- **World sampling budget**: `BlindSearch` defaults to `MAX_SUBSAMPLED_WORLDS = 24` with bounded warmup in [`crates/bot/src/blind.rs`](../crates/bot/src/blind.rs). Probing `with_max_worlds` via [`crates/bot/examples/world_budget_probe.rs`](../crates/bot/examples/world_budget_probe.rs) on off-pool positions showed no clear improvement over `24`: at `27k` (`64` fallback positions, `mean_fallback_cands = 7.75`), top-1 flip rates for `K = 4..192` (`0.1250–0.2188`) stayed below the `K = 24` seed-noise floor (`0.2344`); at `81k` (`48` positions), `K = 48/96` (`0.2708/0.2292`) matched seed noise (`0.2500`); with `BeliefPrior` (`num_strata = 16`, `mean_fallback_cands = 13.83`, `27k`), `K = 8..96` (`0.2813–0.2969`) also stayed below seed noise (`0.3281`); and `27k` paired `both-offpool` duels (`96` games each) scored `0.5208 ± 0.0712` (`K = 64` vs `24`, `ms_ratio = 0.997`) and `0.5000 ± 0.0772` (`K = 4` vs `24`, `ms_ratio = 1.062`). Keep `MAX_SUBSAMPLED_WORLDS = 24` unless `FallbackCandPool` diversity or root resolution changes substantially.

### 1.2 Implementation & Probe Pointers

- **Symmetric hidden timers**: [`crates/bot/src/player_state.rs`](../crates/bot/src/player_state.rs), [`crates/bot/src/import.rs`](../crates/bot/src/import.rs), [`crates/bot/src/blind.rs`](../crates/bot/src/blind.rs)
- **Live Hidden Power narrowing & deductive item exclusions**: [`crates/bot/src/hp_evidence.rs`](../crates/bot/src/hp_evidence.rs), [`crates/bot/src/observe.rs`](../crates/bot/src/observe.rs), [`crates/bot/src/position.rs`](../crates/bot/src/position.rs), [`web/src/position.ts`](../web/src/position.ts)
- **Known prior, zero-prior species supplement, near-pool repair & off-pool EV/IV spread synthesis**: [`crates/bot/src/belief.rs`](../crates/bot/src/belief.rs), [`crates/bot/src/belief_supplement_v1.json`](../crates/bot/src/belief_supplement_v1.json), [`data/belief-pool-v3/belief-pool.json`](../data/belief-pool-v3/belief-pool.json), [`data/training-v3/supplemental.json`](../data/training-v3/supplemental.json)
- **Non-self-play evaluation & corpus safety audit (`corpus_audit`, `calibration`, `gauntlet`)**: [`crates/bot/examples/belief_probe.rs`](../crates/bot/examples/belief_probe.rs)
- **World-count sensitivity & paired duel probe**: [`crates/bot/examples/world_budget_probe.rs`](../crates/bot/examples/world_budget_probe.rs)

### 1.3 Validation Pitfalls (Do Not Repeat)

- **Avoid self-play-only validation**: [`crates/bot/examples/offpool_fallback_gauntlet.rs`](../crates/bot/examples/offpool_fallback_gauntlet.rs) pits `BlindAgent` vs `BlindAgent`, hiding shared blind spots; pair it with `OpenAgent` oracle controls and held-out human rosters (`belief_probe`).
- **Avoid skewed spectator-reveal move priors**: Observed ladder logs over-represent immediate attack moves and under-count passive/reactive moves; derive carry priors from complete-team sources or unforced distributions rather than raw reveal counts.
- **Avoid hard filtering on Gen 2 damage rolls or turn-order ties**: `[217..255]/255` damage variance, `/100` PS HP rounding, crits, and `Quick Claw` (`60/256`) make hard exclusion brittle.

---

## 2. Track 3 — Zero-Drift Search Speedups

### 2.1 Profiling & Gate Commands (Always Re-Measure on Current HEAD)

Do not rely on historical flamegraphs or snapshot micro-benchmarks. Measure current `HEAD` directly before optimizing:

- **Native micro-benchmarks**: `cargo run --release -p nc2000-bot --example perf_probe`
- **Native flamegraph on recorded games**: `cargo run --release -p nc2000-bot --example profile_blind -- <RECORDED_GAMES> tmp/blind.svg`
- **Portable WASM/browser regression harness**: [`web/tests/portable-perf.mjs`](../web/tests/portable-perf.mjs) (tracks exact decision parity and median/p95 wall-clock across Chrome/Node WASM)

Note that single-turn `profile_mcts` is dominated by cold allocator churn; use multi-turn `profile_blind` over real recorded decisions to inspect live hotspots.

### 2.2 Already Rejected Optimization Approaches (Do Not Repeat)

- **WASM `simd128` (`RUSTFLAGS="-C target-feature=+simd128"`) and manual `u64x2` intrinsics in `WordHasher`**: Failed the 5% median/p95 acceptance gate on both Node and Chromium in `portable-perf.mjs`; keep default WASM flags.
- **`BellmanCache` vectorization (`linear::solve_bellman_into`)**: `BellmanCache` already absorbs almost all terminal/near-terminal evaluations; remaining time in [`crates/bot/src/linear.rs`](../crates/bot/src/linear.rs) is key probing and survival curves.
- **Reverted hashing/table micro-experiments**: Tail-batching `hash_state_impl`, naive PP-slot splits, `PartyMatchCache+Cow<MatchTable>`, cond lookup tables, `-C target-cpu=native`-only search claims, and 1-dispatch-per-iteration GPU offload.
- **Non-goal**: Pervasive per-field dirty bitmasks across `Battle`/`Pokemon` due to desync risk against `ENGINE_FINGERPRINT` ([`crates/engine/build.rs`](../crates/engine/build.rs)).

### 2.3 Fingerprint & Artifact Rules

- **Fingerprint & artifact locks**: Any edit inside `crates/engine/src/` updates `ENGINE_FINGERPRINT` ([`crates/engine/build.rs`](../crates/engine/build.rs)) and requires updating its allowlist plus verifying conformance (`kifu`, `replay`, `conformance_suite`, `sleep_clause`) and `data/compiled-preview-v1.json` lock validity ([`AGENTS.md`](../AGENTS.md)).
