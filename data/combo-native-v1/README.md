# Native Perish Song decision screen

The countdown-escape rollout candidate was **not promoted**. It changes some
defensive root choices, but this screen does not establish a win-rate improvement
or general strength noninferiority. Product search, evaluation, and rollout code
are unchanged. The protocol trap reconstruction fix is separately committed at
`b70a041`.

## Positions and measurement

`combo_native` starts real engine battles from existing meta-pool teams. No
protocol importer or hand-planted volatile is involved. Team picks, switches, and
scripted moves run through the engine. Three initial simultaneous switches reveal
both selected teams, preventing uncertainty about the selected bench from
confounding the open-sheet comparison. Fixture `expected` snapshots assert the
active HP, boosts, trap flag, and countdown at measured stages.

The three offensive kits are Misdreavus with Mean Look / Perish Song / Protect
plus Rest, Confuse Ray, or Destiny Bond. Opposing Miltank uses either Curse or
Return throughout the prefix. Return cannot damage Misdreavus, but damages its
switch-ins. The defensive fixture instead leads with Whirlwind Zapdos and uses
Hidden Power Ice during the prefix. All prefixes use engine seed `1,2,3,4`.

Stages 0–4 mean: before Mean Look, after Mean Look, Perish count 3, count 2, count
1. These are constructed scenarios, not historical-game reconstructions or
claims that both players would choose the prefix under competitive play.

Open-sheet probes use the browser's `BlindSearch` with a belief pinned at preview,
30,000 iterations and UCB c=1. Blind probes use the pool belief, 27,000 iterations
and c=0.4. Parameters come from `data/search-profiles.json`.

## Findings

| Probe | Shipped search | Escape candidate |
| --- | --- | --- |
| Trap then song, three kits, seed 91001 | Mean Look then Perish Song in all three | Not screened |
| Unboosted victim, count 1, Rest and Bond kits, four seeds each | Switch in 8/8 | Switch in 8/8 |
| Unboosted victim, count 1, Confuse Ray kit, four seeds | Stay in 4/4 | Stay in 4/4 |
| Curse-boosted victim, count 1, three kits, four seeds each | Stay in 12/12 | Stay in 12/12 |
| Zapdos defense, count 3, four discovery seeds | Whirlwind in 3/4 | Whirlwind in 4/4 |
| Zapdos defense, count 3, eight held-out seeds | Whirlwind in 7/8 | Whirlwind in 8/8 |
| Zapdos defense, count 2, four seeds | Whirlwind in 4/4 | Whirlwind in 4/4 |
| Blind Zapdos defense, counts 3 and 2, four seeds each | Whirlwind in 8/8 | Not screened |

Staying at count 1 is not, by itself, a proven error: preserving the singer costs
a switch-in hit, and the double KO can lead to a favorable remaining matchup.
The candidate also changes which staying move is chosen; that is not counted as
a tactical improvement.

`--trace` records tree choices and replays each rollout with the production picker.
Every replay asserts the final state hash and battle RNG, and the complete traced
search must match an ordinary search's root visits, means, and best action. In the
Zapdos count-3 trace, the defender switches on only 289 of 4,030 rollout decisions
where its Perish count is 1 and switching is legal. These are correlated search
events, not independent games or a strength estimate.

## Terminal counterfactuals

Each alternative forces only the first action, then two fresh shipped OpenAgents
play to a real engine outcome. Agents initialize their beliefs at preview. The
paired alternatives share battle and agent seeds; subsequent random consumption
can diverge. The continuation policy is a reference opponent, not an optimal-play
oracle. Eight seeds per comparison are too few to establish noninferiority.

| Position / continuation budget | Forced action | Wins / games |
| --- | --- | --- |
| Confuse Ray kit, unboosted count 1 / 3,000 | Confuse Ray | 8/8 |
| Same | Switch 2 | 7/8 |
| Same | Switch 3 | 8/8 |
| Zapdos count 3 / 3,000 | Thunderbolt | 4/8 |
| Same | Whirlwind | 5/8 |
| Zapdos count 3 / 10,000, separate seed set | Thunderbolt | 4/8 |
| Same | Whirlwind | 4/8 |

The last comparison has identical paired outcomes in all eight games. Because
both budget and seeds differ from the discovery comparison, this does not isolate
a budget effect. Neither comparison justifies promoting the rollout change.
The instrumented rerun of seed 97001 shows the source choosing Protect against
both forced actions: neither attack nor Whirlwind lands. Its scores and game
lengths reproduce the saved continuation results. Choosing Whirlwind one turn
earlier is therefore not itself evidence of a better defense.

## Candidate and controls

The experimental `--perish-escape` path exists only in the example. In a rollout,
after consuming the normal pick's RNG draws, it replaces a count-1 choice with a
legal switch maximizing remaining HP fraction minus the foe's estimated incoming
hit. The tree still searches every legal action. This is a heuristic: switch-in
damage, sacrifice, pursuit, and retained boosts can make unconditional escape
undesirable. No root action is forcibly removed.

The no-song control selects six Pokémon with no Perish Song. Both sides and two
seeds give four exact matches of root actions, visits, means, masks, and best
choice, with zero escape overrides. This validates the dormant path on those
positions; it is not a broad strength gate. The native search traces also replay
exactly. `cargo check --workspace --all-targets` and the example's rustfmt check
pass. No product policy was changed to obtain these results.

## Reproduction

```sh
cargo build --release -p nc2000-bot --example combo_native
target/release/examples/combo_native --fixture data/combo-native-v1/fixtures/rest-phaze.json --stages 2,3 --sides 1 --seed 93001 --seeds 4
target/release/examples/combo_native --fixture data/combo-native-v1/fixtures/rest-phaze.json --stages 2,3 --sides 1 --seed 93001 --seeds 4 --perish-escape
target/release/examples/combo_native --fixture data/combo-native-v1/fixtures/rest-phaze.json --stages 2 --sides 1 --seed 93002 --trace
target/release/examples/combo_native --fixture data/combo-native-v1/fixtures/rest-phaze.json --stages 2 --sides 1 --seed 97001 --continuations 8 --follow-iters 10000 --follow-actions 'move thunderbolt,move whirlwind'
python3 data/combo-native-v1/summarize.py
```

Raw results omit some metadata fields added during instrumentation; all use the
same search and candidate algorithms. The final example reproduces the saved
trace's action statistics and events exactly. Timings came from concurrent runs
and are not performance benchmarks. `manifest.json` records final source hashes.
