# Withdrawn independent-tree experiment

The user stopped this direction before the registered comparisons completed. The 30-tree candidate changed the reported T11 action but did not explain or correct the mechanism that produced the original bad decision. It is withdrawn as a proposed fix. The added OpenAgent/BlindAgent integration and arena option have been reverted to `bde8828`; production defaults were never changed. The original research-only prototype remains reproducible from the battle-4296 report.

`stopped.json` records the stop, saved cohorts and hashes. The original `plan.json`, `supplement.json` and `build.json` are retained as the frozen experimental record. Their planned sample sizes were not reached. No final strength gate is evaluated. Completed shards are a subset selected by completion time, which can depend on battle duration; their scores must not be treated as a completed non-regression test.

## Saved partial results

Candidate wins and losses against the current single-tree agent, from fully saved shards only:

| Comparison | Saved / planned games | Wins | Losses |
|---|---|---|---|
| Open, meta pool | 96 / 256 | 40 | 56 |
| Blind, meta pool | 96 / 256 | 35 | 61 |
| Blind, fixture pool | 64 / 128 | 29 | 35 |

These saved shards contain no ties, turn caps or invalid games. Interrupted shards did not save complete per-pair outcomes and are excluded. The direction was stopped at the user's request; it was not a preregistered statistical stopping decision.

## Candidate and scope

The candidate divided each in-battle decision into 30 independent searches and selected the action with the most summed root visits. Open used 30 × 1,000 simulations against a 30,000-simulation baseline, with c=1. Blind used 30 × 900 against 27,000, with c=0.4. Preview remained one tree. Member seeds came from the persistent agent RNG; the earlier research prototype used XOR-derived seeds, so its results were checked again through the shared implementation.

The primary pool has 32 meta teams. Thirty public preview signatures identify one known candidate, while two teams share a signature. The separate fixture pool has 120 generated teams, all outside that prior, exercising fallback imputation. They are not a sample of competitive human team frequencies. `preview-coverage.json` retains the census and fixture hashes. No baked preview tables were available in this checkout. The ensemble did not change the fallback set model.

## Completed controls and position probes

Four identical-arm CRN controls, eight games each, returned score 0.5 and zero split pairs. Default-agent results against greedy, 16 games per mode, matched the saved pre-change executable in wins, losses, ties, pair scores and total turns. Full-budget random controls were Open single 7/8 wins and all other arms 8/8, with no caps. These controls verify selected implementation and measurement properties, not a root-cause repair.

Before withdrawal, all 14 focused tests and `cargo check --workspace --all-targets` passed. The ensemble-specific tests were removed with the reverted integration; the pre-existing tests remain.

After withdrawal, all five changed code files match `bde8828` exactly, `cargo check --workspace --all-targets` passes, and all 12 remaining focused tests pass. All 20 saved arena shards pass the existing artifact validator individually.

Position probes used seeds 7600101–7600116. Open pinned the reporter's actual sheet; Blind omitted it and used ordinary fallback imputation. Blind is therefore a counterfactual reconstruction of an OpenSheet report.

| Mode | Turn | Single tree | Ensemble |
|---|---|---|---|
| Open | 11 | Hidden Power 16 | Earthquake 16 |
| Open | 26 | Thunderbolt 16 | Thunderbolt 16 |
| Blind | 11 | Switch 8, Hidden Power 5, Swords Dance 3 | Hidden Power 15, Earthquake 1 |
| Blind | 26 | Thunder Wave 12, Dynamic Punch 3, Fire Punch 1 | Thunderbolt 13, Thunder Wave 2, Dynamic Punch 1 |

These are action-selection counts. Repeating a preferred action across seeds establishes a reproducible behavior change; it does not establish why the old policy was wrong or whether the candidate repairs that cause. See the [T11 diagnostics](../report-4296/T11.md) for what has actually been isolated and what remains unresolved.

## Reproduction of the withdrawn code

The measured implementation is preserved in commit `1f9ea54`. Reproduce it in an isolated checkout of that revision, using the configurations and seeds in the original plans. `build.json` binds the measured executable and source hashes. The current source no longer exposes `ensemble:30:...` or `--shared-ensemble`. `tools/aggregate-arena.py` can validate the saved shards, but incomplete cohorts must not be substituted for the registered final sample.
