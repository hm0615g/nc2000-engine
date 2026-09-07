# OpenSheet exploration coefficient 0.4

**Lowering c mitigates the measured exploration behavior, but is not supported as an OpenSheet profile change.** On 32 paired new T11 seeds, Hidden Power selections fall from 32 to 7, mostly replaced by Ampharos switches; Earthquake is selected once. On the paired T26 regression check, Thunderbolt falls from 16/16 to 5/16. The profiles remain blind c=0.4/27,000 and open c=1/30,000.

The user proposes carrying the existing blind coefficient (0.4) into OpenSheet to reduce exploratory mistakes. This experiment compares c=1 with c=0.4 in the ordinary single-tree OpenSheet search, keeping the total budget at 30,000 and pinning the actual opponent sheet. It does not use the rejected ensemble or frozen-policy correction. The coefficient is specified before looking at results; it is not selected by a parameter sweep.

The mechanistic prediction is a reduction in exploration-bonus-driven selections below Hidden Power, measured with the existing T11 trace classifier: the first observed tree decision with full-HP Starmie against living Marowak, conditioned on root Encore as well as over all root replies. A selected visited action below the node's highest empirical mean counts as exploratory; choosing an unvisited action is reported separately. These are observations within an adaptive search, not independent games or a proof that every non-Surf choice is a mistake.

Previously studied seeds are 61001, 62001–62004 and 63001–63008. Five detailed c=0.4 traces will be compared with the archived c=1 traces. A separate 32-seed confirmation uses 66001–66032 at both coefficients. If the T11 result supports mitigation, T26 receives a paired 16-seed check using 66001–66016. Choosing Earthquake is a targeted diagnostic supported by earlier continuation experiments, not proof of globally optimal play or a substitute for a full-game strength comparison.

Changing c scales the UCB bonus at fixed counts; it does not specify a fixed exploration probability. Both sides, the root allocation, node coverage and later estimates may change. The production profiles remain unchanged while this hypothesis is tested.

## Results

At the exact T13 node from the causal investigation, holding counts and empirical means fixed gives:

| Opponent action | UCB at c=1 | UCB at c=0.4 |
|---|---:|---:|
| Surf | 0.790555 | 0.492693 |
| Psychic | 0.796425 | 0.468196 |

Surf becomes the highest-scoring action over the entire legal list. This is a direct intervention on the coefficient at the recorded node; the c=0.4 search from T11 follows a different trajectory and need not reach the same node with these statistics.

Five full traces at each coefficient provide the following comparison, conditioned on root Hidden Power into Encore and the first observed tree decision with full-HP Starmie against living Marowak:

| Observed behavior | c=1 | c=0.4 |
|---|---:|---:|
| Encounters | 17,078 | 6,338 |
| Select a visited action below the maximum empirical mean | 5,137 / 30.08% | 767 / 12.10% |
| Select an unvisited action | 806 / 4.72% | 610 / 9.62% |
| Either of these exploration categories | 34.80% | 21.73% |
| Surf | 10,809 / 63.29% | 4,280 / 67.53% |
| Full-HP Recover | 1,064 / 6.23% | 281 / 4.43% |

The corresponding Earthquake/Encore branch has 11.62% → 5.14% visited-below-maximum selections and 0.33% → 0.42% unvisited selections. Thus the measured asymmetry is reduced, but persists. The unvisited-action rule is independent of c, and less visited branches retain proportionally more initial exploration. These are path-dependent adaptive-search counts, not matched states or independent binomial trials. The existing classifier also does not prove that every non-Surf action is a mistake. `summary.json` includes the same comparison over all root replies, and the detailed trace summary preserves per-seed and per-quarter results.

The new paired T11 seeds are 66001–66032:

| Final choice | c=1 | c=0.4 |
|---|---:|---:|
| Earthquake | 0 | 1 |
| Hidden Power Bug | 32 | 7 |
| Switch to Ampharos | 0 | 22 |
| Swords Dance | 0 | 2 |

The earlier 13 seeds give 13 Hidden Powers at c=1 versus 11 switches and two Hidden Powers at c=0.4. Reducing Hidden Power selections therefore does not by itself establish the intended T11 improvement. The earlier independent continuation comparison favored Earthquake over both Hidden Power and switching; it did not establish either coefficient's general playing strength.

T26 uses paired seeds 66001–66016, with the same 30,000-iteration OpenSheet budget:

| Final choice | c=1 | c=0.4 |
|---|---:|---:|
| Thunderbolt | 16 | 5 |
| Fire Punch | 0 | 7 |
| Dynamic Punch | 0 | 4 |

Thunderbolt is the supported action in the earlier T26 reconstruction, mechanic checks and continuation comparison. This is an adverse change in that targeted decision check, not a measured full-game win-rate loss. The coefficient change affects our allocation and both sides' interior selection, so its benefit at the diagnosed opponent node cannot be generalized to all decisions.

The mechanism remains in the algorithm: UCB samples exploration actions and their returns still enter ancestor means. c=0.4 lowers the bonus to 40% at fixed visit counts; it does not mean a 40% exploration probability, eliminate initial exploration, or alter the heavy rollout's epsilon=0.2. The experiment supports partial mitigation of the observed behavior, not elimination of the credit-assignment defect or adoption of c=0.4 in OpenSheet.

## Validation and reproduction

The only code change is adding `--c` to `trace_decision` and recording it in output; the default stays 1. At the default, every archived 30,000-iteration root field for seed 61001 matches exactly, including the joint matrix. For all five traced c=0.4 seeds, observed and ordinary searches have exactly identical visits, means, matrices and node counts, also after 100 subsequent ordinary iterations. The trace aggregator independently reconstructs every final root action's count and mean from the 150,000 traced rewards. `cargo check -p nc2000-bot --example trace_decision` passes with the pre-existing unused-function warning. No production search or profile is modified.

```sh
cargo build --release -p nc2000-bot --example trace_decision
mkdir -p tmp/t11-c04

target/release/examples/trace_decision \
  --position data/report-4296/turn-11.json \
  --opponent-team data/report-4296/opponent-team.json \
  --c 0.4 --seed 61001 --trace-out tmp/t11-c04/trace-61001.jsonl --verify
```

Use `--seed 62001 --seeds 4` for the second trace file. Root-only confirmation uses `--interval 30000`, each coefficient, and `--seed 66001 --seeds 32` (the saved files divide this into two 16-seed shards). T26 substitutes `turn-26.json` and uses 16 seeds. Detailed trace aggregation uses the unchanged `t11-cause/analyze.py`; `compare.py` combines its summary with the saved root files:

```sh
python3 data/report-4296/t11-c04/compare.py
```

Full traces remain reproducible scratch files with hashes in `trace-summary/summary.json`. `build.json` binds the measured executable and inputs, while `summary.json` hashes the retained results. No large full-game benchmark was started.
