# M16c rollout arm, re-measured: null in self-play at 4x the resolution

**Status: NOT SHIPPED; stays parked.** `RmConfig::rollout_status` and
`RmConfig::rollout_switch` (formerly the single `rollout_m16c`) stay off by
default.

## Question

Cluster 2 of the M16b list says that where humans play a status or setup move
(Curse, Sleep Powder, Double Team, ...), the bot often plays a damaging move
instead. One explanation was the rollout policy. `mcts::greedy_pick` scores
every status move 0, so no rollout side ever sets up. The Perish combo showed
that this kind of rollout blindness can cost strength, and that a rollout rule
can recover it (`data/perish-rollout-v1`: +0.086 under forced exposure).

The rollout rules that address this were already built in 2026-07-21 as the
M16c arm:

- `status_pseudo_score` gives sleep, paralysis, Spikes, heal-below-half and
  Curse/Swords Dance/Amnesia/Belly Drum a pseudo-damage value.
- A bad-matchup voluntary switch fires in the rollout.

The arm was parked as "null", but it was measured at 0.465 ± 0.069 (@300) and
0.510 ± 0.098 (@1000). Those intervals are too wide to call a null. Here it is
re-measured against the current shipped agent (Perish combo on), at product-like
budgets, with both halves also measured separately.

## Exposure is not the problem here

Self-play almost never fields a trapper, so the Perish combo needed a
forced-exposure gauntlet. These rules are different: they fire in ordinary
full-pool play. Under CRN agent seeds, 42–46% of side-swap pairs changed
outcome in every run below, so the arena sees the change directly.

## Results

Full 32-team meta pool, `--crn-seeds`, 800 games per run, arm A vs the shipped
agent B at equal budget:

| A | Seed | A score (95%, pair unit) | Split pairs | think ms/move A / B |
|---|---|---|---|---|
| `open:3000:m16c` | 21 | 0.522 ± 0.032 | 170/400 | 106.8 / 104.6 |
| `open:3000:m16c` | 23 | 0.496 ± 0.033 | 185/400 | 88.8 / 88.3 |
| `open:10000:m16c` | 22 | 0.496 ± 0.032 | 171/400 | 348.2 / 349.0 |
| `open:3000:m16c_status` | 24 | 0.489 ± 0.033 | 181/400 | 88.3 / 87.4 |
| `open:3000:m16c_status` | 25 | 0.497 ± 0.032 | 168/400 | 91.2 / 89.9 |
| `open:3000:m16c_switch` | 26 | 0.466 ± 0.032 | 177/400 | 86.8 / 87.6 |

Pooled:

- Both halves, 3k + 10k (2,400 games): **0.505 ± 0.019**, [0.486, 0.524].
- Status/setup pseudo-scores alone, 3k (1,600 games): **0.493 ± 0.023**,
  [0.470, 0.516].
- Switching alone, 3k (800 games): 0.466 ± 0.032. This is the only interval
  that excludes 0.5, and only by 0.002. It is one seed among six runs, and it
  points the same way as the old 0.490.

## Reading

- Giving rollouts a setup/status policy does not make the bot stronger. The
  upper bound on the gain is about +0.02.
- The M16c conclusion ("the tree, not the rollout tail, owns root values")
  holds at this resolution.
- The Perish combo is the exception, not the pattern. Without the combo
  rules, the rollout scored every Perish line as a mutual KO: its value sign
  was wrong. For setup and status moves, the leaf eval already prices boosts
  and status, so the rollout's zero score only costs resolution the tree
  recovers.
- This is a statement about strength against the same bot. It is not a
  statement about human agreement: cluster 2's "setup where humans set up"
  remains an agreement gap. Agreement and strength are known not to be aligned
  (`docs/SWITCHING-QUESTION-HANDOFF.md`).

## Reproduce

```
cargo build --release -p nc2000-bot --example arena
target/release/examples/arena open:3000:m16c open:3000 --pool meta \
    --games 800 --seed 21 --threads 12 --crn-seeds --jsonl results/arena-open3k-s21.jsonl
# likewise: open:10000:m16c seed 22; open:3000:m16c seed 23;
# open:3000:m16c_status seeds 24, 25; open:3000:m16c_switch seed 26
```

Controls:

- `open:300:m16c` vs itself scores exactly 0.500 with 0/20 split pairs.
- Splitting the flag into two halves is behaviour-preserving:
  `open:300:m16c` and `open:300:m16c_status,m16c_switch` vs `open:300` both
  reproduce 102W 97L 1T on seed 5.
- Runs s21–s23 were built before the split and the rest after, so their
  `fingerprints.build` differ.

Machine: Apple M6, 12 threads.
