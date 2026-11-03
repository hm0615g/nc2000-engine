# Blind-boundary repair handoff — 2026-11-03

- Worktree: `/Volumes/WorkData/work/nc2000-engine/tmp/blind-boundary-repair`, branch `fix/blind-boundary`. Original `master` remains at `97a133f6`, unchanged.
- Commits: `646a08be` implements the repair; `872d8ed8` records the T2 experiment below. No merge or deployment.
- Implemented: player-protocol reconstruction for product entry points, private `ObservedPosition` construction, per-candidate HP bucket reconstruction, and removal of heuristic product root exclusions. Includes unequal-roster/relaxed-cap compatibility and a self-KO exclusion counterexample.
- Preview bake: 182 Nash-involving matrices recomputed, 946 reused with dependency checks; 149 affected keys rebuilt and 841 unrelated keys preserved. Recomputed choices were identical. All 990 keys remain active.
- Verification: Web 70 tests, five boundary tests including 81k invariance, native/WASM parity, 2,178 production preview cases and 69,696 legal-slot checks passed. Eight bot-library failures also reproduce at the baseline; evidence is in `data/blind-boundary-repair-v1/`.

## Critical correction

The T2 64-game experiment is **not the requested Nash validation**. It changed search policy, compute, preview response and prior knowledge. Its 42–22 result has no bearing on the fixed-bot party-mixture Nash claim. Withdraw the earlier Nash conclusion. Raw records may remain as an unrelated experiment.

The proposed candidate with 78.125% came from an **older, different Nash mixture** and is also inapplicable. The user states no recorded candidate exceeds 50% against the **current** Nash mixture. Candidate selection has not been completed correctly.

Committed Nash-related conclusions in the repair README, `challenger-summary.json`, `verification.json`, and related UI/documentation changes still need correction; this was discussed but not executed before the user stopped work. The earlier completion report was incorrect for the Nash subtask.

## Outstanding user request

Select exactly one party: the highest historical opponent score **against the current Nash mixture**, even though that score is at most 50%. Directly test it against the current mixture using the repaired bot on both sides, identical budgets, information, prior and production selection rules. No T2/oracle, additional solver, candidate-specific prior injection, party optimization or new candidate sweep.

Current mixture: `data/kodoku-r3/conclusion-equilibrium-deme.json` (four parties, weights 0.30/0.25/0.25/0.20). Read budgets from `data/live-search.json`. Check each historical record's actual target mixture and score orientation; PartyBox `payoff`/`payoff_vs_apex4` scores the Nash side, so lower means a stronger opponent. Do not assume PartyBox's best is the global best.

Session stopped at the user's request. No additional matches or source changes were made after the stop; no jobs are running. This handoff is the only new change.
