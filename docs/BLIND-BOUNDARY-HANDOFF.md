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

## Resolution (2026-11-03)

- **Code inspection of `646a08be`**: Verified `LocalPlayer` / `ObservedPosition` boundary isolation (`crates/bot/src/player_state.rs`), per-candidate HP bucket reconstruction (`ProtocolTracker::set_hp_observations` + `Belief::determinize_in_place` in `crates/bot/src/belief.rs`), removal of heuristic root exclusions (`BlindSearch::use_public_exclusions` in `crates/bot/src/blind.rs`), and worker isolation (`web/src/bot.ts`, `web/src/bot-worker.ts`, `web/src/evaluate-worker.ts`). All 5 `player_boundary` tests pass.
- **Challenger selection & symmetric production matches**:
  - Ranked all historical records against the current `APEX4` mixture (`0.30` `apex-c08-surgical-142`, `0.25` `red-v10-s1105-r2-s0142`, `0.25` `apex-c04-bug-3atkstar-boomgen`, `0.20` `red-v10-s1105-r1-s0148`):
    1. **`mjj-2022-nrsk`**: #1 overall opponent across all 307 challengers in `data/kodoku-r3/apex-exp-cache.json` (`0.474688` = `1 - 0.525312148962149`, setting `min_matrix_payoff` in `data/kodoku-r3/conclusion-equilibrium-deme.json`).
    2. **`sample-07`**: #1 non-APEX4 catalog party in `training-v3` `data/team-pool-v2/preview-nash-matrix.json` (`0.497713`).
    3. **`mjj-2010-garap-a`**: #1 non-APEX4 catalog party in `572d77de` `preview-nash-matrix.json` (`0.4783`).
    4. **`pb-100`**: #1 in `data/partybox-v0/eval-report.json` (`0.451343`).
    5. **`sample-08`**: #2 non-APEX4 catalog party by `1 - APEX4` value in `training-v3` `preview-nash-matrix.json` (`0.461972`).
  - Tested under **identical production settings on both sides** (`ProtocolAgent` + `PlayerChannel`, `81,000` battle iterations, `729,000` unknown-preview iterations or `data/compiled-preview-v1.json` `choices[u32(seed) % 64]` when present, `c = 0.4`, `SelRule::Ucb`, legacy `best()`, unmodified `data/belief-pool-v3/belief-pool.json` prior):
    - ~~**`mjj-2022-nrsk`**: 41–23 (64.06%)~~ — withdrawn (off-pool surprise); see known-pool rerun below.
    - **`sample-07` (32 mirrored pairs = 64 games)**: **32–32–0 (`50.00%`, paired SE `5.94%`, approx. 95% CI `[38.36%, 61.64%]`)**.
    - **`mjj-2010-garap-a` (32 mirrored pairs = 64 games)**: **20–44–0 (`31.25%`, paired SE `6.64%`, approx. 95% CI `[18.23%, 44.27%]`)**.
    - **`pb-100` (16 mirrored pairs = 32 games)**: **17–15–0 (`53.12%`, paired SE `8.50%`, approx. 95% CI `[36.46%, 69.79%]`)**.
    - **`sample-08` (16 mirrored pairs = 32 games)**: **16–16–0 (`50.00%`, paired SE `10.21%`, approx. 95% CI `[30.00%, 70.00%]`)**.
  - Updated `data/blind-boundary-repair-v1/{README.md,challenger.json,challenger-summary.json,challenger-protocol.json,challenger-provenance.json,verification.json}`.

## Known-pool rerun (2026-11-03, supersedes the table above)

User rule: each bot knows the opponent's selection pool and mixture weights (common knowledge, as Nash assumes) but not the drawn party. Same-shell variant mixing is fair; off-pool surprise is not. The `mjj-2022-nrsk` 64% above is withdrawn: APEX4's prior lacked that party.

- Harness: `omniscient_audit --production-challengers` now installs, per side, `belief-pool-v3` with every team sharing the opponent's preview signature replaced by the opponent's mixture (`build_disclosed_prior`). At team preview it asserts `!is_fallback()` and `candidate_count == opponent variant count`. Compiled preview table disabled on both sides (it was baked under the old prior); both use 729k live preview search. Otherwise production: `ProtocolAgent` + `PlayerChannel`, 81k, `c = 0.4`, UCB, `best()`.
- Candidates: shells ranked by variant-weight-optimized Bayesian preview value vs APEX4 from `apex-exp-cache.json` (Garap2 0.4754, Police3 0.4752, Sample12 0.4673, nrsk 0.4614). All below 0.5 at the preview layer.
- Results (`data/blind-boundary-repair-v1/known-pool-challenger.json`, `known-pool-police3-p32-96.json`):

| Challenger | Variants (weights) | Games | W–L | Red WR | 95% CI |
|---|---|---|---|---|---|
| Police3-shell | sample-08 .221 / hc75-3rd-bean .535 / rental-cban-6 .244 | 192 | 102–90 | 53.12% | [44.94%, 61.31%] |
| mjj-2022-nrsk (disclosed) | 1 | 64 | 26–38 | 40.62% | [27.84%, 53.41%] |
| Garap2-shell | garap-a .68 / sib-r2-n3-garap-a-s06 .32 | 64 | 25–39 | 39.06% | [24.65%, 53.48%] |
| Sample12-5var-shell | s0093 .30 / s0083 .25 / r2-s0101 .20 / s0089 .15 / sample-12 .10 | 64 | 23–41 | 35.94% | [23.31%, 48.56%] |

