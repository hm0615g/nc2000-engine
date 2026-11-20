# Community-Data Priors (`BeliefPrior` & `PickPrior`) — Design & Contract

> **Status**: Implemented as opt-in, inert-by-default priors (`BeliefPrior` in [`crates/bot/src/prior.rs`](../crates/bot/src/prior.rs) and `PickPrior` in [`crates/bot/src/pick_prior.rs`](../crates/bot/src/pick_prior.rs)) for hidden-team ladder / protocol play and setup-panel overrides. Do **not** place a file at `data/belief-prior-v0.json`; normal Web doors remain governed by [`data/belief-pool-v3/belief-pool.json`](../data/belief-pool-v3/belief-pool.json) and [`crates/bot/src/belief.rs`](../crates/bot/src/belief.rs) unless explicitly configured.

---

## 1. Purpose & Architecture

`BeliefPrior` and `PickPrior` separate declarative community frequency tables from certified engine/search code:

- **Code = certified invariants**: Revealed moves, species, levels, genders, HP buckets, PP, status, `|teamsize|`, and legality always override any installed prior. A bad table can only worsen unrevealed-slot or unrevealed-pick guesses and must never cause a tactical blunder against revealed facts or a runtime crash.
- **`BeliefPrior` (`prior.rs`)**: Opt-in per-species marginal weights (`moves`, `items`, `lead`) applied once when building off-pool `FallbackCandPool` loadouts and lead sampling (without double-counting in `LoadoutDraw`).
- **`PickPrior` (`pick_prior.rs`)**: Opt-in 3-of-6 opponent selection frequency table (`nc2000-pick-prior-v1`) conditioned on `(team, enemy_preview, side)` with smoothing and backoff, consumed during Team Preview and hidden bench sampling.
- **Total, malformed-robust parsers**: Invalid or unknown entries clamp/normalize or fall back cleanly without panicking.

---

## 2. `BeliefPrior` Data Format

```json
{
  "format": "nc2000-belief-prior",
  "version": 1,
  "note": "free text; ignored by code",
  "species": {
    "snorlax": {
      "moves": {
        "bodyslam": 0.82,
        "curse": 0.61,
        "rest": 0.58,
        "sleeptalk": 0.55
      },
      "items": { "leftovers": 0.70 },
      "lead": 0.31
    }
  }
}
```

- Keys use lowercase PS IDs (`snorlax`, `bodyslam`).
- `moves` probabilities represent `P(species carries move)` (carry marginals), not raw spectator reveal rates `P(carries) * P(uses | carries)`.
- `items` weights unrevealed held-item synthesis in `FallbackCandPool`; `lead` weights off-pool lead sampling (`fallback_lead_weights` / `sample_preview_prior`).

---

## 3. Code & Tooling Pointers

- **Native parsers & belief integration**: [`crates/bot/src/prior.rs`](../crates/bot/src/prior.rs), [`crates/bot/src/pick_prior.rs`](../crates/bot/src/pick_prior.rs), [`crates/bot/src/belief.rs`](../crates/bot/src/belief.rs), [`crates/bot/src/blind.rs`](../crates/bot/src/blind.rs), [`crates/bot/src/import.rs`](../crates/bot/src/import.rs)
- **WASM & Web wiring**: `setBeliefPrior` / `beliefPriorInfo` and `setPickPrior` / `pickPriorInfo` in [`crates/wasm/src/lib.rs`](../crates/wasm/src/lib.rs) and [`web/src/belief-prior.ts`](../web/src/belief-prior.ts) (`pinOpponent` clears installed priors).
- **Showdown client CLI flags**: `tools/ps-client.js --belief-prior <FILE> --pick-prior <FILE>` (refused when combined with `--mode open` or `--random`).
- **Offline counters & safety gates**:
  - Marginal counter: [`crates/bot/examples/count_belief_prior.rs`](../crates/bot/examples/count_belief_prior.rs)
  - Reveal-dominance gate: [`crates/bot/examples/reveal_dominance_gate.rs`](../crates/bot/examples/reveal_dominance_gate.rs)
