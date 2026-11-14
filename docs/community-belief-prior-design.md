# Community-Data Belief Prior — Design & Contract

> **Status**: Implemented as an opt-in, inert-by-default marginal prior
> (`BeliefPrior` in [`crates/bot/src/prior.rs`](../crates/bot/src/prior.rs))
> for hidden-team ladder / protocol play. Do **not** place a file at
> `data/belief-prior-v0.json`; normal Web doors remain governed exclusively by
> [`data/belief-pool-v3/belief-pool.json`](../data/belief-pool-v3/belief-pool.json)
> and [`crates/bot/src/belief.rs`](../crates/bot/src/belief.rs).

---

## 1. Purpose & Architecture

`BeliefPrior` separates declarative community move marginals from certified
engine/search code:

- **Code = certified invariants**: Revealed moves, species, levels, genders, HP,
  PP, status, and legality always override the prior. A bad data edit can only
  worsen unrevealed-slot guesses (Class B suboptimality) and must never cause a
  tactical blunder against revealed facts (Class A) or a runtime crash.
- **Data = opt-in per-`(species, move)` marginal weights**: Sampled via ISMCTS
  determinization in `Belief` (`crates/bot/src/belief.rs`) when filling unseen
  slots after full-pool fallback. No conditional cross-move correlations are
  stored.
- **Total, malformed-robust parser**: `BeliefPrior::parse` clamps/normalizes
  weights, ignores unknown keys, records structured `warnings`, and falls back
  cleanly instead of panicking or throwing.

---

## 2. Data Format

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
- `moves` probabilities represent `P(species carries move)` (carry marginals),
  not raw spectator reveal rates `P(carries) * P(uses | carries)`.
- `items` and `lead` are parsed for forward compatibility.

---

## 3. Code & Tooling Pointers

- **Prior parser & sampler integration**: [`crates/bot/src/prior.rs`](../crates/bot/src/prior.rs),
  [`crates/bot/src/belief.rs`](../crates/bot/src/belief.rs),
  [`crates/bot/src/corpus.rs`](../crates/bot/src/corpus.rs)
- **WASM / protocol ladder wiring**: `ProtocolSearcher.setBeliefPrior` and
  `ProtocolSearcher.beliefPriorInfo` in [`crates/wasm/src/lib.rs`](../crates/wasm/src/lib.rs)
  (`pinOpponent` explicitly clears any installed prior to an empty table).
- **Showdown client CLI flag**: `tools/ps-client.js --belief-prior <FILE>`
  (probes the table on startup and logs warnings/species counts; refused when
  combined with `--mode open` or `--random`).
- **Offline counters & safety gates**:
  - Marginal counter (`moves` from complete sets vs spectator reveals):
    [`crates/bot/examples/count_belief_prior.rs`](../crates/bot/examples/count_belief_prior.rs)
  - Reveal-dominance gate (verifies hand-edited tables never erase revealed
    moves or violate legality):
    [`crates/bot/examples/reveal_dominance_gate.rs`](../crates/bot/examples/reveal_dominance_gate.rs)
