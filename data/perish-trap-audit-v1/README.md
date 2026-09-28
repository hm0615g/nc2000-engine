# Perish trapping: a protocol reconstruction defect

The importer restored a public trapping volatile but left the opponent's cached
`trapped` flag false. At the first search decision, the opponent could therefore
switch out of Mean Look, Spider Web, Wrap or Fire Spin illegally. The engine
recomputes that flag at the next turn boundary, so this defect affected the
reconstructed root even though later simulated turns could enforce trapping.

The fix runs the engine's `TrapPokemon` event for the opposing active after
restoring effects and refreshing masks, at move requests. Our own flag still
comes from the authoritative request. No selection policy, rollout policy,
evaluation weight or budget changes.

## Product scope

This fixes `ProtocolAgent` / `ProtocolSearcher`: Showdown integration, imported
positions and replay diagnostics. The browser game's `BlindSearcher` and native
`BlindAgent` / `OpenAgent` start from an existing engine battle and do not use this
importer. Their search behavior is unchanged. An ordinary native arena comparison
would not exercise this fix and cannot serve as its strength gate.

This is **not a general solution to combo play or a completed win-rate
non-inferiority experiment**. In particular, the defensive turn-28 action remains
unchanged. Apparent failures found only in an imported position must first be
checked for this reconstruction artifact before attributing them to the browser
bot.

## Mechanical proof

`tests/import.rs::imported_trapping_preserves_both_sides_switch_legality` starts
real engine battles, feeds each player's visible protocol to a fresh importer,
and compares the available switch destinations for both players. Before the fix,
the first Mean Look case failed: the true victim had no switch choices; the
imported opponent could switch to Skarmory or Smeargle.

After the fix, all four trapping moves pass before trapping, while trapped, after
the source switches, at forced replacement after source fainting, and after the
replacement. Wrap and Fire Spin also pass after natural expiration. The test
passes with debug event-mask assertions enabled and in release mode.

Other checks:

- Importer corpus: 60 fixtures, both information modes, 8,236 decisions and
  position round trips, zero public-field mismatches or illegal recorded choices.
  The existing 22 diagnostic volatile-set differences remain; this does not
  claim every reconstructed field is exact.
- `import`, `position`, `blind`, `report_4296`: 18 tests passed.
- Engine `verdicts_engine_rules`: 18 tests passed.
- `cargo check --workspace --all-targets`: passed, with existing warnings.

## Search observations

These are frozen **reconstructions** of battle 4069, not a replay of a saved live
bot RNG. Side 0 was the human trapper; placing a bot there is a counterfactual.
The side-0 sets include two `cand-fill` entries. Own sets can use later evidence
to reconstruct private information; no future opponent moves are injected.
The corresponding `.provenance.json` files retain those limitations. Own selected
parties are known in these late positions; the early-turn imputed-party artifact
is excluded.

At turn 27, Mean Look has already trapped Snorlax. Before the fix the simulated
Snorlax's root `trapped` flag is false; after the fix it is true. The decisions
below use product profiles from `data/search-profiles.json`:

| Cohort | Before | After |
|---|---|---|
| Blind 27k, discovery seeds 81001–81004 | Mean Look 4/4 | Perish Song 4/4 |
| Blind 27k, held-out seeds 82001–82008 | Mean Look 4, Rest 3, Perish Song 1 | Perish Song 8/8 |
| Open 30k, held-out seeds 82001–82008 | Mean Look 1, Perish Song 7 | Perish Song 8/8 |

The Open cohort pins a **reconstructed defender sheet**, not an original submitted
sheet. Its original Mint Berry and Miracle Berry are restored in the sheet;
their consumed state remains in the position. It tests the Open configuration
under those explicit inputs, not the historical information set.

Two unaffected controls use seeds 81001–81004: Blind turn 28 of this battle and
Open turn 11 of report 4296. All action visits, means and aggregated observed
events are exactly equal before/after. Turn 30 is retained as an unresolved
expiry position: the fix does not consistently make the singer switch out.
Neither selecting Perish Song nor increasing its search reward is a measured
terminal win-rate gain.

`results.json` retains every paired root result and hashes of the full scratch
observations. `summarize.py` checks cohort completeness and the two equal-control
claims. The audit independently reproduces each rollout on cloned state/RNG using
the production move picker and checks its final state. It also verifies that
observed versus ordinary search has identical visits, means, root matrices and
node counts, including 100 subsequent iterations. The final instrumentation
additionally checks the battle RNG at every replayed leaf; its held-out seed
82001 reproduces the recorded action and event aggregates exactly.

## Reproduction

```sh
cargo build --release -p nc2000-bot --example perish_audit
target/release/examples/perish_audit \
  --position data/perish-trap-audit-v1/turn-27-offense.json \
  --seed 82001 --seeds 8 --out tmp/perish-audit/t27-holdout-after
target/release/examples/perish_audit \
  --position data/perish-trap-audit-v1/turn-27-offense.json \
  --opponent-team data/perish-trap-audit-v1/reconstructed-defender-team.json \
  --profile open --seed 82001 --seeds 8 --out tmp/perish-audit/t27-open-after
cargo test --release -p nc2000-bot --test import --test position --test blind --test report_4296
cargo test --release -p nc2000-engine --test verdicts_engine_rules
```

For a baseline build in an isolated checkout, keep the audit example and public
`mcts::playout_pick` export, but use `crates/bot/src/import.rs` from `dd77415`.
The public export changes visibility only. `build.json` binds baseline revision,
binary hashes, current source hashes and frozen inputs. Full observation files
remain under `tmp/perish-audit`; regenerating them requires no external log when
using the frozen positions above.
