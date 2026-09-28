# Perish deadline and Destiny Bond

Two engine-played tactical fixtures expose an incorrect final-choice exclusion.
They are reduced two-Pokémon battles, not a representative ladder corpus. The
prefix spends PP and changes HP through legal choices; it does not plant the
decision state. The shared fixture is
`crates/bot/tests/support/perish_deadline.rs`.

At the decision, our last Misdreavus faces a trapped Gengar with 29 HP and Perish
count 1. Gengar has a living Splash-only Magikarp behind it. Mean Look has no PP;
Shadow Ball and Psychic KO Gengar. Waiting survives the Perish expiry without
triggering Destiny Bond.

- **Return:** Gengar is faster and can use its last Destiny Bond before an attack.
  Return cannot damage Gengar. The type-immunity no-op rule discards Return.
- **Snore:** Misdreavus is faster, Gengar's Destiny Bond is already active, and its
  PP is exhausted. Awake Snore fails. The awake-Snore no-op rule discards Snore.

The action certificates enumerate every legal opposing root reply and all chance
outcomes, then use `BoundSolver` on the successors. Unknown or truncated cells
retain [0, 1]. Values are expected battle reward (win 1, tie 0.5, loss 0), not
empirical rollout averages. In both cases the waiting action has a certified
security interval [0.9806485497079629, 1], while each attack has [0, 0]. The lower
bound stops short of 1 because the solver's default width tolerance is 0.02.

## Separate engine correction

Before testing the exclusion, `Battle::try_trap` was corrected to call
`run_status_immunity("trapped")`. The old hardcoded Ghost immunity contradicted
the exported Gen 2 dex. This follows upstream
[Pokemon.tryTrap](https://github.com/smogon/pokemon-showdown/blob/master/sim/pokemon.ts)
and the older-generation
[Ghost type chart](https://github.com/smogon/pokemon-showdown/blob/master/data/mods/gen5/typechart.ts).
Commit `8349c26` contains this correction and 13 trapping lifecycle tests.

Every before/after result here includes that engine correction. Thus the paired
comparison isolates the bot mask change; it does not attribute the trapping fix
to the bot improvement. The before build uses bot code from `0ddfd32` plus the
engine correction. The previous experimental countdown rollout policy remains
disabled.

## Paired results

Open-information `BlindSearch` with a pinned sheet, UCB c=1, 30,000 iterations:

| Fixture | Seeds | Before best | After best | Search statistics |
| --- | --- | --- | --- | --- |
| Return | 102001–102004 | Shadow Ball, 4/4 | Return, 4/4 | Identical visits and means |
| Snore | 102001–102004 | Psychic, 4/4 | Snore, 4/4 | Identical visits and means |

Additional seeds 202001–202008 select the waiting action in 8/8 cases for each
fixture. The old search already allocated at least 99.68% of root visits to
waiting; only the final mask suppressed it. These are regression anchors, not a
claim of general combo strength or statistical noninferiority.

The production change bypasses only `noop_reason` when the **opposing active**
has Perish count 1. Self-KO and Sleep Clause exclusions still take precedence.
It does not force a wait, inspect hidden moves, or change tree search, rollout,
evaluation, or RNG consumption. Ordinary positions and other counters retain
the previous exclusion rules.

## Reproduce

```sh
cargo test --release -p nc2000-engine --test ghost_trapping
cargo test --release -p nc2000-bot --test perish_deadline
cargo test --release -p nc2000-bot --example combo_tactics combo_certify::tests
cargo run --release -p nc2000-bot --example combo_tactics -- --proof
cargo run --release -p nc2000-bot --example combo_tactics -- --snore --proof
cargo run --release -p nc2000-bot --example combo_tactics -- --seed 202001 --seeds 8
cargo run --release -p nc2000-bot --example combo_tactics -- --snore --seed 202001 --seeds 8
```

`*-before.jsonl` and `*-after.jsonl` contain the native battle log, certificates,
and all root action statistics. The older before harness labels both fixtures
`bond`; their moves identify the variant. `*-heldout.jsonl` contains the
additional-seed runs. Boundary tests exercise both player perspectives and
separate opponent-active count 1 from own-only, bench-only, absent, and counts
2–3; the same tests retain the unconditional loss exclusions.
