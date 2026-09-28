# Perish rollout rules: the combo inside the search's own playouts

**Status: SHIPPED.** `RmConfig::rollout_combo` defaults to ON, so every
search entry point uses it: the Web game (`open`), the Showdown ladder client
(`ProtocolSearcher`), the party evaluator (`?evaluate`, blind) and native
`BlindAgent` / `OpenAgent` / `RmAgent`. In arena, census and gauntlet specs
the shipped rules are the default; `-perish_combo` is the pre-change bot and
`perish_escape` runs the escape alone.

Every measurement below was taken before the default flipped. There, arm A
was spelled `…:perish_combo` and the plain spec (arm B) was the pre-change
bot, which is `…:-perish_combo` today.

## Why every earlier Perish fix measured null

Two findings, both from `examples/combo_census.rs`, which replays the arena's
exact schedule (same team draws, battle and agent seeds; its scores equal the
arena's for the same arguments) and reads every decision off the live state.

1. **The rollout could never play the combo, so the search could only see
   it where its own tree reached every step.** The heavy rollout is
   ε-greedy max damage: status moves score 0, so no rollout side ever used
   Mean Look, Perish Song or Roar/Whirlwind, and none ever switched out at
   Perish count 1. Every Perish line therefore scored as both actives
   dying. Singing into a foe free to switch looked like a guaranteed trade,
   Mean Look added nothing, a trapper escaping its own song never existed,
   and a defender could not see the trap coming.
2. **Self-play almost never contains a Perish line.** On the 14 meta-pool
   teams with a Mean Look + Perish Song user, the pre-change `open:3000`
   fielded its trapper in 3 of 200 games, and Perish Song was chosen in 9
   of 400 games (`results/census-escape3k-s1.jsonl`,
   `results/census-combo3k-s1.jsonl`). An A/B arena between two bots
   therefore measures almost nothing about combo play, which is what the
   parked M16c rollout and the `combo-native-v1` escape candidate ran into.

## The rules (`mcts::perish_escape`, `mcts::perish_combo`)

Applied in `mcts::playout_pick` after the ordinary pick has drawn its RNG,
so a rollout where no rule fires keeps its exact stream (unit test
`perish_rules_draw_the_same_rng_as_the_ordinary_pick`). The tree still
searches every legal action; only playouts change.

- **Escape:** at Perish count 1 with a switch legal, switch to the bench mon
  expected to keep the most HP through the foe's best hit.
- **Combo** (implies escape), for a side with a bench, not itself trapped,
  and with no expected KO this turn: Perish Song on a foe that cannot switch
  (trapped or benchless) and is not counting yet; otherwise Mean Look /
  Spider Web if the mon also carries Perish Song (not into a Ghost). A
  trapped mon with 2+ turns left on its count phazes with Roar/Whirlwind.

## Gauntlet: forced exposure, fixed opponent, paired arms

`examples/combo_gauntlet.rs`. Each arm plays the same opponent on identical
team draws, battle seeds and agent seeds; identical arms produce identical
games (null control: 48/48 equal).

- `--mode offense`: the arm's team carries a trapper and leads with it; the
  opponent is a plain agent with a trapper-free team.
- `--mode defense`: the arm's team is trapper-free; the opponent leads its
  trapper and plays the rules above as a fixed script (the human line of
  battle 4069: trap, sing, stall, leave at count 1). This favors the
  candidate, whose rollouts model exactly that opponent.

Combo arm (A) vs pre-change (B), all `open:3000`:

| Run | Games | A | B | A − B (95%, paired) |
|---|---|---|---|---|
| offense, seed 1 | 200 | 0.400 | 0.365 | +0.035 ± 0.086 |
| offense, seed 2 | 800 | 0.403 | 0.320 | **+0.083 ± 0.041** |
| defense, seed 1 | 200 | 0.600 | 0.620 | −0.020 ± 0.090 |
| defense, seed 2 | 800 | 0.647 | 0.610 | +0.037 ± 0.043 |

What changed in play (offense, seed 2, 800 games per arm):

| | pre-change | combo |
|---|---|---|
| Perish Songs chosen | 1,725 | 1,078 |
| …against a foe that could still switch | 1,445 (84%) | 499 (46%) |
| Foe mons that fainted at count 1 | 107 | 286 |
| Own mons that fainted at count 1 | 171 | 81 |

Defense, seed 2: own mons fainted at count 1, 306 → 198.

## Non-regression: ordinary self-play on the full 32-team meta pool

CRN arena (`--crn-seeds`, identical agent seeds per side-swap pair), combo
arm A vs pre-change B at equal budget:

| Agents | Seed | Games | A score (95%) | think ms/move A / B |
|---|---|---|---|---|
| `open:3000` | 11 | 800 | 0.492 ± 0.027 | (census run, not timed) |
| `open:3000` | 12 | 800 | 0.512 ± 0.027 | 88.3 / 87.0 |
| `open:10000` | 13 | 800 | **0.536 ± 0.027** | 350.5 / 346.0 |
| `blind:3000:0.4` | 14 | 800 | 0.525 ± 0.027 | 92.8 / 91.6 |

The two `open:3000` samples pool to 0.502 ± 0.019. Search cost rises about
1.3–1.5%.

Exposure is structural. In the seed-11 run (`combo_census`, same schedule
as the arena), every one of the 120 pairs in which neither team carries
Perish Song was bit-identical (same winner side, same length): the rules
cannot fire without a Perish count or a Perish Song user. All differences
come from the 280 pairs with one, which scored 0.489 ± 0.038.

Behavior change to know about: in full-pool self-play the combo arm fielded
its own trapper in 1 of 800 games against 11 for the pre-change bot, whose
Perish Songs were mostly aimed at foes free to switch.

## Limits

- Nothing here ran at the Web product budget (30k + ponder); the trend from
  3k to 10k is upward.
- Party-evaluator (`?evaluate`) results from before and after this change
  measure different bots and are not comparable; the export's build
  fingerprint tells them apart.

## Files and reproduction

`results/` holds the raw rows: `gauntlet-{off,def}-combo3k-s{1,2}.jsonl`
(one row per scheduled game, both arms), `census-*.jsonl` (per game, arena
schedule) and `arena-full-*.jsonl` (arena artifacts). Arm labels inside
them predate the flip, as described at the top. Their `armed` / `loaded`
tallies also predate the exclusion of the singer's last mon; the first 48
offense seed-2 games rerun from the command below reproduce every outcome
and game length exactly and differ only in those two tallies.

```sh
cargo build --release -p nc2000-bot --example combo_gauntlet --example combo_census --example arena
target/release/examples/combo_gauntlet open:3000 open:3000:-perish_combo --vs open:3000:-perish_combo --mode offense --games 800 --seed 2
target/release/examples/combo_gauntlet open:3000 open:3000:-perish_combo --vs open:3000:-perish_combo --mode defense --games 800 --seed 2
target/release/examples/combo_census open:3000 open:3000:-perish_combo --games 800 --seed 11 --crn-seeds
target/release/examples/arena open:10000 open:10000:-perish_combo --games 800 --seed 13 --crn-seeds --pool meta
COMBO_TRACE=1 target/release/examples/combo_gauntlet open:3000 open:3000:-perish_combo --vs open:3000:-perish_combo --mode offense --games 800 --seed 2 --only 76
```

The measurements used the pre-change bot as the gauntlet opponent, which is
why the reproduction spells it `-perish_combo`.