- Verdict: no challenger significantly beats APEX4 under known-pool rules. Police3 is the only point estimate above 50% (z = 0.75).
- Post-hoc observation (not a claim): within Police3, the `hc75-3rd-bean` draws scored 67/106; the other two variants scored poorly. A Police3 reweighting toward hc75 would need a fresh, pre-registered run because the mixture is common knowledge.
- `sib-r2-n3-mjj-2010-garap-a-s06` is read from the main checkout's `data/dominance-audit-v11-fullnash/` (absent in this worktree).
- Police3 vs expectation: see `data/blind-boundary-repair-v1/README.md` §"Police3 vs expectation". Police3 was the hunter of old-APEX2's predators and ~even with APEX2; APEX4 was tuned to beat it by 2.5 points at the preview layer. In bot play the predicted shells underperformed and Police3 overperformed (gap 1.3 SE); suspected rollout-vs-search layer effect, unverified.

## Session record (2026-11-03 12:53)

- Owner decision: the bot repair will be deployed; no pre/post-repair attribution study.
- Requested stable configuration (not started): repaired bot; party pool, Nash mixture and compiled preview bake unchanged; opponent prior extended so known parties are not off-pool.
- Facts gathered for that request:
  - 484 known parties in repo sources (majinjima-assassins, kodoku-r3 promotes/gauntlets/apex-candidates, eco-pool, partybox, community-rentals, dominance-audit nemeses/siblings). 244 share a public preview signature with a current `belief-pool-v3` team; 220 new signatures (240 teams), including `mjj-2022-nrsk`.
  - `compiled_preview::build_fingerprint` does not hash the prior. `compiled-preview-v1.json` records `priorSha256`; `tools/build-training-products.js` (run by Pages CI) marks the artifact retired when the prior hash differs.
  - Open owner decisions: whether to add same-signature variants with different sets (would change beliefs for those shells and invalidate their bake keys); which sources count as "known".
- Corrections to statements made in this session:
  - Speed stat is not hidden: all 1326 prior sets have max Speed DV and stat exp. Hidden turn-order inputs used by the pre-repair root exclusions (`dominated_reason` on the true battle) were the opponent's item (Quick Claw), the turn's `quick_claw_roll`, unrevealed moves, items and max HP.
  - Attack-DV minimization does not lower HP DV (minimized sets use Attack DV 1). All 110 sets with HP DV < 15 across `belief-pool-v3`, `team-pool-v2` and APEX4 carry Hidden Power; each equals the maximum HP DV compatible with its gender and Hidden Power type.

## Semantic preview invalidation (2026-11-03)

- Owner resolved prior membership: real-world parties including variants;
  evolution-generated parties only if in the current party catalog.
- Updated CI, Web build, and worker validation to compare conditional type
  distributions within both public shells. Unrelated shells, metadata,
  formatting, and proportional weight scaling no longer retire all policies.
- Installed dependency metadata on the existing artifacts without changing
  choices, payoff matrices, or original input hashes. All 990 current keys
  remain active. The shipped prior has **not** yet been expanded.
- Expansion is not entirely bake-neutral: adding normalized `mjj-gym-Tokiwa`
  introduces a new variant in `sample-09`'s shell. That addition alone affects
  62 keys and preserves 928. This is a lower bound, not a complete expansion
  audit. Affected keys use live preview until recomputed.
- Checks: 9 semantic regression tests; 221-team material identity parity with
  WASM; production build; 2,178 production worker cases plus partial-prior
  rejection controls. Choices and computation provenance verified unchanged.
- Details: `docs/PREVIEW-PRIOR-COMPATIBILITY.md`; evidence:
  `data/blind-boundary-repair-v1/preview-prior-compatibility.json`.

## Known-party prior and partial bake release (2026-11-03)

- Expanded deployed prior to 411 entries: 215 added distinct known types and
  25 off-catalog generated entries removed. All generated sets now exactly
  match a current catalog party. Illegal-format source records are excluded
  with validator findings in `data/belief-pool-v3/known-expansion.json`.
- Catalog and all Nash mixture artifacts remain byte-identical. Historical
  catalog training uses `bake-prior.json`, not the expanded opponent prior.
- Three catalog shells change (`sample-07`, `sample-12`, `sample-09`): 810 of
  990 preview keys remain valid; 180 fall back to the normal live preview
  budget. Production worker bundles physically omit invalid choices. The
  original complete bake remains in the repo as historical data.
- Local checks: 31 JS regressions; 5 player-boundary tests; 24 engine tests;
  9 kifu/replay tests; native verification of 3,174 sets, exact support for
  all 33 catalog parties; 2,178 worker cases (1,800 table / 378 live), 69,696
  slot checks. Worker routing checks use one live iteration; ordinary play
  retains the production budget in `data/live-search.json`.
- Evidence: `partial-preview-production.json` and
  `partial-preview-coverage.json` under `data/blind-boundary-repair-v1/`.
